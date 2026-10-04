#!/usr/bin/env python3
"""Run frozen cases, then score source concordance and visible truth separately.

Truth paths never appear in runner or selector argv. All native/selector attempts
finish before the first truth file is opened. No renderer is needed for scoring.
"""

from __future__ import annotations

import argparse
import hashlib
import json
import os
import subprocess
import sys
from datetime import UTC, datetime
from pathlib import Path

ROOT = Path(__file__).resolve().parent


def identity(path: Path) -> dict:
    data = path.read_bytes()
    return {"sha256": hashlib.sha256(data).hexdigest(), "size": len(data)}


def load(path: Path) -> dict:
    return json.loads(path.read_text())


def write(path: Path, value: dict) -> None:
    path.write_text(json.dumps(value, indent=2, sort_keys=True) + "\n")


def timestamp() -> str:
    return datetime.now(UTC).isoformat()


def verify_policy_freeze(path: Path) -> dict:
    """Require the pre-scoring source fingerprint; never refresh it during evaluation."""
    frozen = load(path)
    project = ROOT.parents[1]
    if frozen.get("policy") != "source_declared_unicode_v1" or not frozen.get("files"):
        raise ValueError("missing source-selection policy freeze")
    for relative, expected in frozen["files"].items():
        actual = (project / relative).resolve(strict=True)
        if not actual.is_relative_to(project) or identity(actual) != expected:
            raise ValueError(f"frozen implementation changed: {relative}")
    return frozen


def run_cases(args: argparse.Namespace, freeze: dict) -> list[dict]:
    """Use only public sources and operator executables; never open scoring truth."""
    rows = []
    for case in freeze["cases"]:
        source = (args.fixtures / case["path"]).resolve(strict=True)
        if identity(source) != {key: case[key] for key in ("sha256", "size")}:
            raise ValueError(f"frozen input changed: {case['id']}")
        job = args.out / case["id"]
        argv = [
            sys.executable,
            str(ROOT / "run_extraction.py"),
            "--tpe",
            str(args.tpe),
            "--fusion",
            str(args.fusion),
            "--source",
            str(source),
            "--out",
            str(job),
            "--pdfium",
            "--timeout-ms",
            "60000",
        ]
        # Explicit operator-selected executables and hash-checked synthetic source; no shell.
        error = None
        try:
            result = subprocess.run(argv, capture_output=True, text=True, timeout=75)  # noqa: S603
            code, stdout, stderr = result.returncode, result.stdout, result.stderr
        except subprocess.TimeoutExpired as failure:
            # subprocess.run kills and waits for the runner. Its frozen lifecycle
            # helper establishes PDEATHSIG for its native child before exec.
            code, stdout, stderr = None, failure.stdout or b"", failure.stderr or b""
            error = {"kind": "runner_timeout", "detail": str(failure)}
        except OSError as failure:
            code, stdout, stderr = None, "", ""
            error = {"kind": "runner_start_failed", "detail": str(failure)}
        if isinstance(stdout, bytes):
            stdout = stdout.decode(errors="replace")
        if isinstance(stderr, bytes):
            stderr = stderr.decode(errors="replace")
        (args.out / f"{case['id']}.stdout.txt").write_text(stdout)
        (args.out / f"{case['id']}.stderr.txt").write_text(stderr)
        rows.append(
            {
                "id": case["id"],
                "source_sha256": case["sha256"],
                "runner_exit": code,
                "execution_error": error,
                "runner_argv": argv,
            }
        )
    return rows


def span_index(artifact: dict, run: dict) -> int | None:
    """Match the authored location, without consulting expected text."""
    found = []
    for index, span in enumerate(artifact["pages"][0]["spans"]):
        box = span.get("bbox")
        if box is None:
            continue
        center_y = (box["y0"] + box["y1"]) / 2
        if abs(box["x0"] - run["x"]) <= 5 and abs(center_y - run["y"]) <= run["size"]:
            found.append(index)
    return found[0] if len(found) == 1 else None


def projected(derived: dict, index: int | None) -> dict | None:
    if index is None:
        return None
    found = [
        span
        for page in derived.get("pages") or []
        for span in page["spans"]
        if span["baseline"]["page_index"] == 0 and span["baseline"]["span_index"] == index
    ]
    return found[0] if len(found) == 1 else None


def expected_action(truth: dict) -> str:
    target = truth["target"]
    if target["map"] != "complete":
        return "abstain_unsupported_source"
    return "retain" if target["encoding"] == "named" else "select"


