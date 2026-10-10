"""Report whole-process peak RSS and wall time of `tpe extract` per PDF.

Usage: uv run python scripts/measure_rss.py target/release/tpe FILE.pdf [...]

Each file is extracted in a fresh child process (`--jobs 1`, JSON output to a
temporary database). Peak RSS is `getrusage(RUSAGE_CHILDREN).ru_maxrss`
sampled after the child exits, so it covers the controller and its disposable
worker. Linux reports KiB and macOS bytes; both are normalised to MiB.
See docs/MEMORY.md for the measurements this script produced.
"""

from __future__ import annotations

import json
import os
import resource
import subprocess
import sys
import tempfile
import time


def _find(obj: object, keys: tuple[str, ...]) -> object | None:
    if isinstance(obj, dict):
        for key in keys:
            if key in obj:
                return obj[key]
        for value in obj.values():
            found = _find(value, keys)
            if found is not None:
                return found
    if isinstance(obj, list):
        for value in obj:
            found = _find(value, keys)
            if found is not None:
                return found
    return None


def _maxrss_mib(raw: int) -> float:
    unit = 1024 if sys.platform != "darwin" else 1024 * 1024
    return raw * (1024 / unit) / 1024


def measure(tpe: str, path: str) -> tuple[str, int, str, float, float, str]:
    with tempfile.TemporaryDirectory() as tmp:
        db = os.path.join(tmp, "m.db")
        before = resource.getrusage(resource.RUSAGE_CHILDREN).ru_maxrss
        start = time.monotonic()
        proc = subprocess.run(  # noqa: S603 -- argv to the local tpe binary, no shell
            [tpe, "extract", path, "--db", db, "--json", "--jobs", "1"],
            capture_output=True,
            text=True,
            check=False,
        )
        wall = time.monotonic() - start
        after = resource.getrusage(resource.RUSAGE_CHILDREN).ru_maxrss
    pages = "?"
    status = f"exit {proc.returncode}"
    try:
        report = json.loads(proc.stdout)
        count = _find(report, ("page_count", "pageCount"))
        if count is None:
            listed = _find(report, ("pages",))
            count = len(listed) if isinstance(listed, list) else None
        pages = str(count) if count is not None else "?"
        found = _find(report, ("status",))
        if found is not None:
            status = str(found)[:30]
    except json.JSONDecodeError:
        pass
    # ru_maxrss is a high-water mark over all children; a fresh interpreter per
    # file keeps it per-file, which is how docs/MEMORY.md was produced.
    peak = _maxrss_mib(max(after, before))
    return os.path.basename(path), os.path.getsize(path), pages, peak, wall, status


def main(argv: list[str]) -> int:
    if len(argv) < 3:
        print(__doc__, file=sys.stderr)
        return 2
    tpe, files = argv[1], argv[2:]
    if len(files) > 1:
        # Re-exec per file so RUSAGE_CHILDREN's high-water mark is per file.
        for path in files:
            subprocess.run(  # noqa: S603 -- re-exec of this script, argv only
                [sys.executable, "-I", argv[0], tpe, path], check=False
            )
        return 0
    name, size, pages, peak, wall, status = measure(tpe, files[0])
    print(f"{name:28} {size:9} {pages:>6} {peak:9.1f} MiB {wall:7.2f} s {status}")
    return 0


if __name__ == "__main__":
    raise SystemExit(main(sys.argv))
