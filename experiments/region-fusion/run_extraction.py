#!/usr/bin/env python3
"""Linux pilot: durable lopdf first, optional externally reviewed PDFium fusion.

Standard library only. This orchestrates explicitly supplied local executables;
it is resource containment, not a security sandbox for hostile executables.
Run with ``uv run --no-project python run_extraction.py --help``.
The review digest and authority must come from the caller's trusted adjudication
channel, never from a PDF or an extraction candidate. Selection consumes that
adjudication; it does not automatically verify visible transcription semantics.
"""

from __future__ import annotations

import argparse
import contextlib
import ctypes
import hashlib
import json
import math
import os
import resource
import selectors
import signal
import stat
import subprocess
import sys
import time
from pathlib import Path

MIB = 1024 * 1024
CHUNK = 64 * 1024
MAX_REVIEW = MIB
MAX_RENDER = 32 * MIB
MAX_TOOL = 1024 * MIB


class StopRun(Exception):
    def __init__(self, state: str, detail: str):
        super().__init__(detail)
        self.state = state


def linux_libc():
    libc = ctypes.CDLL(None, use_errno=True)
    # prctl is variadic in C; declare all five positions explicitly so pointer-
    # width unsigned-long arguments cannot be truncated by ctypes defaults.
    libc.prctl.argtypes = [ctypes.c_int] + [ctypes.c_ulong] * 4
    libc.prctl.restype = ctypes.c_int
    return libc


def owned_child_pids() -> list[int]:
    # Some Linux kernels omit /proc/.../children (CONFIG_CHECKPOINT_RESTORE).
    # /proc/PID/stat still identifies the runner's directly adopted children.
    result = []
    with os.scandir("/proc") as entries:
        for entry in entries:
            if not entry.name.isdigit():
                continue
            try:
                with open(f"/proc/{entry.name}/stat", encoding="ascii") as handle:
                    fields = handle.read(16384).rsplit(")", 1)[1].split()
                if int(fields[1]) == os.getpid():
                    result.append(int(entry.name))
            except OSError, ValueError, IndexError:
                continue  # A process can exit between enumeration and stat.
    return result


def sync_directory(path: Path) -> None:
    fd = os.open(path, os.O_RDONLY | os.O_DIRECTORY)
    try:
        os.fsync(fd)
    finally:
        os.close(fd)


def atomic_json(path: Path, value: dict) -> None:
    """Replace only our own journal; no pre-existing output directory is accepted."""
    temporary = path.with_name(path.name + ".pending")
    with temporary.open("xb") as handle:
        handle.write((json.dumps(value, indent=2, sort_keys=True) + "\n").encode())
        handle.flush()
        os.fsync(handle.fileno())
    os.replace(temporary, path)
    sync_directory(path.parent)


def regular_reader(path: Path):
    # O_NONBLOCK prevents a FIFO/device from hanging before fstat rejects it.
    fd = os.open(path, os.O_RDONLY | os.O_NOFOLLOW | os.O_NONBLOCK)
    try:
        if not stat.S_ISREG(os.fstat(fd).st_mode):
            raise StopRun("invalid_input", f"not a regular file: {path}")
        return os.fdopen(fd, "rb")
    except BaseException:
        os.close(fd)
        raise


def fingerprint(info: os.stat_result) -> tuple:
    return info.st_dev, info.st_ino, info.st_size, info.st_mtime_ns, info.st_ctime_ns