def score_case(args: argparse.Namespace, row: dict, freeze: dict) -> dict:
    case_id = row["id"]
    truth_path = args.truth_dir / f"{case_id}.truth.json"
    committed = freeze["private_truth_commitments"][f"{case_id}.truth"]
    if identity(truth_path) != committed:
        raise ValueError(f"pre-extraction truth commitment changed: {case_id}")
    truth = load(truth_path)
    if truth["source"]["sha256"] != row["source_sha256"]:
        raise ValueError("truth refers to a different source")
    job = args.out / case_id
    journal = load(job / "journal.json") if (job / "journal.json").exists() else {}
    source_path = job / "source.pdf"
    if source_path.exists() and identity(source_path)["sha256"] != row["source_sha256"]:
        raise ValueError("captured source differs from frozen input")
    baseline_path = job / "baseline.json"
    candidate_path = job / "candidate.json"
    derived_path = job / "fusion-output.json"
    if baseline_path.exists():
        actual = identity(baseline_path)
        receipt = journal.get("artifacts", {}).get("baseline", {})
        if receipt.get("sha256") != actual["sha256"] or receipt.get("bytes") != actual["size"]:
            raise ValueError("baseline evidence differs from persisted receipt")
    baseline = load(baseline_path) if baseline_path.exists() else {"pages": [{"spans": []}]}
    candidate = load(candidate_path) if candidate_path.exists() else {"pages": [{"spans": []}]}
    derived = load(derived_path) if derived_path.exists() else {}
    available = committed_projection(job, journal, derived_path, derived, row)
    target_index = span_index(baseline, truth["target"])
    neighbor_index = span_index(baseline, truth["neighbor"])
    target = projected(derived, target_index) if available else None
    neighbor = projected(derived, neighbor_index) if available else None
    base_spans = baseline["pages"][0]["spans"]
    base_target = base_spans[target_index]["text"] if target_index is not None else None
    base_neighbor = base_spans[neighbor_index]["text"] if neighbor_index is not None else None
    target_text = target["text"] if target is not None else base_target
    selected = [
        span
        for page in derived.get("pages") or []
        for span in page["spans"]
        if available and span["selected"]["attempt_id"] == "candidate"
    ]
    selected_source = []
    selected_visible = []
    locator_integrity = True
    for span in selected:
        locator = span["selected"]
        original = candidate["pages"][locator["page_index"]]["spans"][locator["span_index"]]
        locator_integrity &= span["text"] == original["text"]
        locator_integrity &= locator["artifact"]["sha256"] == identity(candidate_path)["sha256"]
        base_index = span["baseline"]["span_index"]
        if base_index == target_index:
            selected_source.append(span["text"] == truth["mapped_semantic_truth"])
            selected_visible.append(span["text"] == truth["visible_target_truth"])
        elif base_index == neighbor_index:
            selected_source.append(span["text"] == truth["neighbor"]["text"])
            selected_visible.append(span["text"] == truth["neighbor"]["text"])
        else:
            selected_source.append(False)
            selected_visible.append(False)
    neighbor_preserved = base_neighbor == truth["neighbor"]["text"] and (
        neighbor is None
        or (neighbor["selected"] == neighbor["baseline"] and neighbor["text"] == base_neighbor)
    )
    return {
        **row,
        "journal_state": journal.get("state"),
        "source_outcome": derived.get("outcome", {}).get("status"),
        "projection_available": available,
        "selected_count": len(selected),
        "baseline_neighbor_preserved": neighbor_preserved,
        "selected_source_consistent": all(selected_source) if selected else None,
        "selected_visible_correct": all(selected_visible) if selected else None,
        "selected_locator_integrity": locator_integrity,
        "expected_action": expected_action(truth),
        "visible_target_correct": target_text == truth["visible_target_truth"],
        "semantic_target_correct": (
            target_text == truth["mapped_semantic_truth"]
            if truth["mapped_semantic_truth"] is not None
            else None
        ),
        "baseline_target_correct": base_target == truth["visible_target_truth"],
        "target_selected": target is not None and target["selected"]["attempt_id"] == "candidate",
        "target_text": target_text,
        "baseline_target_text": base_target,
        "visible_target_truth": truth["visible_target_truth"],
        "mapped_semantic_truth": truth["mapped_semantic_truth"],
        "scoring_error": None,
        "evaluation_class": truth["evaluation_class"],
        "regions": derived.get("regions", []),
        "artifacts": {
            name: identity(job / name)
            for name in (
                "source.pdf",
                "baseline.json",
                "candidate.json",
                "fusion-output.json",
                "fused.json",
                "journal.json",
            )
            if (job / name).exists()
        },
    }


