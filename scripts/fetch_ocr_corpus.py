"""Fetch the explicitly pinned, benign Tesseract OCR qualification subset.

This downloads public test assets only; no input documents are uploaded.
The new destination retains its license, README, exact truth and original pixels.
"""

import argparse
import hashlib
import json
from datetime import UTC, datetime
from pathlib import Path
from urllib.request import urlopen

MANIFEST = Path(__file__).resolve().parent.parent / "docs/validation/cpu-ocr-public-corpus.json"


def fetch(destination):
    destination = destination.resolve()
    destination.mkdir(parents=True, exist_ok=False)
    manifest = json.loads(MANIFEST.read_text())
    commit = manifest["dataset"]["commit"]
    prefix = f"https://raw.githubusercontent.com/tesseract-ocr/test/{commit}/"
    for item in manifest["files"]:
        relative = Path(item["repository_path"])
        if (
            relative.is_absolute()
            or ".." in relative.parts
            or item["url"] != prefix + str(relative)
        ):
            raise ValueError("Unexpected corpus path or source URL")
        target = destination / relative
        target.parent.mkdir(parents=True, exist_ok=True)
        digest = hashlib.sha256()
        byte_count = 0
        # All URLs come from the reviewed immutable public corpus manifest.
        with urlopen(item["url"], timeout=30) as response, target.open("xb") as output:  # noqa: S310
            while chunk := response.read(65536):
                byte_count += len(chunk)
                if byte_count > item["bytes"]:
                    raise ValueError("Corpus asset exceeds pinned size")
                digest.update(chunk)
                output.write(chunk)
        if byte_count != item["bytes"] or digest.hexdigest() != item["sha256"]:
            raise ValueError("Corpus asset integrity mismatch")
        item["local_path"] = str(target)
    for fixture in manifest["fixtures"]:
        fixture["path"] = str(destination / fixture["path"])
        fixture["ground_truth_path"] = str(destination / fixture["ground_truth_path"])
    manifest["dataset"]["retrieved_at"] = datetime.now(UTC).isoformat()
    manifest["dataset"]["retrieval_manifest_sha256"] = hashlib.sha256(
        MANIFEST.read_bytes()
    ).hexdigest()
    output = destination / "manifest.json"
    output.write_text(json.dumps(manifest, indent=2) + "\n")
    return output


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument(
        "--destination", required=True, type=Path, help="New local corpus directory"
    )
    args = parser.parse_args()
    print(fetch(args.destination))


if __name__ == "__main__":
    main()