class Runner:
    def __init__(self, args: argparse.Namespace):
        self.args = args
        self.started = time.monotonic()
        self.deadline = self.started + args.timeout_ms / 1000
        self.cancelled = 0
        self.committed = False
        self.directory = args.out.absolute()
        self.document: dict = {
            "version": 1,
            "state": "running",
            "selection_lane": "external_trusted_adjudication",
            "semantic_verification": "not_performed_by_runner",
            "limits": {
                "shared_wall_timeout_ms": args.timeout_ms,
                "max_source_bytes": args.max_source_bytes,
                "max_output_bytes": args.max_output_bytes,
                "max_memory_growth_mib": args.max_memory_growth_mib,
                "controller_address_space_cap_mib": args.max_memory_growth_mib + 256,
            },
            "artifacts": {},
            "attempts": [],
            "events": [],
        }

    def check(self) -> None:
        if self.cancelled:
            raise StopRun("cancelled", f"received signal {self.cancelled}")
        if time.monotonic() >= self.deadline:
            raise StopRun("timed_out", "shared wall deadline exhausted")

    def signal(self, number: int, _frame) -> None:
        if not self.committed:
            self.cancelled = number

    def persist(self) -> None:
        self.document["elapsed_ms"] = round((time.monotonic() - self.started) * 1000)
        atomic_json(self.directory / "journal.json", self.document)

    def event(self, name: str, **details) -> None:
        self.document["events"].append({"event": name, **details})
        self.persist()

    def digest(self, path: Path, cap: int | None) -> tuple[str, int]:
        digest = hashlib.sha256()
        size = 0
        with regular_reader(path) as handle:
            before = os.fstat(handle.fileno())
            if cap is not None and before.st_size > cap:
                raise StopRun("output_limit", f"file exceeds {cap} bytes: {path.name}")
            while True:
                self.check()
                data = handle.read(CHUNK)
                if not data:
                    break
                size += len(data)
                if cap is not None and size > cap:
                    raise StopRun("output_limit", f"file exceeds {cap} bytes: {path.name}")
                digest.update(data)
            if fingerprint(before) != fingerprint(os.fstat(handle.fileno())):
                raise StopRun("artifact_changed", f"file changed while hashing: {path.name}")
        return digest.hexdigest(), size

    def snapshot(self, source: Path, name: str, cap: int | None) -> dict:
        """Copy to an exclusively created file, then publish immutable exact bytes."""
        self.check()
        destination = self.directory / name
        temporary = destination.with_name(destination.name + ".pending")
        digest = hashlib.sha256()
        size = 0
        with regular_reader(source) as input_file, temporary.open("xb") as output_file:
            before = os.fstat(input_file.fileno())
            if cap is not None and before.st_size > cap:
                raise StopRun("input_limit", f"input exceeds {cap} bytes: {source.name}")
            while True:
                self.check()
                data = input_file.read(CHUNK)
                if not data:
                    break
                size += len(data)
                if cap is not None and size > cap:
                    raise StopRun("input_limit", f"input exceeds {cap} bytes: {source.name}")
                digest.update(data)
                output_file.write(data)
            if fingerprint(before) != fingerprint(os.fstat(input_file.fileno())):
                raise StopRun("artifact_changed", f"input changed during snapshot: {source.name}")
            output_file.flush()
            os.fchmod(output_file.fileno(), 0o444)
            os.fsync(output_file.fileno())
        # link is exclusive and cannot overwrite an existing artifact.
        os.link(temporary, destination)
        temporary.unlink()
        sync_directory(self.directory)
        result = {"path": name, "sha256": digest.hexdigest(), "bytes": size}
        self.verify(result, cap)
        return result

    def verify(self, artifact: dict, cap: int | None) -> None:
        actual, size = self.digest(self.directory / artifact["path"], cap)
        if actual != artifact["sha256"] or size != artifact["bytes"]:
            raise StopRun("artifact_changed", f"artifact digest mismatch: {artifact['path']}")

    def tool(self, path: Path) -> dict:
        path = path.resolve(strict=True)
        if not path.is_file() or not os.access(path, os.X_OK):
            raise StopRun("invalid_input", f"not an executable file: {path}")
        digest, size = self.digest(path, MAX_TOOL)
        return {"path": str(path), "sha256": digest, "bytes": size}

    def verify_inputs(self) -> None:
        for name in ("source", "baseline", "candidate", "review", "render"):
            artifact = self.document["artifacts"].get(name)
            if artifact:
                cap = {
                    "source": artifact["bytes"],
                    "review": MAX_REVIEW,
                    "render": MAX_RENDER,
                }.get(name, self.args.max_output_bytes)
                self.verify(artifact, cap)

    def execute(self, name: str, command: list[str]) -> int:
        """Drain pipes incrementally with a hard retained-byte cap and one deadline."""
        self.check()
        identity = self.document["tools"]["fusion" if name == "fusion" else "tpe"]
        actual, size = self.digest(Path(identity["path"]), MAX_TOOL)
        if actual != identity["sha256"] or size != identity["bytes"]:
            raise StopRun("artifact_changed", f"{name} executable changed before launch")
        attempt = {"name": name, "state": "running", "argv": command}
        self.document["attempts"].append(attempt)
        self.persist()  # durable start record precedes process creation
        self.check()
        parent_pid = os.getpid()
        address_space = (self.args.max_memory_growth_mib + 256) * MIB
        output_cap = self.args.max_output_bytes
        cpu_seconds = max(1, math.ceil(self.deadline - time.monotonic()))

        def child_setup() -> None:
            # Runner is single-threaded. The real TPE controller supplies the same
            # PDEATHSIG kill guarantee to each native worker it owns.
            libc = linux_libc()
            if libc.prctl(1, signal.SIGKILL, 0, 0, 0) != 0 or os.getppid() != parent_pid:
                os._exit(125)
            resource.setrlimit(resource.RLIMIT_AS, (address_space, address_space))
            resource.setrlimit(resource.RLIMIT_FSIZE, (output_cap, output_cap))
            resource.setrlimit(resource.RLIMIT_CPU, (cpu_seconds, cpu_seconds))
            resource.setrlimit(resource.RLIMIT_CORE, (0, 0))

        process = None
        selector = selectors.DefaultSelector()
        captures = []
        retained = 0
        try:
            env = os.environ.copy()
            env.update({"OMP_NUM_THREADS": "1", "RAYON_NUM_THREADS": "1"})
            process = subprocess.Popen(  # noqa: S603 - caller supplies explicit local tool.
                command,
                stdin=subprocess.DEVNULL,
                stdout=subprocess.PIPE,
                stderr=subprocess.PIPE,
                start_new_session=True,
                preexec_fn=child_setup,
                env=env,
                cwd=self.directory,
            )
            attempt["pid"] = process.pid
            self.persist()
            for label, pipe in (("stdout", process.stdout), ("stderr", process.stderr)):
                path = self.directory / f"{name}.{label}.log"
                handle = path.open("xb")
                captures.append(handle)
                os.set_blocking(pipe.fileno(), False)
                selector.register(pipe, selectors.EVENT_READ, handle)
            while selector.get_map() or process.poll() is None:
                self.check()
                for key, _ in selector.select(min(0.02, max(0, self.deadline - time.monotonic()))):
                    data = os.read(key.fd, CHUNK)
                    if not data:
                        selector.unregister(key.fileobj)
                        key.fileobj.close()
                        continue
                    remaining = output_cap - retained
                    key.data.write(data[:remaining])
                    retained += min(len(data), remaining)
                    if len(data) > remaining:
                        raise StopRun("output_limit", f"{name} diagnostic capture exceeded cap")
            return_code = process.wait()
            attempt.update(state="exited", returncode=return_code, captured_bytes=retained)
            return return_code
        except StopRun as error:
            attempt.update(state=error.state, detail=str(error), captured_bytes=retained)
            raise
        except (OSError, subprocess.SubprocessError) as error:
            attempt.update(state="process_error", detail=str(error))
            raise StopRun("process_error", f"{name}: {error}") from error
        finally:
            if process is not None:
                # Include descendants even when the immediate process already
                # exited; a leftover descendant must not keep pipes or work alive.
                with contextlib.suppress(ProcessLookupError):
                    os.killpg(process.pid, signal.SIGKILL)
                process.wait()
                # Real TPE workers may create separate process groups. This
                # single-job runner owns every child, so reap all adopted workers.
                until = time.monotonic() + 1
                while True:
                    try:
                        child, _ = os.waitpid(-1, os.WNOHANG)
                        if child == 0:
                            # Adopted descendants receive the native worker's
                            # PDEATHSIG; also stop any already-adopted live child.
                            for pid in owned_child_pids():
                                with contextlib.suppress(ProcessLookupError):
                                    os.kill(pid, signal.SIGKILL)
                            if time.monotonic() >= until:
                                attempt["cleanup_incomplete"] = True
                                break
                            time.sleep(0.005)
                    except ChildProcessError:
                        break
                for pipe in (process.stdout, process.stderr):
                    if pipe is not None:
                        pipe.close()
            selector.close()
            for handle in captures:
                handle.flush()
                os.fsync(handle.fileno())
                handle.close()
            self.persist()
            if attempt.get("cleanup_incomplete"):
                raise StopRun(
                    "cleanup_failed", f"{name}: owned children not reaped within one second"
                )

    def extraction(self, backend: str, name: str) -> str:
        self.verify_inputs()
        pass_directory = self.directory / f"{name}-pass"
        pass_directory.mkdir(mode=0o700)
        self.check()
        remaining = max(1, int((self.deadline - time.monotonic()) * 1000))
        command = [
            self.document["tools"]["tpe"]["path"],
            "extract",
            "--backend",
            backend,
            "--out",
            str(pass_directory),
            "--db",
            str(pass_directory / "ledger.sqlite"),
            "--jobs",
            "1",
            "--max-files",
            "1",
            "--max-bytes",
            str(self.document["artifacts"]["source"]["bytes"]),
            "--timeout-ms",
            str(remaining),
            "--max-memory-growth-mib",
            str(self.args.max_memory_growth_mib),
            "--max-output-bytes",
            str(self.args.max_output_bytes),
            str(self.directory / "source.pdf"),
        ]
        code = self.execute(name, command)
        self.verify_inputs()
        if code not in (0, 1):
            raise StopRun("child_failed", f"{name} exited {code}; no result accepted")
        source = self.document["artifacts"]["source"]
        output = pass_directory / (source["sha256"] + ".json")
        # Decode only the capped local file after the process has exited. Large
        # stdout is never requested, buffered, or loaded into parent memory.
        self.digest(output, self.args.max_output_bytes)
        with regular_reader(output) as handle:
            raw = handle.read(self.args.max_output_bytes + 1)
        if len(raw) > self.args.max_output_bytes:
            raise StopRun("output_limit", f"{name} JSON exceeds cap")
        result = json.loads(raw)
        if not isinstance(result, dict):
            raise StopRun("invalid_artifact", f"{name} result is not an object")
        status = result.get("status")
        if (
            result.get("document", {}).get("hash") != source["sha256"]
            or result.get("document", {}).get("size") != source["bytes"]
            or result.get("backend", {}).get("name") != backend
            or status not in ("complete", "partial")
            or code != (0 if status == "complete" else 1)
        ):
            raise StopRun("invalid_artifact", f"{name} source/backend/status/exit mismatch")
        expected = hashlib.sha256(raw).hexdigest()
        artifact = self.snapshot(output, name + ".json", self.args.max_output_bytes)
        if artifact["sha256"] != expected:
            raise StopRun("artifact_changed", f"{name} changed after validation")
        artifact["extraction_status"] = status
        self.document["artifacts"][name] = artifact
        self.document["attempts"][-1]["state"] = "completed"
        self.event(name + "_persisted", sha256=artifact["sha256"], extraction_status=status)
        return status

    def run(self) -> int:
        self.directory.mkdir(mode=0o700)  # Fail closed on every existing path/alias.
        self.directory = self.directory.resolve(strict=True)
        sync_directory(self.directory.parent)
        self.persist()
        try:
            self.document["artifacts"]["source"] = self.snapshot(
                self.args.source, "source.pdf", self.args.max_source_bytes
            )
            if self.document["artifacts"]["source"]["bytes"] == 0:
                raise StopRun("invalid_input", "source is empty")
            self.event("source_snapshotted")
            self.document["tools"] = {"tpe": self.tool(self.args.tpe)}
            baseline_status = self.extraction("lopdf", "baseline")
            self.document["baseline_status"] = baseline_status
            candidate_status = None
            # The baseline artifact and its journal receipt have both been
            # fsynced before even validating the optional candidate inputs.
            if self.args.pdfium:
                review = self.snapshot(self.args.review, "review.json", MAX_REVIEW)
                if review["sha256"] != self.args.trusted_review_sha256:
                    raise StopRun("review_rejected", "review differs from caller-approved digest")
                self.document["artifacts"]["review"] = review
                self.document["artifacts"]["render"] = self.snapshot(
                    self.args.render, "render-proof.bin", MAX_RENDER
                )
                self.document["trusted_by"] = self.args.trusted_by
                self.document["tools"]["fusion"] = self.tool(self.args.fusion)
                configured = os.environ.get("PDFIUM_DYNAMIC_LIB_PATH")
                if configured:
                    library = Path(configured)
                    if library.is_dir():
                        library = library / "libpdfium.so"
                    digest, size = self.digest(library, MAX_TOOL)
                    self.document["pdfium_library_configuration"] = {
                        "path": str(library.absolute()),
                        "sha256": digest,
                        "bytes": size,
                        "claim": "configured_path_only_not_proof_of_loaded_runtime",
                    }
                self.event("review_accepted_as_external_adjudication")
                candidate_status = self.extraction("pdfium", "candidate")
                self.document["candidate_status"] = candidate_status
                self.verify_inputs()
                pending = self.directory / "fusion-output.json"
                code = self.execute(
                    "fusion",
                    [
                        self.document["tools"]["fusion"]["path"],
                        "--source",
                        str(self.directory / "source.pdf"),
                        "--max-source-bytes",
                        str(self.document["artifacts"]["source"]["bytes"]),
                        "--baseline",
                        str(self.directory / "baseline.json"),
                        "--candidate",
                        str(self.directory / "candidate.json"),
                        "--review",
                        str(self.directory / "review.json"),
                        "--trusted-review-sha256",
                        self.args.trusted_review_sha256,
                        "--trusted-by",
                        self.args.trusted_by,
                        "--render",
                        str(self.directory / "render-proof.bin"),
                        "--output",
                        str(pending),
                    ],
                )
                self.verify_inputs()
                if code:
                    raise StopRun("child_failed", f"fusion exited {code}")
                pending_digest, _ = self.digest(pending, self.args.max_output_bytes)
                with regular_reader(pending) as handle:
                    derived_bytes = handle.read(self.args.max_output_bytes + 1)
                if len(derived_bytes) > self.args.max_output_bytes:
                    raise StopRun("output_limit", "fusion output exceeds cap")
                if hashlib.sha256(derived_bytes).hexdigest() != pending_digest:
                    raise StopRun("artifact_changed", "fusion output changed before validation")
                derived = json.loads(derived_bytes)
                outcome = derived.get("outcome", {}).get("state")
                if outcome != "evaluated":
                    raise StopRun("fusion_incomplete", f"fusion outcome is {outcome!r}")
                self.check()
                artifact = self.snapshot(pending, "fused.json", self.args.max_output_bytes)
                if artifact["sha256"] != pending_digest:
                    raise StopRun("artifact_changed", "fusion output changed after validation")
                self.check()
                self.document["artifacts"]["fused"] = artifact
                self.document["attempts"][-1]["state"] = "completed"
                self.event("derived_view_persisted", sha256=artifact["sha256"])
            self.verify_inputs()
            self.check()
            self.document.update(
                state="completed",
                baseline_status=baseline_status,
                exit_code=1 if "partial" in (baseline_status, candidate_status) else 0,
            )
            self.persist()
            # The completion checkpoint follows durable journal publication.
            # A signal received during that fsync still demotes the attempt.
            # After this checkpoint, the job has already completed.
            self.check()
            self.committed = True
            return self.document["exit_code"]
        except (StopRun, OSError, ValueError, TypeError, AttributeError, MemoryError) as error:
            state = error.state if isinstance(error, StopRun) else "failed"
            self.document.update(state=state, detail=str(error)[:2048])
            fused = self.directory / "fused.json"
            if fused.exists():
                diagnostic = self.directory / "uncommitted-fused.json"
                fused.rename(diagnostic)
                sync_directory(self.directory)
                artifact = self.document["artifacts"].pop("fused", {})
                artifact["path"] = diagnostic.name
                self.document["artifacts"]["uncommitted_fused"] = artifact
            if self.document["attempts"] and self.document["attempts"][-1]["state"] == "exited":
                self.document["attempts"][-1].update(state=state, detail=str(error)[:2048])
            self.document["exit_code"] = 128 + self.cancelled if self.cancelled else 1
            self.persist()
            return self.document["exit_code"]