def committed_projection(job: Path, journal: dict, path: Path, derived: dict, row: dict) -> bool:
    """Score only a committed receipt whose immutable inputs and output still match."""
    if (
        row.get("execution_error") is not None
        or journal.get("state") != "completed"
        or journal.get("exit_code") != row["runner_exit"]
        or derived.get("outcome", {}).get("status") != "evaluated"
    ):
        return False
    receipts = journal.get("artifacts", {})
    for key, filename in (
        ("source", "source.pdf"),
        ("baseline", "baseline.json"),
        ("candidate", "candidate.json"),
        ("fused", "fused.json"),
    ):
        artifact = job / filename
        receipt = receipts.get(key, {})
        if not artifact.is_file():
            return False
        actual = identity(artifact)
        if key == "source" and actual["sha256"] != row["source_sha256"]:
            return False
        if receipt.get("sha256") != actual["sha256"] or receipt.get("bytes") != actual["size"]:
            return False
    return identity(path) == identity(job / "fused.json")


def score_cases(args: argparse.Namespace, rows: list[dict], freeze: dict) -> list[dict]:
    """Retain every attempted case, including malformed scoring evidence."""
    cases = []
    for row in rows:
        try:
            cases.append(score_case(args, row, freeze))
        except (OSError, ValueError, TypeError, KeyError, IndexError, AttributeError) as error:
            cases.append(
                {
                    **row,
                    "scoring_error": {"kind": type(error).__name__, "detail": str(error)},
                    "projection_available": False,
                    "selected_count": 0,
                    "visible_target_correct": False,
                    "baseline_target_correct": False,
                    "baseline_neighbor_preserved": False,
                    "selected_visible_correct": None,
                    "selected_source_consistent": None,
                }
            )
    return cases


def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--fixtures", type=Path, default=ROOT / "fixtures")
    parser.add_argument("--truth-dir", type=Path, required=True)
    parser.add_argument("--policy-freeze", type=Path, default=ROOT / "fixtures/policy-freeze.json")
    parser.add_argument("--out", type=Path, required=True)
    parser.add_argument("--tpe", type=Path, default=os.environ.get("SOURCE_TPE_BIN"))
    parser.add_argument("--fusion", type=Path, default=os.environ.get("SOURCE_FUSION_BIN"))
    args = parser.parse_args(argv)
    if args.tpe is None or args.fusion is None:
        parser.error("provide --tpe/--fusion or SOURCE_TPE_BIN/SOURCE_FUSION_BIN")
    args.tpe = args.tpe.resolve(strict=True)
    args.fusion = args.fusion.resolve(strict=True)
    verify_policy_freeze(args.policy_freeze)
    args.out.mkdir(parents=True, exist_ok=False)
    args.out = args.out.resolve()
    freeze = load(args.fixtures / "input-freeze.json")
    phases = {"execution_started": timestamp(), "truth_access_during_execution": False}
    rows = run_cases(args, freeze)
    phases["all_attempts_finished_before_truth_read"] = timestamp()
    write(args.out / "execution-complete.json", {"phases": phases, "cases": rows})
    verify_policy_freeze(args.policy_freeze)
    phases["scoring_started"] = timestamp()
    cases = score_cases(args, rows, freeze)
    report = {
        "schema": "source-fusion-evaluation-v1",
        "phases": phases,
        "cases": cases,
        "source_freeze": identity(args.fixtures / "input-freeze.json"),
        "policy_freeze": identity(args.policy_freeze),
        "executables": {"tpe": identity(args.tpe), "fusion": identity(args.fusion)},
        "totals": {
            "attempted": len(cases),
            "execution_errors": sum(c["execution_error"] is not None for c in cases),
            "scoring_errors": sum(c["scoring_error"] is not None for c in cases),
            "projections_available": sum(c["projection_available"] for c in cases),
            "unavailable_projections": sum(not c["projection_available"] for c in cases),
            "selected_regions": sum(c["selected_count"] for c in cases),
            "visible_targets_correct": sum(c["visible_target_correct"] for c in cases),
            "baseline_targets_correct": sum(c["baseline_target_correct"] for c in cases),
            "neighbors_preserved": sum(c["baseline_neighbor_preserved"] for c in cases),
            "selected_cases_visible_wrong": sum(
                c["selected_visible_correct"] is False for c in cases
            ),
            "selected_cases_source_wrong": sum(
                c["selected_source_consistent"] is False for c in cases
            ),
        },
        "scope": "Six synthetic source-profile cases, not a population accuracy estimate.",
    }
    write(args.out / "report.json", report)
    print(json.dumps({"report": str(args.out / "report.json"), "totals": report["totals"]}))
    return int(report["totals"]["execution_errors"] > 0 or report["totals"]["scoring_errors"] > 0)


if __name__ == "__main__":
    raise SystemExit(main())
