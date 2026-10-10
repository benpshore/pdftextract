"""Prototype `pdftotext -layout` over the lopdf engine's published JSON.

Nothing here reads the PDF. It uses only what `tpe extract --out` already
emits per page: spans (text, box, size; one per shown string) and lines.

Ported from Poppler TextOutputDev.cc (GitHub mirror tsdgeos/poppler_mirror at
5b97e47, version 26.09.90):
  * rows: fragments whose baselines differ by < maxIntraLineDelta (0.5) x size
  * column assignment: TextBlock::coalesce (lines in a block) and
    TextPage::coalesce (blocks), or TextPage::assignColumns when there is no
    block structure. Character edges are linearly interpolated inside a
    fragment because the engine publishes no per-character positions.
  * dump: pad to the fragment's column, 1..5 newlines by baseline distance
The word-space threshold is the engine's own 0.15 x size (Poppler's is per
character). Both column assignments are quadratic in fragments, as in Poppler.

Variants
  lines      engine line text exactly as published (after its text cleanup)
  spans      spans only; no engine line or block structure
  linespans  engine line membership and block ids (`column`), text rebuilt
             from the member spans

Usage: layout_proto.py CORPUS_JSON PDF_DIR TPE_JSON_DIR [RENDER_DIR] > layout.json
"""

import json
import re
import sys
from collections import Counter
from pathlib import Path

from common import engine_doc, lcs_len, load_corpus, norm, run, split_pages

MAX_INTRA_LINE_DELTA = 0.5  # Poppler maxIntraLineDelta
MAX_WORD_SPACING = 1.5  # Poppler maxWordSpacing
SPACE_GAP = 0.15  # engine reading_order::SPACE_GAP
DESCENT = -0.2  # engine lopdf_backend::DESCENT


def rotated(box: dict, size: float) -> bool:
    width, height = box["x1"] - box["x0"], box["y1"] - box["y0"]
    return height > 1.6 * size and height > 2.5 * width


def join_spans(spans: list[dict]) -> str:
    text, right, size = "", None, 0.0
    for span in sorted(spans, key=lambda s: s["bbox"]["x0"]):
        box, span_size = span["bbox"], span.get("size") or 0
        if right is not None:
            gap = box["x0"] - right
            wide = gap > SPACE_GAP * max(size, span_size)
            if wide and not text.endswith(" ") and not span["text"].startswith(" "):
                text += " "
        text += span["text"]
        right = box["x1"] if right is None else max(right, box["x1"])
        size = max(size, span_size)
    return " ".join(text.split())


