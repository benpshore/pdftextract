"""Explicit, offline CPU OCR with disposable per-page POSIX process groups.

Run with uv run python scripts/cpu_ocr.py --help. This is a qualification CLI,
not the Rust extraction backend or a browser runtime. No network is used.
"""

import argparse
import concurrent.futures
import contextlib
import csv
import ctypes
import hashlib
import json
import math
import os
import re
import resource
import select
import shutil
import signal
import stat
import subprocess
import sys
import tempfile
import threading
import time
from pathlib import Path


class OcrError(Exception):
    """A safe, document-content-free operational failure."""


class Supervisor:
    """Bound command lifetime; native descendants share the command's group."""

    def __init__(self, timeout):
        self.started = time.monotonic()
        self.deadline = self.started + timeout
        self.cancelled = threading.Event()
        self.signum = None

    def check(self):
        if self.cancelled.is_set():
            raise OcrError("cancelled")
        if time.monotonic() >= self.deadline:
            raise OcrError("timeout")

    def signal(self, signum, _frame):
        self.signum = signum
        self.cancelled.set()

    def run(self, command, directory, memory_mib, *, page=False):
        self.check()
        mode = "__page" if page else "__exec"
        args = [sys.executable, str(Path(__file__).resolve()), mode, str(memory_mib), *command]
        env = dict(os.environ, OMP_THREAD_LIMIT="1", OMP_NUM_THREADS="1", LC_ALL="C")
        with (directory / "stdout").open("wb") as out, (directory / "stderr").open("wb") as err:
            # Explicit argv, no shell. A new group contains worker and native grandchildren.
            process = subprocess.Popen(  # noqa: S603
                args,
                stdin=subprocess.DEVNULL,
                stdout=out,
                stderr=err,
                cwd=directory,
                env=env,
                start_new_session=True,
            )
            try:
                while process.poll() is None:
                    self.check()
                    try:
                        process.wait(timeout=0.03)
                    except subprocess.TimeoutExpired:
                        continue
                self.check()
                if process.returncode:
                    raise OcrError(f"worker_exit_{process.returncode}")
            finally:
                # Also kill descendants if their immediate parent exited first.
                with contextlib.suppress(ProcessLookupError):
                    os.killpg(process.pid, signal.SIGKILL)
                process.wait()
                # main() makes this Linux process a subreaper. Reap adopted descendants
                # in this command's group without touching another page worker's children.
                while True:
                    try:
                        os.waitpid(-process.pid, 0)
                    except ChildProcessError:
                        break


def sha256(path):
    with path.open("rb") as handle:
        return hashlib.file_digest(handle, "sha256").hexdigest()


def snapshot(source, target, supervisor):
    """Copy one regular-file descriptor; detect changes without chasing growth."""
    fd = os.open(source, os.O_RDONLY | os.O_NONBLOCK)
    with os.fdopen(fd, "rb") as original, target.open("xb") as dest:
        before = os.fstat(original.fileno())
        if not stat.S_ISREG(before.st_mode):
            raise OcrError("input_not_regular_file")
        digest = hashlib.sha256()
        remaining = before.st_size
        while remaining:
            supervisor.check()
            chunk = original.read(min(65536, remaining))
            if not chunk:
                raise OcrError("input_changed_during_snapshot")
            dest.write(chunk)
            digest.update(chunk)
            remaining -= len(chunk)
        after = os.fstat(original.fileno())
        if original.read(1) or (before.st_size, before.st_mtime_ns, before.st_ctime_ns) != (
            after.st_size,
            after.st_mtime_ns,
            after.st_ctime_ns,
        ):
            raise OcrError("input_changed_during_snapshot")
    target.chmod(0o400)
    return {"path": str(source.resolve()), "sha256": digest.hexdigest(), "bytes": before.st_size}


def input_kind(path):
    with path.open("rb") as handle:
        head = handle.read(16)
    if head.startswith(b"%PDF-"):
        return "pdf"
    if head.startswith(
        (b"\x89PNG\r\n\x1a\n", b"\xff\xd8\xff", b"P5\n", b"P6\n", b"II*\0", b"MM\0*")
    ):
        return "image"
    raise OcrError("unsupported_input_use_pdf_png_jpeg_tiff_or_binary_pnm")