def parse_arguments(argv: list[str] | None = None) -> argparse.Namespace:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--source", required=True, type=Path)
    parser.add_argument("--out", required=True, type=Path, help="new job directory; must not exist")
    parser.add_argument("--tpe", required=True, type=Path, help="explicit local TPE executable")
    parser.add_argument(
        "--fusion", required=True, type=Path, help="explicit local fusion executable"
    )
    parser.add_argument(
        "--pdfium", action="store_true", help="opt in to candidate and reviewed fusion"
    )
    parser.add_argument("--review", type=Path)
    parser.add_argument("--trusted-review-sha256")
    parser.add_argument("--trusted-by")
    parser.add_argument("--render", type=Path, help="externally adjudicated render proof")
    parser.add_argument("--timeout-ms", type=int, default=60000)
    parser.add_argument("--max-memory-growth-mib", type=int, default=1024)
    parser.add_argument("--max-output-bytes", type=int, default=64 * MIB)
    parser.add_argument(
        "--max-source-bytes", type=int, help="optional caller source cap; no default size ceiling"
    )
    args = parser.parse_args(argv)
    if sys.platform != "linux":
        parser.error("this process-containment pilot supports Linux only")
    for name, lower, upper in (
        ("timeout_ms", 1, 300000),
        ("max_memory_growth_mib", 32, 4096),
        ("max_output_bytes", 1024, 256 * MIB),
    ):
        if not lower <= getattr(args, name) <= upper:
            parser.error(f"--{name.replace('_', '-')} must be {lower}..{upper}")
    if args.pdfium and not all(
        (args.review, args.trusted_review_sha256, args.trusted_by, args.render)
    ):
        parser.error("--pdfium requires --review, --trusted-review-sha256, --trusted-by, --render")
    if args.trusted_review_sha256 and (
        len(args.trusted_review_sha256) != 64
        or any(character not in "0123456789abcdef" for character in args.trusted_review_sha256)
    ):
        parser.error("--trusted-review-sha256 must be a lowercase SHA-256 hex digest")
    if args.trusted_by and len(args.trusted_by) > 1024:
        parser.error("--trusted-by must be at most 1024 characters")
    if args.max_source_bytes is not None and args.max_source_bytes < 1:
        parser.error("--max-source-bytes must be positive")
    return args


def main(argv: list[str] | None = None) -> int:
    args = parse_arguments(argv)
    runner = Runner(args)
    parent_cap = (args.max_memory_growth_mib + 256) * MIB
    _, previous_hard = resource.getrlimit(resource.RLIMIT_AS)
    if previous_hard != resource.RLIM_INFINITY:
        parent_cap = min(parent_cap, previous_hard)
    resource.setrlimit(resource.RLIMIT_AS, (parent_cap, parent_cap))
    libc = linux_libc()
    if libc.prctl(36, 1, 0, 0, 0) != 0:  # PR_SET_CHILD_SUBREAPER
        print("unable to establish Linux child reaping", file=sys.stderr)
        return 1
    signal.signal(signal.SIGINT, runner.signal)
    signal.signal(signal.SIGTERM, runner.signal)
    try:
        result = runner.run()
    except OSError as error:
        print(f"job directory rejected: {error}", file=sys.stderr)
        return 1
    print(
        json.dumps(
            {"state": runner.document["state"], "journal": str(runner.directory / "journal.json")}
        )
    )
    return result


if __name__ == "__main__":
    raise SystemExit(main())
