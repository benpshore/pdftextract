"""Python entry point for the separately installed native PDF engine."""

import os
import shutil
import sys


def main() -> None:
    """Replace this process with tpe, preserving arguments, streams and signals."""
    executable = shutil.which("tpe")
    if executable is None:
        print(
            "text-processing-engine: native tpe executable not found on PATH. "
            "Install a verified native release or build it with cargo build --release "
            "and add its directory to PATH.",
            file=sys.stderr,
        )
        raise SystemExit(127)
    try:
        # Do not capture PDF output or introduce another supervisor process: the
        # native extract command owns worker deadlines and resource containment.
        os.execv(executable, [executable, *sys.argv[1:]])  # noqa: S606
    except OSError as error:
        print(f"text-processing-engine: cannot execute {executable}: {error}", file=sys.stderr)
        raise SystemExit(126) from error