def runtime_identity(command, option, root, supervisor, memory_mib):
    candidate = shutil.which(command)
    if candidate is None:
        raise OcrError(f"missing_executable_{Path(command).name}")
    path = Path(candidate).resolve()
    with path.open("rb") as handle:
        magic = handle.read(4)
    if magic not in (b"\x7fELF", b"\xcf\xfa\xed\xfe", b"\xfe\xed\xfa\xcf", b"\xca\xfe\xba\xbe"):
        raise OcrError(f"native_binary_required_not_launcher_{path.name}")
    directory = root / (path.name + "-identity")
    directory.mkdir()
    supervisor.run([str(path), option], directory, memory_mib)
    version = b""
    for channel in ("stdout", "stderr"):
        with (directory / channel).open("rb") as handle:
            version += handle.read(4096)
    lines = version.decode("utf-8", errors="replace").splitlines()
    return {"path": str(path), "sha256": sha256(path), "version": lines[0] if lines else "unknown"}


def read_tsv(path):
    words = []
    dimensions = None
    last_line = None
    text_parts = []
    with path.open(encoding="utf-8", newline="") as handle:
        rows = csv.DictReader(handle, delimiter="\t", quoting=csv.QUOTE_NONE)
        for row in rows:
            level = int(row["level"])
            if int(row["page_num"]) != 1:
                raise OcrError("multiple_image_frames_unsupported")
            x, y, width, height = (int(row[key]) for key in ("left", "top", "width", "height"))
            if level == 1:
                if dimensions is not None or x != 0 or y != 0 or width <= 0 or height <= 0:
                    raise OcrError("invalid_page_geometry")
                dimensions = (width, height)
            if level != 5 or not row["text"].strip():
                continue
            confidence = float(row["conf"])
            if not math.isfinite(confidence) or not 0 <= confidence <= 100:
                raise OcrError("invalid_word_confidence")
            if (
                dimensions is None
                or min(x, y) < 0
                or min(width, height) <= 0
                or x + width > dimensions[0]
                or y + height > dimensions[1]
            ):
                raise OcrError("invalid_word_geometry")
            line = tuple(int(row[key]) for key in ("block_num", "par_num", "line_num"))
            if text_parts:
                text_parts.append(" " if line == last_line else "\n")
            text_parts.append(row["text"])
            last_line = line
            words.append(
                {
                    "text": row["text"],
                    "confidence": confidence,
                    "bbox": [x, y, width, height],
                    "block": line[0],
                    "paragraph": line[1],
                    "line": line[2],
                }
            )
    if dimensions is None:
        raise OcrError("missing_page_geometry")
    return words, "".join(text_parts), dimensions


def native_run(args, directory, name):
    with (
        (directory / f"{name}.stdout").open("wb") as out,
        (directory / f"{name}.stderr").open("wb") as err,
    ):
        # The outer supervisor kills this entire worker group on cancellation/deadline.
        result = subprocess.run(  # noqa: S603
            args, stdin=subprocess.DEVNULL, stdout=out, stderr=err, check=False
        )
    if result.returncode:
        raise OcrError(f"{name}_exit_{result.returncode}")


