"""Download only the four public visual witnesses, verifying original #272 hashes.

This opt-in CI/developer helper never discovers or uploads user files. PDF bytes
are stored outside git. A download is bounded to 2 MB and a 60 second timeout.
Usage: uv run python scripts/fetch_layout_witnesses.py OUTPUT_DIRECTORY
"""

import hashlib
import json
import sys
import urllib.request
from pathlib import Path
from urllib.parse import quote

ROOT = Path(__file__).resolve().parents[1]
MAX_BYTES = 2_000_000


def main():
    destination = Path(sys.argv[1])
    destination.mkdir(parents=True, exist_ok=True)
    corpus = json.loads((ROOT / "docs/analysis/layout-eval-2026-10-09/corpus.json").read_text())[
        "documents"
    ]
    truth = json.loads((ROOT / "tests/fixtures/layout-witnesses/expected.json").read_text())
    for case in truth["cases"]:
        source = next(item for item in corpus if item["id"] == case["id"])
        path = destination / f"{source['id']}.pdf"
        if path.exists():
            if path.stat().st_size > MAX_BYTES:
                raise ValueError(f"oversized existing file: {path}")
            data = path.read_bytes()
        else:
            url = (
                f"https://raw.githubusercontent.com/{source['source_repo']}/"
                f"{source['source_commit']}/{quote(source['source_path'])}"
            )
            with urllib.request.urlopen(url, timeout=60) as response:
                data = response.read(MAX_BYTES + 1)
        if len(data) > MAX_BYTES or hashlib.sha256(data).hexdigest() != source["sha256"]:
            raise ValueError(f"source hash/size mismatch: {source['id']}")
        if not path.exists():
            with path.open("xb") as output:
                output.write(data)
        print(f"verified {source['id']} {source['sha256']}")


if __name__ == "__main__":
    main()
