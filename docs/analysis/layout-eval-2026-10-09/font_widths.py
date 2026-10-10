"""List simple fonts that carry no /Widths array, per document.

The lopdf backend advances every glyph of such a font by a fixed 500/1000 em
(`DEFAULT_WIDTH`), because it has no built-in metrics for the 14 standard
fonts and does not read widths from anywhere else.

Reads `qpdf --json` (version 2 layout, qpdf 11). Every font object in the
file is listed, whether or not a page uses it.

Usage: font_widths.py CORPUS_JSON PDF_DIR > font-widths.json
"""

import json
import sys
from pathlib import Path

from common import load_corpus, run

SIMPLE = {"/Type1", "/TrueType", "/MMType1"}
FONT_FILES = ("/FontFile", "/FontFile2", "/FontFile3")


def objects(pdf: Path) -> dict:
    data = json.loads(run(["qpdf", "--json", str(pdf)]))
    table = {}
    for group in data.get("qpdf", []):
        if not isinstance(group, dict):
            continue
        for key, value in group.items():
            if key.startswith("obj:") and isinstance(value, dict):
                body = value.get("value")
                if isinstance(body, dict):
                    table[key[4:]] = body
    return table


def resolve(table: dict, value):
    return table.get(value, {}) if isinstance(value, str) else (value or {})


def name(value) -> str:
    """qpdf writes names as `/Name` and, when not plain ASCII, as `n:/Name`."""
    text = value if isinstance(value, str) else "?"
    return text.removeprefix("n:").lstrip("/")


def main() -> None:
    corpus, pdf_dir = Path(sys.argv[1]), Path(sys.argv[2])
    report = {}
    for entry in load_corpus(corpus):
        table = objects(pdf_dir / f"{entry['id']}.pdf")
        simple, missing = 0, []
        for body in table.values():
            if body.get("/Type") != "/Font" or body.get("/Subtype") not in SIMPLE:
                continue
            simple += 1
            if "/Widths" in body:
                continue
            descriptor = resolve(table, body.get("/FontDescriptor"))
            embedded = isinstance(descriptor, dict) and any(k in descriptor for k in FONT_FILES)
            missing.append({"base_font": name(body.get("/BaseFont")), "embedded": embedded})
        missing.sort(key=lambda font: font["base_font"])
        report[entry["id"]] = {"simple_fonts": simple, "without_widths": missing}
    json.dump(report, sys.stdout, indent=1, sort_keys=True)
    print()


if __name__ == "__main__":
    main()