def page_worker(config_path):
    """Runs under RLIMIT_AS; parsing/JSON allocation is contained with native OCR."""
    config = json.loads(Path(config_path).read_text())
    directory = Path(config["directory"])
    started = time.monotonic()
    image = Path(config["snapshot"])
    raster_seconds = 0.0
    if config["kind"] == "pdf":
        native_run(
            [
                config["pdftoppm"],
                "-f",
                str(config["page"]),
                "-l",
                str(config["page"]),
                "-r",
                str(config["dpi"]),
                "-singlefile",
                "-gray",
                str(image),
                str(directory / "raster"),
            ],
            directory,
            "rasterize",
        )
        image = directory / "raster.pgm"
        raster_seconds = time.monotonic() - started
    image_hash = sha256(image)
    before_ocr = time.monotonic()
    native_run(
        [
            config["tesseract"],
            str(image),
            str(directory / "recognition"),
            "--tessdata-dir",
            config["models_dir"],
            "-l",
            config["language"],
            "--oem",
            "1",
            "--psm",
            str(config["psm"]),
            "-c",
            "tessedit_create_tsv=1",
        ],
        directory,
        "ocr",
    )
    ocr_seconds = time.monotonic() - before_ocr
    before_parse = time.monotonic()
    tsv = directory / "recognition.tsv"
    words, text, (width, height) = read_tsv(tsv)
    warnings = ["ocr_is_inferred_text_not_verified_complete", "automatic_orientation_not_performed"]
    if config["kind"] == "pdf":
        warnings.append("pdf_coordinate_transform_unverified")
    if not words:
        warnings.append("no_text_recognized_does_not_prove_blank")
    if any(word["confidence"] < 80 for word in words):
        warnings.append("low_confidence_words")
    if (directory / "ocr.stderr").stat().st_size:
        warnings.append(
            "tesseract_diagnostics_retained_locally"
            if config["artifacts_dir"]
            else "tesseract_diagnostics_present_not_retained"
        )
    artifacts = {"tsv": {"sha256": sha256(tsv), "path": None}}
    if config["artifacts_dir"]:
        destination = Path(config["artifacts_dir"]) / f"page-{config['page']:06d}.tsv"
        with destination.open("xb") as output, tsv.open("rb") as source:
            shutil.copyfileobj(source, output, length=65536)
        artifacts["tsv"]["path"] = str(destination)
        for name in ("ocr.stderr", "rasterize.stderr"):
            diagnostic = directory / name
            if diagnostic.exists():
                target = destination.with_suffix("." + name)
                with target.open("xb") as output, diagnostic.open("rb") as source:
                    shutil.copyfileobj(source, output, length=65536)
    record = {
        "event": "page",
        "page": config["page"],
        "status": "partial",
        "text": text,
        "source_sha256": config["source_sha256"],
        "words": words,
        "raster": {
            "width": width,
            "height": height,
            "sha256": image_hash,
            "coordinate_space": "raster_pixels_top_left",
            "pdf_transform": None,
            "dpi": config["dpi"] if config["kind"] == "pdf" else None,
        },
        "warnings": warnings,
        "artifacts": artifacts,
        "timing_seconds": {
            "rasterize": raster_seconds,
            "ocr": ocr_seconds,
            "parse": time.monotonic() - before_parse,
            "total": time.monotonic() - started,
        },
        "resources": {
            "child_max_rss_kib": resource.getrusage(resource.RUSAGE_CHILDREN).ru_maxrss,
            "worker_max_rss_kib": resource.getrusage(resource.RUSAGE_SELF).ru_maxrss,
            "rss_scope": "separate_process_maxima_not_simultaneous_tree_peak",
        },
    }
    with (directory / "result.jsonl").open("w", encoding="utf-8") as output:
        json.dump(record, output, ensure_ascii=False, allow_nan=False)
        output.write("\n")


def positive_int(value):
    parsed = int(value)
    if parsed <= 0:
        raise argparse.ArgumentTypeError("must be positive")
    return parsed


def positive_float(value):
    parsed = float(value)
    if not math.isfinite(parsed) or parsed <= 0:
        raise argparse.ArgumentTypeError("must be positive and finite")
    return parsed


def parser():
    result = argparse.ArgumentParser(description=__doc__)
    result.add_argument("input", type=Path)
    result.add_argument("--tessdata-dir", type=Path, required=True)
    result.add_argument("--language", default="eng")
    result.add_argument("--workers", type=positive_int, default=1)
    result.add_argument("--dpi", type=positive_int, default=200)
    result.add_argument("--timeout-seconds", type=positive_float, default=120)
    result.add_argument("--memory-mib", type=positive_int, default=1024)
    result.add_argument("--psm", type=int, choices=[3, 4, 5, 6, 7, 8, 9, 10, 11, 13], default=6)
    result.add_argument("--tesseract", default="tesseract")
    result.add_argument("--pdftoppm", default="pdftoppm")
    result.add_argument("--pdfinfo", default="pdfinfo")
    result.add_argument(
        "--artifacts-dir", type=Path, help="new private directory for TSV/diagnostics"
    )
    result.add_argument("--output", type=Path, help="new JSONL file; defaults to stdout")
    return result


def write_output(output, data, supervisor):
    """Respect cancellation/deadline even if a pipe reader stops consuming JSONL."""
    view = memoryview(data)
    while view:
        supervisor.check()
        if not select.select([], [output.fileno()], [], 0.03)[1]:
            continue
        try:
            written = os.write(output.fileno(), view)
            view = view[written:]
        except BlockingIOError:
            continue


def emit(output, record, supervisor=None):
    deadline = supervisor or Supervisor(1.0)
    write_output(
        output, (json.dumps(record, ensure_ascii=False, allow_nan=False) + "\n").encode(), deadline
    )


