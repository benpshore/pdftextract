"""Shared helpers for the 2026-10-09 layout evaluation scripts (standard library only)."""

import hashlib
import json
import subprocess
import unicodedata
from pathlib import Path


def norm(text: str) -> str:
    return unicodedata.normalize("NFKC", text)


def sha256_file(path: Path) -> str:
    digest = hashlib.sha256()
    with path.open("rb") as handle:
        for block in iter(lambda: handle.read(1 << 20), b""):
            digest.update(block)
    return digest.hexdigest()


def run(cmd: list[str]) -> str:
    """Run a fixed local tool (pdftotext, qpdf) and return its stdout as text."""
    result = subprocess.run(  # noqa: S603 - fixed executable and argument vector, no shell
        cmd, capture_output=True, check=False
    )
    return result.stdout.decode("utf-8", errors="replace")


def load_corpus(path: Path) -> list[dict]:
    with path.open(encoding="utf-8") as handle:
        return json.load(handle)["documents"]


def engine_doc(json_dir: Path, digest: str) -> dict:
    with (json_dir / f"{digest}.json").open(encoding="utf-8") as handle:
        return json.load(handle)


def split_pages(text: str) -> list[str]:
    """Split pdftotext output on form feeds; drop the empty tail after the last one."""
    pages = text.split("\f")
    if pages and not pages[-1].strip():
        pages = pages[:-1]
    return pages


def lcs_len(a, b) -> int:
    """Length of the longest common subsequence, bit-parallel (Allison-Dix / Hyyro).

    Exact, not sampled. `a` and `b` are sequences of hashable items.
    """
    if not a or not b:
        return 0
    masks: dict = {}
    for index, item in enumerate(a):
        masks[item] = masks.get(item, 0) | (1 << index)
    full = (1 << len(a)) - 1
    vector = full
    for item in b:
        match = vector & masks.get(item, 0)
        vector = ((vector + match) | (vector - match)) & full
    return len(a) - vector.bit_count()
