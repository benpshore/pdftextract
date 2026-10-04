#!/usr/bin/env python3
"""Linux pilot: lopdf first, then optional source-based PDFium region fusion.

The frozen sibling region-fusion runner supplies process containment, snapshots,
artifact validation and durable journaling. This adapter replaces orchestration
and accepts no review, transcript, truth or render inputs. It never calls the
sibling's reviewed-selection orchestration. Python 3.14, standard library only.
"""

from __future__ import annotations

import argparse
import hashlib
import importlib.util
import json
import os
import resource
import signal
import sys
from pathlib import Path

_HELPERS = Path(__file__).resolve().parents[1] / "region-fusion" / "run_extraction.py"
_SPEC = importlib.util.spec_from_file_location("_frozen_region_fusion_runner", _HELPERS)
if _SPEC is None or _SPEC.loader is None:
    raise RuntimeError(f"missing frozen runner dependency: {_HELPERS}")
helpers = importlib.util.module_from_spec(_SPEC)
_SPEC.loader.exec_module(helpers)
StopRun = helpers.StopRun


class SourceRunner(helpers.Runner):
    """Reuse the frozen lifecycle helpers without invoking reviewed selection."""

    def __init__(self, args: argparse.Namespace):
        super().__init__(args)
        self.document["selection_lane"] = "automatic_source_concordance"
        self.document["semantic_verification"] = "source_verifier_scope_only"
        self.document["runner_dependency"] = {
            "path": str(_HELPERS),
            "sha256": hashlib.sha256(_HELPERS.read_bytes()).hexdigest(),
        }

    def candidate(self) -> str:
        self.document["tools"]["fusion"] = self.tool(self.args.fusion)
        configured = os.environ.get("PDFIUM_DYNAMIC_LIB_PATH")
        if configured:
            library = Path(configured)
            if library.is_dir():
                library = library / "libpdfium.so"
            digest, size = self.digest(library, helpers.MAX_TOOL)
            self.document["pdfium_library_configuration"] = {
                "path": str(library.absolute()),
                "sha256": digest,
                "bytes": size,
                "claim": "configured_path_only_not_proof_of_loaded_runtime",
            }
        status = self.extraction("pdfium", "candidate")
        self.document["candidate_status"] = status
        return status

    def fuse(self) -> None:
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
                "--output",
                str(pending),
            ],
        )
        self.verify_inputs()
        if code:
            raise StopRun("child_failed", f"source fusion exited {code}")
        pending_digest, _ = self.digest(pending, self.args.max_output_bytes)
        with helpers.regular_reader(pending) as handle:
            derived_bytes = handle.read(self.args.max_output_bytes + 1)
        if len(derived_bytes) > self.args.max_output_bytes:
            raise StopRun("output_limit", "fusion output exceeds cap")
        if hashlib.sha256(derived_bytes).hexdigest() != pending_digest:
            raise StopRun("artifact_changed", "fusion output changed before validation")
        derived = json.loads(derived_bytes)
        outcome = derived.get("outcome", {}).get("status")
        if outcome != "evaluated":
            raise StopRun("fusion_incomplete", f"source fusion outcome is {outcome!r}")
        self.check()
        artifact = self.snapshot(pending, "fused.json", self.args.max_output_bytes)
        if artifact["sha256"] != pending_digest:
            raise StopRun("artifact_changed", "fusion output changed after validation")
        self.check()
        self.document["artifacts"]["fused"] = artifact
        self.document["attempts"][-1]["state"] = "completed"
        self.event("source_derived_view_persisted", sha256=artifact["sha256"])

    def failed(self, error: Exception) -> int:
        state = error.state if isinstance(error, StopRun) else "failed"
        self.document.update(state=state, detail=str(error)[:2048])
        fused = self.directory / "fused.json"
        if fused.exists():
            diagnostic = self.directory / "uncommitted-fused.json"
            fused.rename(diagnostic)
            helpers.sync_directory(self.directory)
            artifact = self.document["artifacts"].pop("fused", {})
            artifact["path"] = diagnostic.name
            self.document["artifacts"]["uncommitted_fused"] = artifact
        if self.document["attempts"] and self.document["attempts"][-1]["state"] == "exited":
            self.document["attempts"][-1].update(state=state, detail=str(error)[:2048])
        self.document["exit_code"] = 128 + self.cancelled if self.cancelled else 1
        self.persist()
        return self.document["exit_code"]

    def run(self) -> int:
        self.directory.mkdir(mode=0o700)
        self.directory = self.directory.resolve(strict=True)
        helpers.sync_directory(self.directory.parent)
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
            # extraction() has fsynced baseline.json and its journal receipt
            # before any optional tool, native library or candidate work starts.
            if self.args.pdfium:
                candidate_status = self.candidate()
                self.fuse()
            self.verify_inputs()
            self.check()
            self.document.update(
                state="completed",
                exit_code=1 if "partial" in (baseline_status, candidate_status) else 0,
            )
            self.persist()
            # Cancellation delivered during final persistence still wins.
            self.check()
            self.committed = True
            return self.document["exit_code"]
        except (StopRun, OSError, ValueError, TypeError, AttributeError, MemoryError) as error:
            return self.failed(error)


def parse_arguments(argv: list[str] | None = None) -> argparse.Namespace:
    parser = argparse.ArgumentParser(description=__doc__, allow_abbrev=False)
    parser.add_argument("--source", required=True, type=Path)
    parser.add_argument("--out", required=True, type=Path, help="new job directory; must not exist")
    parser.add_argument("--tpe", required=True, type=Path)
    parser.add_argument("--fusion", required=True, type=Path, help="local fuse_source executable")
    parser.add_argument("--pdfium", action="store_true", help="opt in to source-based fusion")
    parser.add_argument("--timeout-ms", type=int, default=60000)
    parser.add_argument("--max-memory-growth-mib", type=int, default=1024)
    parser.add_argument("--max-output-bytes", type=int, default=64 * helpers.MIB)
    parser.add_argument("--max-source-bytes", type=int, help="optional caller source cap")
    args = parser.parse_args(argv)
    if sys.platform != "linux":
        parser.error("this process-containment pilot supports Linux only")
    for name, lower, upper in (
        ("timeout_ms", 1, 300000),
        ("max_memory_growth_mib", 32, 4096),
        ("max_output_bytes", 1024, 256 * helpers.MIB),
    ):
        if not lower <= getattr(args, name) <= upper:
            parser.error(f"--{name.replace('_', '-')} must be {lower}..{upper}")
    if args.max_source_bytes is not None and args.max_source_bytes < 1:
        parser.error("--max-source-bytes must be positive")
    return args


def main(argv: list[str] | None = None) -> int:
    args = parse_arguments(argv)
    runner = SourceRunner(args)
    cap = (args.max_memory_growth_mib + 256) * helpers.MIB
    _, hard = resource.getrlimit(resource.RLIMIT_AS)
    if hard != resource.RLIM_INFINITY:
        cap = min(cap, hard)
    resource.setrlimit(resource.RLIMIT_AS, (cap, cap))
    if helpers.linux_libc().prctl(36, 1, 0, 0, 0) != 0:
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