def execute(args, supervisor, output, root):
    if not re.fullmatch(r"[A-Za-z0-9_-]+(?:\+[A-Za-z0-9_-]+)*", args.language):
        raise OcrError("invalid_language")
    snapshot_path = root / "input"
    source = snapshot(args.input, snapshot_path, supervisor)
    kind = input_kind(snapshot_path)
    source["kind"] = kind
    models_dir = root / "models"
    models_dir.mkdir()
    models = []
    for language in dict.fromkeys(args.language.split("+")):
        model = snapshot(
            args.tessdata_dir / f"{language}.traineddata",
            models_dir / f"{language}.traineddata",
            supervisor,
        )
        model["language"] = language
        models.append(model)
    runtime = {
        "tesseract": runtime_identity(
            args.tesseract, "--version", root, supervisor, args.memory_mib
        ),
        "models": models,
        "execution_provider": "native_cpu_tesseract_lstm",
        "native_dependency_closure": "not_fingerprinted",
    }
    total_pages = 1
    if kind == "pdf":
        for name in ("pdfinfo", "pdftoppm"):
            runtime[name] = runtime_identity(
                getattr(args, name), "-v", root, supervisor, args.memory_mib
            )
        info_dir = root / "info"
        info_dir.mkdir()
        supervisor.run([runtime["pdfinfo"]["path"], str(snapshot_path)], info_dir, args.memory_mib)
        found_pages = None
        with (info_dir / "stdout").open("rb") as handle:
            continuation = False
            while line := handle.readline(4096):
                supervisor.check()
                if not continuation and re.fullmatch(rb"Pages:\s+[0-9]+\s*\n", line):
                    # Metadata precedes the real Pages field; use the last matching field.
                    found_pages = int(line.split(b":", 1)[1])
                continuation = not line.endswith(b"\n")
        if found_pages is None:
            raise OcrError("missing_pdf_page_count")
        total_pages = found_pages
        if total_pages < 1:
            raise OcrError("invalid_pdf_page_count")
    options = {
        "workers": args.workers,
        "dpi": args.dpi,
        "psm": args.psm,
        "language": args.language,
        "memory_mib_per_process": args.memory_mib,
        "timeout_seconds": args.timeout_seconds,
        "omp_thread_limit": 1,
    }
    emit(
        output,
        {
            "event": "start",
            "source": source,
            "runtime": runtime,
            "options": options,
            "total_pages": total_pages,
        },
        supervisor,
    )
    common = {
        "snapshot": str(snapshot_path),
        "kind": kind,
        "dpi": args.dpi,
        "psm": args.psm,
        "language": args.language,
        "models_dir": str(models_dir),
        "tesseract": runtime["tesseract"]["path"],
        "source_sha256": source["sha256"],
        "pdftoppm": runtime.get("pdftoppm", {}).get("path"),
        "artifacts_dir": str(args.artifacts_dir.resolve()) if args.artifacts_dir else None,
    }

    def process_page(page):
        directory = root / f"page-{page}"
        directory.mkdir()
        config_path = directory / "config.json"
        config_path.write_text(json.dumps(dict(common, directory=str(directory), page=page)))
        supervisor.run([str(config_path)], directory, args.memory_mib, page=True)
        return directory / "result.jsonl", (directory / "status").read_text() == "success"

    completed = failed = submitted = 0
    outcome = "success"
    pool = concurrent.futures.ThreadPoolExecutor(max_workers=args.workers)
    pending = {}
    try:
        while submitted < total_pages or pending:
            supervisor.check()
            while submitted < total_pages and len(pending) < args.workers:
                submitted += 1
                pending[pool.submit(process_page, submitted)] = submitted
            done, _ = concurrent.futures.wait(
                pending, timeout=0.03, return_when=concurrent.futures.FIRST_COMPLETED
            )
            for future in done:
                page = pending.pop(future)
                try:
                    result_path, successful = future.result()
                    with result_path.open("rb") as record:
                        while chunk := record.read(65536):
                            write_output(output, chunk, supervisor)
                    if successful:
                        completed += 1
                    else:
                        failed += 1
                        outcome = "failed"
                except OcrError as exc:
                    if str(exc) in ("timeout", "cancelled"):
                        raise
                    failed += 1
                    outcome = "failed"
                    emit(
                        output, {"event": "page_error", "page": page, "error": str(exc)}, supervisor
                    )
                finally:
                    shutil.rmtree(root / f"page-{page}")
    except OcrError as exc:
        if str(exc) not in ("timeout", "cancelled"):
            raise
        outcome = str(exc)
    finally:
        supervisor.cancelled.set()
        pool.shutdown(wait=True, cancel_futures=True)
    emit(
        output,
        {
            "event": "summary",
            "outcome": outcome,
            "completed_pages": completed,
            "failed_pages": failed,
            "total_pages": total_pages,
            "unattempted_pages": total_pages - submitted,
            "interrupted_pages": submitted - completed - failed,
            "timing_seconds": {"total": time.monotonic() - supervisor.started},
            "warnings": ["ocr_results_are_partial", "native_shared_libraries_not_fingerprinted"],
        },
    )
    return {"success": 0, "failed": 1, "timeout": 124, "cancelled": 128 + (supervisor.signum or 2)}[
        outcome
    ]


