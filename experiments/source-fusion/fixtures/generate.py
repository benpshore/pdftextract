#!/usr/bin/env python3
"""Generate bounded PDF primitives from an independently frozen private plan.

The plan and emitted truth directory must stay outside selector inputs until
the source policy freezes. This script never reads extractor output.
"""

import argparse
import hashlib
import json
import zlib
from pathlib import Path

ROOT = Path(__file__).resolve().parent


def sha(data):
    return hashlib.sha256(data).hexdigest()


def stream(data, extra=""):
    return f"<< /Length {len(data)} {extra} >>\nstream\n".encode() + data + b"\nendstream"


def cmap(body, kind, ordering, name):
    return (
        "/CIDInit /ProcSet findresource begin\n12 dict begin\nbegincmap\n"
        f"/CIDSystemInfo << /Registry (Adobe) /Ordering ({ordering}) /Supplement 0 >> def\n"
        f"/CMapName /{name} def\n/CMapType {kind} def\n/WMode 0 def\n"
        "1 begincodespacerange\n<0000> <FFFF>\nendcodespacerange\n"
        f"{body}\nendcmap\nCMapName currentdict /CMap defineresource pop\nend end\n"
    ).encode("ascii")


def generate(plan, truth_dir):
    font_bytes = (ROOT / "font/DejaVuSans-subset.ttf").read_bytes()
    font = json.loads((ROOT / "font/font-source.json").read_text())
    if sha(font_bytes) != font["subset_font_sha256"]:
        raise RuntimeError("Font subset hash mismatch")
    glyphs = font["glyphs"]
    ordered = sorted(glyphs.items(), key=lambda item: item[1]["gid"])
    scale = 1000 / font["units_per_em"]
    widths = " ".join(f"{v['gid']} [{v['width'] * scale:g}]" for _, v in ordered)
    font_box = " ".join(f"{v * scale:g}" for v in font["bbox"])
    good_maps = [(v["gid"], ord(ch)) for ch, v in ordered]
    frame = {
        "page": 1,
        "page_index": 0,
        "width": 612,
        "height": 792,
        "media_box": [0, 0, 612, 792],
        "crop_box": [0, 0, 612, 792],
        "rotation": 0,
        "origin": "bottom-left",
        "units": "pt",
        "axes": "x-right-y-up",
    }

    def unicode_stream(pairs, name):
        entries = "\n".join(f"<{gid:04X}> <{value:04X}>" for gid, value in pairs)
        return stream(cmap(f"{len(pairs)} beginbfchar\n{entries}\nendbfchar", 2, "UCS", name))

    def show(run, resource):
        encoded = "".join(f"{glyphs[ch]['gid']:04X}" for ch in run["text"])
        return (
            f"BT /{resource} {run['size']} Tf 1 0 0 1 {run['x']} {run['y']} Tm <{encoded}> Tj ET\n"
        ).encode()

    outputs = []
    for case in plan["cases"]:
        pairs = list(good_maps)
        if case["map"] == "misleading":
            change = glyphs[case["misleading_character"]]["gid"]
            pairs = [
                (gid, ord(case["misleading_replacement"]) if gid == change else value)
                for gid, value in pairs
            ]
        elif case["map"] == "ambiguous":
            pairs.append(
                (glyphs[case["ambiguous_character"]]["gid"], ord(case["ambiguous_replacement"]))
            )
        to_unicode = "" if case["map"] == "missing" else "/ToUnicode 9 0 R"
        encoding = "/Identity-H" if case["encoding"] == "named" else "8 0 R"
        objects = [
            b"<< /Type /Catalog /Pages 2 0 R >>",
            b"<< /Type /Pages /Kids [3 0 R] /Count 1 >>",
            (
                b"<< /Type /Page /Parent 2 0 R /MediaBox [0 0 612 792] "
                b"/CropBox [0 0 612 792] /Rotate 0 /Resources "
                b"<< /Font << /Good 5 0 R /Recovery 6 0 R >> >> /Contents 4 0 R >>"
            ),
            stream(show(plan["neighbor"], "Good") + show(case, "Recovery")),
            (
                b"<< /Type /Font /Subtype /Type0 /BaseFont /FixtureDejaVuSans "
                b"/Encoding /Identity-H /DescendantFonts [7 0 R] /ToUnicode 12 0 R >>"
            ),
            (
                f"<< /Type /Font /Subtype /Type0 /BaseFont /FixtureDejaVuSans "
                f"/Encoding {encoding} /DescendantFonts [7 0 R] {to_unicode} >>"
            ).encode(),
            (
                "<< /Type /Font /Subtype /CIDFontType2 /BaseFont /FixtureDejaVuSans "
                "/CIDSystemInfo << /Registry (Adobe) /Ordering (Identity) /Supplement 0 >> "
                f"/FontDescriptor 10 0 R /CIDToGIDMap /Identity /DW 1000 /W [{widths}] >>"
            ).encode(),
            stream(
                cmap(
                    "1 begincidrange\n<0000> <FFFF> 0\nendcidrange",
                    1,
                    "Identity",
                    "FixtureIdentity",
                )
            ),
            unicode_stream(pairs, "TargetUnicode"),
            (
                "<< /Type /FontDescriptor /FontName /FixtureDejaVuSans /Flags 32 "
                f"/FontBBox [{font_box}] /Ascent {font['ascent'] * scale:g} "
                f"/Descent {font['descent'] * scale:g} /CapHeight 750 /ItalicAngle 0 "
                "/StemV 80 /FontFile2 11 0 R >>"
            ).encode(),
            stream(
                zlib.compress(font_bytes, 9), f"/Filter /FlateDecode /Length1 {len(font_bytes)}"
            ),
            unicode_stream(good_maps, "NeighborUnicode"),
        ]
        data = bytearray(b"%PDF-1.7\n%\xe2\xe3\xcf\xd3\n")
        offsets = [0]
        for number, obj in enumerate(objects, 1):
            offsets.append(len(data))
            data += f"{number} 0 obj\n".encode() + obj + b"\nendobj\n"
        xref = len(data)
        data += f"xref\n0 {len(offsets)}\n0000000000 65535 f \n".encode()
        for offset in offsets[1:]:
            data += f"{offset:010d} 00000 n \n".encode()
        data += (
            f"trailer\n<< /Size {len(offsets)} /Root 1 0 R >>\nstartxref\n{xref}\n%%EOF\n"
        ).encode()
        path = ROOT / f"{case['id']}.pdf"
        path.write_bytes(data)
        identity = {"case_id": case["id"], "sha256": sha(data), "size": len(data)}
        semantic = None if case["map"] in {"missing", "ambiguous"} else case["text"]
        if case["map"] == "misleading":
            semantic = case["text"].replace(
                case["misleading_character"], case["misleading_replacement"]
            )
        truth = {
            "schema": "source-fusion-private-truth-v1",
            "source": identity,
            "frame": frame,
            "evaluation_class": case["evaluation_class"],
            "neighbor": plan["neighbor"],
            "target": case,
            "visible_target_truth": case["text"],
            "mapped_semantic_truth": semantic,
            "truth_basis": "Frozen generator text and embedded glyph IDs before extraction.",
            "selection_expectation": (
                "No answer supplied to selector. Evaluate visible and semantic axes separately."
            ),
        }
        (truth_dir / f"{case['id']}.truth.json").write_text(
            json.dumps(truth, indent=2, sort_keys=True) + "\n"
        )
        outputs.append(identity)
    return outputs


if __name__ == "__main__":
    parser = argparse.ArgumentParser()
    parser.add_argument("--plan", type=Path, required=True)
    parser.add_argument("--truth-dir", type=Path, required=True)
    args = parser.parse_args()
    args.truth_dir.mkdir(parents=True, exist_ok=True)
    identities = generate(json.loads(args.plan.read_text()), args.truth_dir)
    print(json.dumps(identities, indent=2))
