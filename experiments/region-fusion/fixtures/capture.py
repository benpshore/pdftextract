#!/usr/bin/env python3
"""Capture actual backend output and two renderer views of synthetic fixtures."""

import argparse
import ctypes as c
import hashlib
import json
import os
import subprocess
import tempfile
from pathlib import Path

import fitz
from PIL import Image

ROOT = Path(__file__).resolve().parent
CASES = [
    "positive-stream-cmap",
    "control-named-cmap",
    "negative-empty-tounicode",
    "repeated-stream-cmap",
]


def digest(path):
    data = Path(path).read_bytes()
    return {"sha256": hashlib.sha256(data).hexdigest(), "size": len(data)}


def render_pdfium(runtime, pdf, output):
    library = c.CDLL(str(runtime))
    signatures = {
        "FPDF_InitLibrary": (None, []),
        "FPDF_DestroyLibrary": (None, []),
        "FPDF_LoadDocument": (c.c_void_p, [c.c_char_p, c.c_char_p]),
        "FPDF_CloseDocument": (None, [c.c_void_p]),
        "FPDF_LoadPage": (c.c_void_p, [c.c_void_p, c.c_int]),
        "FPDF_ClosePage": (None, [c.c_void_p]),
        "FPDFBitmap_Create": (c.c_void_p, [c.c_int, c.c_int, c.c_int]),
        "FPDFBitmap_Destroy": (None, [c.c_void_p]),
        "FPDFBitmap_FillRect": (
            c.c_int,
            [c.c_void_p, c.c_int, c.c_int, c.c_int, c.c_int, c.c_ulong],
        ),
        "FPDF_RenderPageBitmap": (
            None,
            [c.c_void_p, c.c_void_p, c.c_int, c.c_int, c.c_int, c.c_int, c.c_int, c.c_int],
        ),
        "FPDFBitmap_GetBuffer": (c.c_void_p, [c.c_void_p]),
        "FPDFBitmap_GetStride": (c.c_int, [c.c_void_p]),
    }
    for name, (returns, args) in signatures.items():
        function = getattr(library, name)
        function.restype = returns
        function.argtypes = args
    library.FPDF_InitLibrary()
    document = page = bitmap = None
    try:
        document = library.FPDF_LoadDocument(os.fsencode(pdf), None)
        if not document:
            raise RuntimeError("PDFium document open failed")
        page = library.FPDF_LoadPage(document, 0)
        if not page:
            raise RuntimeError("PDFium page open failed")
        width, height = 1224, 1584
        bitmap = library.FPDFBitmap_Create(width, height, 0)
        if not bitmap:
            raise RuntimeError("PDFium bitmap allocation failed")
        if not library.FPDFBitmap_FillRect(bitmap, 0, 0, width, height, 0xFFFFFFFF):
            raise RuntimeError("PDFium bitmap initialization failed")
        library.FPDF_RenderPageBitmap(bitmap, page, 0, 0, width, height, 0, 0)
        stride = library.FPDFBitmap_GetStride(bitmap)
        pointer = library.FPDFBitmap_GetBuffer(bitmap)
        # FPDFBitmap_Create uses four bytes per pixel for these fixed-width bitmaps.
        # An exact stride check also bounds the native buffer copy below.
        if not pointer or stride != width * 4:
            raise RuntimeError("Unexpected PDFium bitmap contract")
        image = Image.frombytes(
            "RGB", (width, height), c.string_at(pointer, stride * height), "raw", "BGRX", stride, 1
        )
        image.save(output, compress_level=9)
    finally:
        if bitmap:
            library.FPDFBitmap_Destroy(bitmap)
        if page:
            library.FPDF_ClosePage(page)
        if document:
            library.FPDF_CloseDocument(document)
        library.FPDF_DestroyLibrary()


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("--binary", type=Path, required=True)
    parser.add_argument("--pdfium-library", type=Path, required=True)
    args = parser.parse_args()
    args.binary = args.binary.resolve(strict=True)
    args.pdfium_library = args.pdfium_library.resolve(strict=True)
    if (
        digest(args.pdfium_library)["sha256"]
        != "7670b3c597b02dfa3f98b23b49c3bb52536312f1ea686b739321731b6011f5a9"
    ):
        raise RuntimeError("This capture requires the reviewed 8066 Linux x64 runtime")
    out = ROOT / "captured"
    out.mkdir(exist_ok=False)
    environment = os.environ.copy()
    environment["PDFIUM_DYNAMIC_LIB_PATH"] = str(args.pdfium_library)
    manifest = {
        "schema": "region-fusion-fixture-capture-v1",
        "binary": {
            "path": str(args.binary),
            **digest(args.binary),
            # The operator explicitly supplies this resolved executable; no shell is used.
            "version_output": subprocess.check_output(  # noqa: S603
                [str(args.binary), "--version"], text=True
            ).strip(),
        },
        "build_source_basis": (
            "Parent-provided unchanged main source "
            "e27e1fb28a5b40dbca7f517c6fe396e6f11ed4ec and main Cargo.lock; "
            "this script records binary identity, not a rebuild."
        ),
        "pdfium_runtime": {
            "path": str(args.pdfium_library),
            "release": "chromium/8066",
            **digest(args.pdfium_library),
        },
        "render_config": {
            "page_index": 0,
            "width_pixels": 1224,
            "height_pixels": 1584,
            "scale_pixels_per_point": 2,
            "rotation": 0,
            "pdfium_flags": 0,
        },
        "mupdf_renderer": {"pymupdf": fitz.VersionBind, "mupdf": fitz.VersionFitz},
        "cases": [],
    }
    for case in CASES:
        pdf = ROOT / f"{case}.pdf"
        case_dir = out / case
        case_dir.mkdir()
        row = {"fixture": case, "input": digest(pdf), "runs": {}, "renders": {}}
        for backend in ["lopdf", "pdfium"]:
            target = case_dir / backend
            target.mkdir()
            with tempfile.TemporaryDirectory(prefix="tpe-region-fixture-ledger-") as temp:
                argv = [
                    str(args.binary),
                    "extract",
                    str(pdf),
                    "--backend",
                    backend,
                    "--db",
                    str(Path(temp) / "ledger.sqlite"),
                    "--out",
                    str(target),
                    "--json",
                ]
                # Explicit operator-selected executable and fixed fixture argv; no shell.
                result = subprocess.run(  # noqa: S603
                    argv, env=environment, capture_output=True, text=True, timeout=60
                )
            (target / "stdout.jsonl").write_text(result.stdout)
            (target / "stderr.txt").write_text(result.stderr)
            outputs = list(target.glob("*.json"))
            if len(outputs) != 1:
                raise RuntimeError(
                    f"Expected one extraction artifact: {case}/{backend}: {result.stderr}"
                )
            artifact = outputs[0]
            extracted = json.loads(artifact.read_text())
            row["runs"][backend] = {
                "command": [
                    "<binary>",
                    "extract",
                    f"<fixtures>/{case}.pdf",
                    "--backend",
                    backend,
                    "--db",
                    "<temporary>/ledger.sqlite",
                    "--out",
                    f"<captured>/{case}/{backend}",
                    "--json",
                ],
                "exit_code": result.returncode,
                "artifact": {"path": str(artifact.relative_to(ROOT)), **digest(artifact)},
                "backend_identity": extracted["backend"],
                "status": extracted["status"],
                "page_text": extracted["pages"][0]["text"],
                "warnings": extracted["warnings"],
            }
        for renderer in ["pdfium", "mupdf"]:
            png = case_dir / f"page-1-{renderer}.png"
            if renderer == "pdfium":
                render_pdfium(args.pdfium_library, pdf, png)
            else:
                with fitz.open(pdf) as document:
                    document[0].get_pixmap(matrix=fitz.Matrix(2, 2), alpha=False).save(png)
            row["renders"][renderer] = {"path": str(png.relative_to(ROOT)), **digest(png)}
        manifest["cases"].append(row)
        print(
            case,
            {backend: (run["status"], run["page_text"]) for backend, run in row["runs"].items()},
            flush=True,
        )
    (out / "manifest.json").write_text(json.dumps(manifest, indent=2, sort_keys=True) + "\n")


if __name__ == "__main__":
    main()