def enable_subreaper():
    # Linux subreaper ensures worker grandchildren are adopted and waitable here.
    libc = ctypes.CDLL(None, use_errno=True)
    if libc.prctl(36, 1, 0, 0, 0) != 0:  # PR_SET_CHILD_SUBREAPER
        raise OcrError("cannot_enable_child_subreaper")


def main(argv=None):
    args = parser().parse_args(argv)
    if sys.platform != "linux":
        print("CPU OCR qualification currently requires Linux", file=sys.stderr)
        return 1
    supervisor = Supervisor(args.timeout_seconds)
    try:
        enable_subreaper()
    except OcrError:
        print("CPU OCR failed: cannot_enable_child_subreaper", file=sys.stderr)
        return 1
    old_handlers = {
        sig: signal.signal(sig, supervisor.signal) for sig in (signal.SIGINT, signal.SIGTERM)
    }
    output = None
    close_output = False
    try:
        if args.artifacts_dir:
            args.artifacts_dir.mkdir(mode=0o700)
        output = args.output.open("xb") if args.output else sys.stdout.buffer
        close_output = bool(args.output)
        if not stat.S_ISREG(os.fstat(output.fileno()).st_mode):
            # A fresh open description avoids changing the invoking shell's flags.
            # Unsupported inherited sockets fail explicitly; --output remains available.
            fd = os.open(
                f"/proc/self/fd/{output.fileno()}", os.O_WRONLY | os.O_NONBLOCK | os.O_NOCTTY
            )
            output = os.fdopen(fd, "wb", buffering=0)
            close_output = True
        with tempfile.TemporaryDirectory(prefix="tpe-cpu-ocr-") as temporary:
            return execute(args, supervisor, output, Path(temporary))
    except (OcrError, OSError, ValueError, KeyError, MemoryError) as exc:
        # Do not expose native stderr, paths or recognized document text in generic errors.
        code = str(exc) if isinstance(exc, OcrError) else type(exc).__name__
        if output is not None:
            with contextlib.suppress(OSError, OcrError):
                emit(output, {"event": "error", "error": code})
        print(f"CPU OCR failed: {code}", file=sys.stderr)
        return (
            124
            if code == "timeout"
            else (128 + (supervisor.signum or 2) if code == "cancelled" else 1)
        )
    finally:
        if output is not None and close_output:
            output.close()
        for sig, handler in old_handlers.items():
            signal.signal(sig, handler)


if __name__ == "__main__":
    if len(sys.argv) > 1 and sys.argv[1] in ("__exec", "__page"):
        limit = int(sys.argv[2]) * 1024 * 1024
        resource.setrlimit(resource.RLIMIT_AS, (limit, limit))
        resource.setrlimit(resource.RLIMIT_CORE, (0, 0))
        if sys.argv[1] == "__exec":
            os.execv(sys.argv[3], sys.argv[3:])  # noqa: S606 -- explicit trusted native executable
        else:
            started = time.monotonic()
            config = json.loads(Path(sys.argv[3]).read_text())
            directory = Path(config["directory"])
            try:
                page_worker(sys.argv[3])
                (directory / "status").write_text("success")
            except (OcrError, OSError, ValueError, KeyError, MemoryError) as exc:
                code = str(exc) if isinstance(exc, OcrError) else type(exc).__name__
                record = {
                    "event": "page_error",
                    "page": config["page"],
                    "error": code,
                    "source_sha256": config["source_sha256"],
                    "timing_seconds": {"total": time.monotonic() - started},
                }
                (directory / "result.jsonl").write_text(json.dumps(record) + "\n")
                (directory / "status").write_text("failed")
    else:
        raise SystemExit(main())
