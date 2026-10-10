"""Generate small, uncompressed PDFs from independent operator specifications.

No production extraction/layout code is imported. Expected .txt files are
hand-authored separately and are never written by this script.
Run: uv run python tests/fixtures/layout_text/generate.py
"""

from pathlib import Path

ROOT = Path(__file__).parent
CMAP = b"""/CIDInit /ProcSet findresource begin 12 dict begin begincmap
/CIDSystemInfo << /Registry (Adobe) /Ordering (UCS) /Supplement 0 >> def
/CMapName /Fixture def /CMapType 2 def
1 begincodespacerange <0000> <FFFF> endcodespacerange
8 beginbfchar
<0001> <00E9> <0002> <4E2D> <0003> <D83DDE00>
<0004> <0009> <0005> <000A> <0006> <0061> <0007> <0062> <0008> <0063>
endbfchar
endcmap CMapName currentdict /CMap defineresource pop end end"""


def stream(data: bytes) -> bytes:
    return b"<< /Length %d >>\nstream\n" % len(data) + data + b"\nendstream"


def pdf(content: str, rotation: int = 0) -> bytes:
    objects = [
        b"<< /Type /Catalog /Pages 2 0 R >>",
        b"<< /Type /Pages /Kids [3 0 R] /Count 1 /MediaBox [0 0 240 180] >>",
        (
            b"<< /Type /Page /Parent 2 0 R /Rotate %d /Contents 4 0 R "
            b"/Resources << /Font << /F1 5 0 R /F2 6 0 R >> >> >>" % rotation
        ),
        stream(content.encode("ascii")),
        (
            b"<< /Type /Font /Subtype /Type1 /BaseFont /Courier "
            b"/FirstChar 32 /LastChar 126 /Widths [" + b"600 " * 95 + b"] >>"
        ),
        (
            b"<< /Type /Font /Subtype /Type0 /BaseFont /Fixture "
            b"/Encoding /Identity-H /DescendantFonts [7 0 R] /ToUnicode 8 0 R >>"
        ),
        (
            b"<< /Type /Font /Subtype /CIDFontType2 /BaseFont /Fixture /DW 600 "
            b"/CIDSystemInfo << /Registry (Adobe) /Ordering (Identity) /Supplement 0 >> >>"
        ),
        stream(CMAP),
    ]
    result = bytearray(b"%PDF-1.5\n")
    offsets = [0]
    for index, body in enumerate(objects, 1):
        offsets.append(len(result))
        result.extend(f"{index} 0 obj\n".encode() + body + b"\nendobj\n")
    start = len(result)
    result.extend(f"xref\n0 {len(offsets)}\n0000000000 65535 f \n".encode())
    for offset in offsets[1:]:
        result.extend(f"{offset:010} 00000 n \n".encode())
    result.extend(
        f"trailer\n<< /Root 1 0 R /Size {len(offsets)} >>\nstartxref\n{start}\n%%EOF\n".encode()
    )
    return bytes(result)


def show(x: int, y: int, text: str, size: int = 10) -> str:
    return f"BT /F1 {size} Tf 1 0 0 1 {x} {y} Tm ({text}) Tj ET\n"


def generate() -> None:
    # Scrambled stream order; the title, indentation, table and blank rows
    # are designed explicitly on a 6-point horizontal / 12-point vertical grid.
    columns = "".join(
        show(x, y, text, size)
        for x, y, text, size in [
            (132, 132, "R1", 10),
            (12, 36, "END", 10),
            (132, 120, "R2", 10),
            (24, 120, "L2", 10),
            (72, 156, "REPORT", 20),
            (12, 132, "L1", 10),
            (132, 96, "R3", 10),
            (12, 96, "item", 10),
            (72, 72, "TABLE", 10),
            (132, 60, "C", 10),
            (12, 60, "A", 10),
            (72, 60, "B", 10),
        ]
    )
    (ROOT / "columns.pdf").write_bytes(pdf(columns))
    # Tight title-to-body spacing hides column gutters in the legacy XY-cut.
    # The expectations are a separate authored reading-order specification.
    two = (
        show(132, 120, "right second row")
        + show(12, 132, "left first row")
        + show(102, 144, "HEADING")
        + show(132, 132, "right first row")
        + show(12, 120, "left second row")
    )
    (ROOT / "semantic-two.pdf").write_bytes(pdf(two))
    three = (
        show(168, 120, "R2")
        + show(90, 132, "M1")
        + show(84, 144, "CENTER HEADING")
        + show(12, 120, "L2")
        + show(168, 132, "R1")
        + show(12, 132, "L1")
        + show(90, 120, "M2")
    )
    (ROOT / "semantic-three.pdf").write_bytes(pdf(three))
    special = (
        "BT /F2 10 Tf 1 0 0 1 12 144 Tm <000100020003> Tj ET\n"
        + show(18, 132, "CD")
        + show(12, 132, "AB")
        + "BT /F1 10 Tf 0 1 -1 0 216 24 Tm (VERTICAL) Tj ET\n"
        + "BT /F2 10 Tf 1 0 0 1 12 60 Tm <00060004000700050008> Tj ET\n"
    )
    (ROOT / "special.pdf").write_bytes(pdf(special))
    for rotation in [0, 90, 180, 270]:
        lines = []
        for y, font, shown in [(144, "F1", "(TOP)"), (132, "F2", "<000100020003>")]:
            matrix = {
                0: (1, 0, 0, 1, 12, y),
                90: (0, 1, -1, 0, 240 - y, 12),
                180: (-1, 0, 0, -1, 240 - 12, 180 - y),
                270: (0, -1, 1, 0, y, 180 - 12),
            }[rotation]
            values = " ".join(str(value) for value in matrix)
            lines.append(f"BT /{font} 10 Tf {values} Tm {shown} Tj ET\n")
        (ROOT / f"rotate-{rotation}.pdf").write_bytes(pdf("".join(lines), rotation))


if __name__ == "__main__":
    generate()
