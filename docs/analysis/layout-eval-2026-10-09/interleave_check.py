"""Count pages where a tool reads side-by-side columns line by line.

Works on the ordered line list each tool publishes, so it needs no ground
truth and no image:
  engine   `lines` (box and text, in the engine's reading order) from the JSON
  Poppler  `pdftotext -bbox-layout` (flow > block > line, in reading order)

Two signals per page, for each tool:
  zip    consecutive lines i, i+1 on one baseline (within 3 pt) with disjoint
         x-ranges, each at least 25 non-space characters. In a correct column
         order the left and right line of one row are far apart.
  fused  one line whose box covers two of the OTHER tool's long lines that sit
         side by side on that baseline (two columns printed as one line).
Both signals also require column-sized geometry, so that one column line the
other tool happened to split in two is not counted: each of the two lines is
at least 20% of the page width and together they span at least 50% of it.
A page is flagged when zip + fused >= 5.

This detects one failure only. It does not see a table or figure read in the
wrong place, or any error inside a single column.

Usage: interleave_check.py CORPUS_JSON PDF_DIR TPE_JSON_DIR > interleave.json
"""

import json
import re
import sys
from itertools import pairwise
from pathlib import Path

from common import engine_doc, load_corpus, run

MIN_CHARS = 25
MIN_PART = 0.20  # each side-by-side line, as a share of the page width
MIN_SPAN = 0.50  # the pair together, as a share of the page width
BASE_TOL = 3.0
FLAG_AT = 5

NUM = r'"([\d.-]+)"'
LINE_RE = re.compile(rf"<line xMin={NUM} yMin={NUM} xMax={NUM} yMax={NUM}>(.*?)</line>", re.S)
WORD_RE = re.compile(r"<word [^>]*>(.*?)</word>", re.S)
PAGE_RE = re.compile(r'<page width="([\d.]+)" height="([\d.]+)">(.*?)</page>', re.S)


def poppler_pages(pdf: Path) -> list[dict]:
    xml = run(["pdftotext", "-bbox-layout", str(pdf), "-"])
    pages = []
    for width, height, body in PAGE_RE.findall(xml):
        lines = []
        for x0, _y0, x1, y1, inner in LINE_RE.findall(body):
            count = sum(len(word) for word in WORD_RE.findall(inner))
            lines.append(
                {"x0": float(x0), "x1": float(x1), "base": float(height) - float(y1), "n": count}
            )
        pages.append({"width": float(width), "lines": lines})
    return pages


def engine_pages(doc: dict) -> list[dict]:
    pages = []
    for page in doc["pages"]:
        lines = []
        for line in page["lines"]:
            box = line.get("bbox")
            if not box:
                continue
            count = len("".join(line["text"].split()))
            lines.append({"x0": box["x0"], "x1": box["x1"], "base": box["y0"], "n": count})
        pages.append({"width": page["width"], "lines": lines})
    return pages


def column_sized(a: dict, b: dict, width: float) -> bool:
    if a["x1"] - a["x0"] < MIN_PART * width or b["x1"] - b["x0"] < MIN_PART * width:
        return False
    return max(a["x1"], b["x1"]) - min(a["x0"], b["x0"]) >= MIN_SPAN * width


def side_by_side(a: dict, b: dict, width: float) -> bool:
    disjoint = b["x0"] >= a["x1"] - 2 or a["x0"] >= b["x1"] - 2
    return disjoint and column_sized(a, b, width)


def zips(page: dict) -> int:
    lines, count = page["lines"], 0
    for a, b in pairwise(lines):
        if a["n"] < MIN_CHARS or b["n"] < MIN_CHARS or abs(a["base"] - b["base"]) > BASE_TOL:
            continue
        if side_by_side(a, b, page["width"]):
            count += 1
    return count


def fused(mine: dict, other: dict) -> int:
    count = 0
    long_other = [line for line in other["lines"] if line["n"] >= MIN_CHARS]
    for line in mine["lines"]:
        inside = [
            o
            for o in long_other
            if abs(o["base"] - line["base"]) <= BASE_TOL + 2
            and o["x0"] >= line["x0"] - 4
            and o["x1"] <= line["x1"] + 4
        ]
        inside.sort(key=lambda o: o["x0"])
        if any(
            b["x0"] >= a["x1"] - 2 and column_sized(a, b, mine["width"])
            for a, b in pairwise(inside)
        ):
            count += 1
    return count


def main() -> None:
    corpus, pdf_dir, json_dir = Path(sys.argv[1]), Path(sys.argv[2]), Path(sys.argv[3])
    report = {}
    for entry in load_corpus(corpus):
        name = entry["id"]
        doc = engine_doc(json_dir, entry["sha256"])
        engine, poppler = engine_pages(doc), poppler_pages(pdf_dir / f"{name}.pdf")
        if len(engine) != len(poppler):
            raise SystemExit(f"{name}: page count differs between the engine and Poppler")
        rows = []
        for number, (eng, pop) in enumerate(zip(engine, poppler, strict=True), 1):
            rows.append(
                {
                    "page": number,
                    "engine_zip": zips(eng),
                    "engine_fused": fused(eng, pop),
                    "poppler_zip": zips(pop),
                    "poppler_fused": fused(pop, eng),
                }
            )
        report[name] = {
            "pages": len(rows),
            "engine_status": doc.get("status"),
            "engine_flagged": [
                r["page"] for r in rows if r["engine_zip"] + r["engine_fused"] >= FLAG_AT
            ],
            "poppler_flagged": [
                r["page"] for r in rows if r["poppler_zip"] + r["poppler_fused"] >= FLAG_AT
            ],
            "per_page": rows,
        }
    json.dump(report, sys.stdout, indent=1, sort_keys=True)
    print()


if __name__ == "__main__":
    main()
