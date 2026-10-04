"""Truth-free regressions for cohort retention and committed publication evidence."""

# ruff: noqa: S101 -- Assertions implement pytest qualification checks.

import argparse
import importlib.util
import json
import subprocess
from pathlib import Path

import pytest

SPEC = importlib.util.spec_from_file_location(
    "source_evaluator", Path(__file__).with_name("evaluate.py")
)
evaluator = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(evaluator)


@pytest.mark.parametrize("failure", ["timeout", "startup"])
def test_failed_runner_keeps_every_attempt_and_retained_streams(tmp_path, monkeypatch, failure):
    fixtures = tmp_path / "fixtures"
    fixtures.mkdir()
    out = tmp_path / "out"
    out.mkdir()
    cases = []
    for index in range(6):
        source = fixtures / f"case-{index}.pdf"
        source.write_bytes(b"authored process-test placeholder")
        cases.append({"id": f"case-{index}", "path": source.name, **evaluator.identity(source)})
    args = argparse.Namespace(fixtures=fixtures, out=out, tpe="unused", fusion="unused")
    calls = []

    def run(argv, **kwargs):
        calls.append(argv)
        if len(calls) == 1:
            if failure == "timeout":
                raise subprocess.TimeoutExpired(
                    argv, 75, output=b"retained prefix", stderr=b"error"
                )
            raise OSError("authored startup failure")
        return subprocess.CompletedProcess(argv, 1, "completed Partial", "")

    monkeypatch.setattr(evaluator.subprocess, "run", run)
    rows = evaluator.run_cases(args, {"cases": cases})
    assert len(calls) == len(rows) == 6
    assert rows[0]["runner_exit"] is None
    assert rows[0]["execution_error"]["kind"] == (
        "runner_timeout" if failure == "timeout" else "runner_start_failed"
    )
    assert all(row["execution_error"] is None for row in rows[1:])
    if failure == "timeout":
        assert (out / "case-0.stdout.txt").read_text() == "retained prefix"
        assert (out / "case-0.stderr.txt").read_text() == "error"


@pytest.mark.parametrize("failure", [ValueError, KeyError, IndexError, OSError, TypeError])
def test_invalid_case_evidence_keeps_the_rest_of_scoring_cohort(monkeypatch, failure):
    rows = [{"id": str(i), "execution_error": None} for i in range(6)]

    def score(args, row, freeze):
        if row["id"] == "0":
            raise failure("authored malformed evidence")
        return {**row, "scoring_error": None}

    monkeypatch.setattr(evaluator, "score_case", score)
    cases = evaluator.score_cases(None, rows, {})
    assert len(cases) == 6
    assert cases[0]["scoring_error"]["kind"] == failure.__name__
    assert not cases[0]["projection_available"]
    assert all(case["scoring_error"] is None for case in cases[1:])


@pytest.mark.parametrize(
    "mutation",
    [
        "none",
        "failed_journal",
        "missing_receipt",
        "input_changed",
        "output_changed",
        "timeout",
        "source_and_receipt_changed",
    ],
)
def test_only_matching_committed_projection_can_be_scored(tmp_path, mutation):
    derived = {"outcome": {"status": "evaluated"}}
    encoded = json.dumps(derived).encode()
    (tmp_path / "fusion-output.json").write_bytes(encoded)
    journal = {"state": "completed", "exit_code": 1, "artifacts": {}}
    row = {"runner_exit": 1, "execution_error": None}
    for key, name in (
        ("source", "source.pdf"),
        ("baseline", "baseline.json"),
        ("candidate", "candidate.json"),
        ("fused", "fused.json"),
    ):
        path = tmp_path / name
        path.write_bytes(encoded)
        actual = evaluator.identity(path)
        journal["artifacts"][key] = {"sha256": actual["sha256"], "bytes": actual["size"]}
    row["source_sha256"] = evaluator.identity(tmp_path / "source.pdf")["sha256"]
    if mutation == "failed_journal":
        journal["state"] = "failed"
    elif mutation == "missing_receipt":
        journal["artifacts"].pop("fused")
    elif mutation == "input_changed":
        (tmp_path / "candidate.json").write_bytes(b"changed")
    elif mutation == "output_changed":
        (tmp_path / "fusion-output.json").write_bytes(b"other diagnostic")
    elif mutation == "timeout":
        row["execution_error"] = {"kind": "runner_timeout"}
    elif mutation == "source_and_receipt_changed":
        (tmp_path / "source.pdf").write_bytes(b"changed source")
        actual = evaluator.identity(tmp_path / "source.pdf")
        journal["artifacts"]["source"] = {"sha256": actual["sha256"], "bytes": actual["size"]}
    available = evaluator.committed_projection(
        tmp_path, journal, tmp_path / "fusion-output.json", derived, row
    )
    assert available == (mutation == "none")


def test_policy_freeze_rejects_implementation_changes_before_truth_access(tmp_path, monkeypatch):
    project = tmp_path / "project"
    root = project / "experiments/source-fusion"
    root.mkdir(parents=True)
    source = root / "policy.rs"
    source.write_bytes(b"authored frozen policy")
    policy = root / "freeze.json"
    policy.write_text(
        json.dumps(
            {
                "policy": "source_declared_unicode_v1",
                "files": {"experiments/source-fusion/policy.rs": evaluator.identity(source)},
            }
        )
    )
    monkeypatch.setattr(evaluator, "ROOT", root)
    assert evaluator.verify_policy_freeze(policy)["policy"] == "source_declared_unicode_v1"
    source.write_bytes(b"changed policy")
    with pytest.raises(ValueError, match="frozen implementation changed"):
        evaluator.verify_policy_freeze(policy)