def frags_from_lines(page: dict) -> list[dict]:
    out = []
    for line in page["lines"]:
        box = line.get("bbox")
        if not box or not line["text"].strip():
            continue
        sizes = sorted(page["spans"][i].get("size") or 0 for i in line["spans"])
        size = (sizes[len(sizes) // 2] if sizes else 0) or (box["y1"] - box["y0"]) or 10.0
        out.append(
            {
                "x0": box["x0"],
                "x1": box["x1"],
                "base": box["y0"] - DESCENT * size,
                "size": size,
                "text": line["text"],
            }
        )
    return out


def frags_from_spans(page: dict) -> list[dict]:
    spans = []
    for span in page["spans"]:
        box, size = span.get("bbox"), span.get("size") or 0
        if not box or not span["text"] or size <= 0 or rotated(box, size):
            continue
        spans.append(
            {
                "x0": box["x0"],
                "x1": box["x1"],
                "base": box["y0"] - DESCENT * size,
                "size": size,
                "text": span["text"],
            }
        )
    spans.sort(key=lambda s: (-s["base"], s["x0"]))
    rows: list[list[dict]] = []
    for span in spans:
        first = rows[-1][0] if rows else None
        if first and abs(first["base"] - span["base"]) < MAX_INTRA_LINE_DELTA * first["size"]:
            rows[-1].append(span)
        else:
            rows.append([span])
    frags = []
    for row in rows:
        row.sort(key=lambda s: s["x0"])
        current = None
        for span in row:
            if current is None:
                current = dict(span)
                continue
            gap = span["x0"] - current["x1"]
            size = max(current["size"], span["size"])
            if gap > MAX_WORD_SPACING * size:
                frags.append(current)
                current = dict(span)
                continue
            spaced = current["text"].endswith(" ") or span["text"].startswith(" ")
            current["text"] += (" " if gap > SPACE_GAP * size and not spaced else "") + span["text"]
            current["x1"] = max(current["x1"], span["x1"])
        if current is not None:
            frags.append(current)
    for frag in frags:
        frag["text"] = " ".join(frag["text"].split())
    return [frag for frag in frags if frag["text"]]


def frags_from_line_spans(page: dict) -> list[dict]:
    out = []
    for line in page["lines"]:
        members = [page["spans"][i] for i in line["spans"] if page["spans"][i].get("bbox")]
        box = line.get("bbox")
        if not box or not members:
            continue
        text = join_spans(members)
        if not text:
            continue
        sizes = sorted(span.get("size") or 0 for span in members)
        size = sizes[len(sizes) // 2] or 10.0
        if rotated(box, size):
            continue
        bases = sorted(s["bbox"]["y0"] - DESCENT * (s.get("size") or size) for s in members)
        out.append(
            {
                "x0": box["x0"],
                "x1": box["x1"],
                "base": bases[len(bases) // 2],
                "size": size,
                "text": text,
                "block": line["column"],
            }
        )
    return out


def column_after(earlier: dict, x0: float) -> int:
    """Column at which text starting at `x0` clears the already placed `earlier`."""
    count = len(earlier["text"])
    if x0 >= earlier["x1"]:
        return earlier["col"] + count + 1
    width = earlier["x1"] - earlier["x0"]
    k = 0 if width <= 0 else int((x0 - earlier["x0"]) / width * count + 0.5)
    return earlier["col"] + max(0, min(count, k))


def assign_columns(frags: list[dict]) -> None:
    """Poppler TextPage::assignColumns, rotation 0."""
    order = sorted(frags, key=lambda f: (f["x0"], -f["base"]))
    for index, frag in enumerate(order):
        frag["col"] = max((column_after(e, frag["x0"]) for e in order[:index]), default=0)


def assign_columns_blocks(frags: list[dict]) -> None:
    """Poppler's two-level assignment; blocks are the engine's `column` ids."""
    blocks: dict = {}
    for frag in frags:
        blocks.setdefault(frag["block"], []).append(frag)
    info = []
    for lines in blocks.values():
        lines.sort(key=lambda f: (f["x0"], -f["base"]))
        for index, line in enumerate(lines):
            line["col"] = max((column_after(e, line["x0"]) for e in lines[:index]), default=0)
        info.append(
            {
                "x0": min(f["x0"] for f in lines),
                "x1": max(f["x1"] for f in lines),
                "top": max(f["base"] for f in lines),
                "ncols": max(f["col"] + len(f["text"]) for f in lines),
                "lines": lines,
            }
        )
    info.sort(key=lambda b: (b["x0"], -b["top"]))
    for index, block in enumerate(info):
        col = 0
        for earlier in info[:index]:
            if block["x0"] > earlier["x1"]:
                cand = earlier["col"] + earlier["ncols"] + 3
            elif earlier["x1"] == earlier["x0"]:
                cand = earlier["col"]
            else:
                share = (block["x0"] - earlier["x0"]) / (earlier["x1"] - earlier["x0"])
                cand = earlier["col"] + int(share * earlier["ncols"])
            col = max(col, cand)
        block["col"] = col
        for line in block["lines"]:
            line["col"] += col


def render(frags: list[dict]) -> str:
    if not frags:
        return ""
    if "block" in frags[0]:
        assign_columns_blocks(frags)
    else:
        assign_columns(frags)
    rows: list[list[dict]] = []
    for frag in sorted(frags, key=lambda f: (-f["base"], f["x0"])):
        first = rows[-1][0] if rows else None
        if first and abs(first["base"] - frag["base"]) < MAX_INTRA_LINE_DELTA * first["size"]:
            rows[-1].append(frag)
        else:
            rows.append([frag])
    out: list[str] = []
    previous = None
    for row in rows:
        row.sort(key=lambda f: (f["col"], f["x0"]))
        if previous is not None:
            drop = int((previous["base"] - row[0]["base"]) / previous["size"])
            out.extend([""] * (max(1, min(drop, 5)) - 1))
        text, col = "", 0
        for frag in row:
            if frag["col"] < col:  # Poppler starts a new output line here
                out.append(text)
                text, col = "", 0
            text += " " * (frag["col"] - col) + frag["text"]
            col = frag["col"] + len(frag["text"])
        out.append(text)
        previous = row[0]
    return "\n".join(out)


def rows_of(text: str) -> list[tuple]:
    return [tuple(norm(row).split()) for row in text.split("\n") if row.strip()]


def cells_of(text: str) -> list[tuple]:
    rows = [norm(row).strip() for row in text.split("\n") if row.strip()]
    return [tuple(" ".join(cell.split()) for cell in re.split(r" {2,}", row)) for row in rows]


def score(proto: str, poppler: str) -> dict:
    poppler_rows, proto_rows = rows_of(poppler), rows_of(proto)
    poppler_words = [word for row in poppler_rows for word in row]
    proto_words = [word for row in proto_rows for word in row]
    poppler_chars, proto_chars = "".join(poppler_words), "".join(proto_words)
    return {
        "rows_poppler": len(poppler_rows),
        "rows_proto": len(proto_rows),
        "rows_exact": sum((Counter(poppler_rows) & Counter(proto_rows)).values()),
        "rows_cells_exact": sum((Counter(cells_of(poppler)) & Counter(cells_of(proto))).values()),
        "chars_poppler": len(poppler_chars),
        "chars_proto": len(proto_chars),
        "chars_lcs": lcs_len(poppler_chars, proto_chars),
        "words_poppler": len(poppler_words),
        "words_proto": len(proto_words),
        "words_lcs": lcs_len(poppler_words, proto_words),
    }


VARIANTS = {
    "lines": frags_from_lines,
    "spans": frags_from_spans,
    "linespans": frags_from_line_spans,
}


def main() -> None:
    corpus, pdf_dir, json_dir = Path(sys.argv[1]), Path(sys.argv[2]), Path(sys.argv[3])
    render_dir = Path(sys.argv[4]) if len(sys.argv) > 4 else None
    report = {}
    for entry in load_corpus(corpus):
        name = entry["id"]
        doc = engine_doc(json_dir, entry["sha256"])
        poppler = split_pages(run(["pdftotext", "-layout", str(pdf_dir / f"{name}.pdf"), "-"]))
        if len(poppler) != len(doc["pages"]):
            raise SystemExit(f"{name}: page count differs between the engine and Poppler")
        report[name] = {"pages": len(poppler), "engine_status": doc.get("status")}
        for variant, build in VARIANTS.items():
            totals: Counter = Counter()
            texts = []
            for page, poppler_page in zip(doc["pages"], poppler, strict=True):
                text = render(build(page))
                texts.append(text)
                totals.update(score(text, poppler_page))
            if render_dir is not None:
                target = render_dir / f"{name}.proto-{variant}.txt"
                target.write_text("\f".join(texts), encoding="utf-8")
            report[name][variant] = dict(totals)
    json.dump(report, sys.stdout, indent=1, sort_keys=True)
    print()


if __name__ == "__main__":
    main()
