#!/usr/bin/env python3
"""Deterministic, synthetic PDF primitives and predeclared fixture truth.

Uses only Python's standard library and the checked-in licensed font subset.
It never obtains truth from an extractor. Run from any directory.
"""

import hashlib
import json
import zlib
from pathlib import Path

ROOT = Path(__file__).resolve().parent
GOOD = "BASELINE REGION RETAINS THIS TEXT."
TARGET = "RECOVER ALPHA 2026"
FRAME = {
    "units": "pt",
    "origin": "bottom-left",
    "axes": "x-right-y-up",
    "page": 1,
    "width": 612,
    "height": 792,
    "media_box": [0, 0, 612, 792],
    "crop_box": [0, 0, 612, 792],
    "rotation_degrees": 0,
}


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


def write_pdf(name, custom_encoding, empty_unicode=False, repeated=False):
    font_bytes = (ROOT / "font/DejaVuSans-subset.ttf").read_bytes()
    font = json.loads((ROOT / "font/font-source.json").read_text())
    if hashlib.sha256(font_bytes).hexdigest() != font["subset_font_sha256"]:
        raise RuntimeError("Font subset differs from its recorded source hash")
    glyphs = font["glyphs"]
    scale = 1000 / font["units_per_em"]
    widths = " ".join(
        f"{v['gid']} [{round(v['width'] * scale, 6):g}]"
        for _, v in sorted(glyphs.items(), key=lambda item: item[1]["gid"])
    )
    encoded_target = "".join(f"{glyphs[ch]['gid']:04X}" for ch in TARGET)
    good = f"BT /Good 14 Tf 1 0 0 1 50 722 Tm ({GOOD}) Tj ET\n"
    recovery = f"BT /Recovery 22 Tf 1 0 0 1 50 622 Tm <{encoded_target}> Tj ET\n"
    if repeated:
        recovery += f"BT /Recovery 22 Tf 1 0 0 1 50 522 Tm <{encoded_target}> Tj ET\n"
    maps = "\n".join(
        f"<{v['gid']:04X}> <{ord(ch):04X}>"
        for ch, v in sorted(glyphs.items(), key=lambda item: item[1]["gid"])
    )
    unicode_body = "" if empty_unicode else f"{len(glyphs)} beginbfchar\n{maps}\nendbfchar"
    font_box = " ".join(f"{v * scale:g}" for v in font["bbox"])
    objects = [
        b"<< /Type /Catalog /Pages 2 0 R >>",
        b"<< /Type /Pages /Kids [3 0 R] /Count 1 >>",
        (
            b"<< /Type /Page /Parent 2 0 R /MediaBox [0 0 612 792] "
            b"/CropBox [0 0 612 792] /Rotate 0 /Resources "
            b"<< /Font << /Good 5 0 R /Recovery 6 0 R >> >> /Contents 4 0 R >>"
        ),
        stream((good + recovery).encode()),
        b"<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica /Encoding /WinAnsiEncoding >>",
        (
            "<< /Type /Font /Subtype /Type0 /BaseFont /FixtureDejaVuSans "
            f"/Encoding {'8 0 R' if custom_encoding else '/Identity-H'} "
            "/DescendantFonts [7 0 R] /ToUnicode 9 0 R >>"
        ).encode(),
        (
            "<< /Type /Font /Subtype /CIDFontType2 /BaseFont /FixtureDejaVuSans "
            "/CIDSystemInfo << /Registry (Adobe) /Ordering (Identity) /Supplement 0 >> "
            f"/FontDescriptor 10 0 R /CIDToGIDMap /Identity /DW 1000 /W [{widths}] >>"
        ).encode(),
        stream(
            cmap("1 begincidrange\n<0000> <FFFF> 0\nendcidrange", 1, "Identity", "FixtureIdentity")
        ),
        stream(cmap(unicode_body, 2, "UCS", "FixtureUnicode")),
        (
            "<< /Type /FontDescriptor /FontName /FixtureDejaVuSans /Flags 32 "
            f"/FontBBox [{font_box}] /Ascent {font['ascent'] * scale:g} "
            f"/Descent {font['descent'] * scale:g} /CapHeight 750 /ItalicAngle 0 "
            "/StemV 80 /FontFile2 11 0 R >>"
        ).encode(),
        stream(zlib.compress(font_bytes, 9), f"/Filter /FlateDecode /Length1 {len(font_bytes)}"),
    ]
    data = bytearray(b"%PDF-1.7\n%\xe2\xe3\xcf\xd3\n")
    offsets = [0]
    for index, obj in enumerate(objects, 1):
        offsets.append(len(data))
        data += f"{index} 0 obj\n".encode() + obj + b"\nendobj\n"
    xref = len(data)
    data += f"xref\n0 {len(offsets)}\n0000000000 65535 f \n".encode()
    for offset in offsets[1:]:
        data += f"{offset:010d} 00000 n \n".encode()
    data += f"trailer\n<< /Size {len(offsets)} /Root 1 0 R >>\nstartxref\n{xref}\n%%EOF\n".encode()
    pdf = ROOT / f"{name}.pdf"
    pdf.write_bytes(data)
    regions = [
        {
            "id": "good-neighbor",
            "expected_text": GOOD,
            "source_font": "Good",
            "font_size_pt": 14,
            "baseline_xy": [50, 722],
            "region_bbox": [40, 708, 330, 742],
            "source_operation": "content object 4, first text show",
            "expected_action": "retain baseline exactly",
        },
        {
            "id": "recovery-target",
            "expected_text": TARGET,
            "source_font": "Recovery",
            "font_size_pt": 22,
            "baseline_xy": [50, 622],
            "region_bbox": [40, 608, 315, 650],
            "source_operation": "content object 4, second text show",
            "expected_action": "adjudicate candidate against source truth",
        },
    ]
    if repeated:
        regions.append(
            {
                **regions[1],
                "id": "repeated-target",
                "baseline_xy": [50, 522],
                "region_bbox": [40, 508, 315, 550],
                "source_operation": "content object 4, third text show",
            }
        )
    truth = {
        "schema": "region-fusion-fixture-truth-v1",
        "fixture": name,
        "input_sha256": hashlib.sha256(data).hexdigest(),
        "frame": FRAME,
        "regions": regions,
        "truth_basis": (
            "Predeclared generator strings, glyph-to-Unicode table and PDF text-show operators; "
            "never extracted candidate text."
        ),
        "region_bbox_semantics": (
            "Generously enclosing source-authored regions, "
            "not exact painted glyph bounds or backend measurements."
        ),
        "encoding": "custom identity CMap stream" if custom_encoding else "named Identity-H",
        "to_unicode": "empty valid CMap"
        if empty_unicode
        else "complete compact bfchar CMap for every shown glyph",
        "repeated_text": repeated,
        "purpose": "negative: mapping absent"
        if empty_unicode
        else (
            "positive: unsupported baseline encoding shape"
            if custom_encoding
            else "control: both encodings supported"
        ),
    }
    (ROOT / f"{name}.truth.json").write_text(json.dumps(truth, indent=2, sort_keys=True) + "\n")
    return {"name": name, "pdf_sha256": truth["input_sha256"]}


if __name__ == "__main__":
    cases = [
        write_pdf("positive-stream-cmap", True),
        write_pdf("control-named-cmap", False),
        write_pdf("negative-empty-tounicode", True, empty_unicode=True),
        write_pdf("repeated-stream-cmap", True, repeated=True),
    ]
    print(json.dumps(cases, indent=2))
