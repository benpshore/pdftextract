#!/usr/bin/env python3
"""Capture source-frozen inputs without reading private truth or plan files."""

import argparse
import hashlib
import json
import os
import subprocess
import tempfile
from pathlib import Path

import fitz

ROOT = Path(__file__).resolve().parent


def reference(path):
    data = path.read_bytes()
    return {"sha256": hashlib.sha256(data).hexdigest(), "size": len(data)}


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("--binary", type=Path, required=True)
    parser.add_argument("--pdfium-library", type=Path, required=True)
    parser.add_argument("--render-dir", type=Path, required=True)
    args = parser.parse_args()
    binary = args.binary.resolve(strict=True)
    runtime = args.pdfium_library.resolve(strict=True)
    if reference(runtime)["sha256"] != (
        "7670b3c597b02dfa3f98b23b49c3bb52536312f1ea686b739321731b6011f5a9"
    ):
        raise RuntimeError("Expected reviewed PDFium8066 Linux x64 binary")
    freeze = json.loads((ROOT / "input-freeze.json").read_text())
    capture = ROOT / "captured"
    capture.mkdir(exist_ok=False)
    args.render_dir.mkdir(parents=True, exist_ok=False)
    env = os.environ.copy()
    env["PDFIUM_DYNAMIC_LIB_PATH"] = str(runtime)
    # Operator-selected resolved executable; fixed argv and no shell.
    version = subprocess.check_output([str(binary), "--version"], text=True).strip()  # noqa: S603
    record = {
        "schema": "source-fusion-native-capture-v1",
        "source_freeze": reference(ROOT / "input-freeze.json"),
        "truth_access": "This capture script does not read private plan/truth files.",
        "binary": {"path": str(binary), "version_output": version, **reference(binary)},
        "pdfium": {"release": "chromium/8066", **reference(runtime)},
        "renderer": {
            "pymupdf": fitz.VersionBind,
            "mupdf": fitz.VersionFitz,
            "scale": 2,
            "width_pixels": 1224,
            "height_pixels": 1584,
        },
        "cases": [],
    }
    for case in freeze["cases"]:
        pdf = ROOT / case["path"]
        if reference(pdf) != {key: case[key] for key in ("sha256", "size")}:
            raise RuntimeError("Frozen PDF identity changed")
        row = {"id": case["id"], "source": reference(pdf), "runs": {}}
        for backend in ("lopdf", "pdfium"):
            target = capture / case["id"] / backend
            target.mkdir(parents=True)
            with tempfile.TemporaryDirectory(prefix="source-fusion-ledger-") as temp:
                argv = [
                    str(binary),
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
                # Operator-selected executable plus source-frozen synthetic input; no shell.
                result = subprocess.run(  # noqa: S603
                    argv,
                    env=env,
                    capture_output=True,
                    text=True,
                    timeout=60,
                )
            (target / "stdout.jsonl").write_text(result.stdout)
            (target / "stderr.txt").write_text(result.stderr)
            outputs = list(target.glob("*.json"))
            if len(outputs) != 1:
                raise RuntimeError(f"Missing native evidence: {case['id']}/{backend}")
            artifact = outputs[0]
            parsed = json.loads(artifact.read_text())
            row["runs"][backend] = {
                "exit_code": result.returncode,
                "status": parsed["status"],
                "backend": parsed["backend"],
                "artifact": {"path": str(artifact.relative_to(ROOT)), **reference(artifact)},
            }
        png = args.render_dir / f"{case['id']}.png"
        with fitz.open(pdf) as document:
            document[0].get_pixmap(matrix=fitz.Matrix(2, 2), alpha=False).save(png)
        row["private_render"] = {"filename": png.name, **reference(png)}
        record["cases"].append(row)
        print(case["id"], "captured both backends and private render", flush=True)
    (capture / "manifest.json").write_text(json.dumps(record, indent=2, sort_keys=True) + "\n")


if __name__ == "__main__":
    main()
