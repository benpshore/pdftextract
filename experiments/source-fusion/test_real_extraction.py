"""Required fresh-engine qualification; evaluation truth never enters the selector."""

# ruff: noqa: S101 -- Assertions implement the pytest qualification checks.

from __future__ import annotations

import hashlib
import json
import os
import subprocess
import sys
from pathlib import Path

import pytest

HERE = Path(__file__).resolve().parent
FIXTURES = HERE / "fixtures"


def digest(path: Path) -> str:
    return hashlib.sha256(path.read_bytes()).hexdigest()


@pytest.fixture(scope="module")
def tools():
    paths = []
    for variable in ("SOURCE_TPE_BIN", "SOURCE_FUSION_BIN"):
        value = os.environ.get(variable)
        assert value, f"required actual qualification input missing: {variable}"
        path = Path(value).resolve(strict=True)
        assert path.is_file() and os.access(path, os.X_OK), variable
        paths.append(path)
    configured = os.environ.get("PDFIUM_DYNAMIC_LIB_PATH")
    assert configured, "the actual pinned PDFium runtime must be explicit"
    native = Path(configured)
    if native.is_dir():
        native /= "libpdfium.so"
    assert digest(native) == "7670b3c597b02dfa3f98b23b49c3bb52536312f1ea686b739321731b6011f5a9"
    return paths


@pytest.fixture(scope="module")
def evaluation(tools, tmp_path_factory):
    output = tmp_path_factory.mktemp("source-evaluation-parent") / "evaluation"
    truth = FIXTURES / "evaluation" / "truth"
    result = subprocess.run(  # noqa: S603 -- Explicit local evaluator and supplied test tools.
        [
            sys.executable,
            str(HERE / "evaluate.py"),
            "--fixtures",
            str(FIXTURES),
            "--truth-dir",
            str(truth),
            "--out",
            str(output),
            "--tpe",
            str(tools[0]),
            "--fusion",
            str(tools[1]),
        ],
        capture_output=True,
        timeout=180,
        check=False,
    )
    assert result.returncode == 0, result.stderr.decode()
    report = json.loads((output / "report.json").read_bytes())
    return output, report


def jobs(output):
    return sorted(output.glob("**/journal.json"))


def test_all_frozen_cases_remain_in_evaluation_and_neighbor_is_preserved(evaluation):
    output, report = evaluation
    frozen = json.loads((FIXTURES / "input-freeze.json").read_bytes())
    cases = report["cases"]
    assert {row["id"] for row in cases} == {row["id"] for row in frozen["cases"]}
    assert len(cases) == len(jobs(output)) == 6
    assert all(row["baseline_neighbor_preserved"] for row in cases)
    assert sum(row["selected_count"] for row in cases) >= 2
    assert all(row["selected_source_consistent"] for row in cases if row["selected_count"])
    assert all(row["selected_visible_correct"] for row in cases if row["selected_count"])
    assert any(row["source_outcome"] == "unsupported_source" for row in cases)


def test_selector_receives_only_source_and_actual_artifacts(evaluation):
    output, _ = evaluation
    for journal_file in jobs(output):
        journal = json.loads(journal_file.read_bytes())
        directory = journal_file.parent
        assert [attempt["name"] for attempt in journal["attempts"]] == [
            "baseline",
            "candidate",
            "fusion",
        ]
        arguments = journal["attempts"][-1]["argv"][1:]
        assert set(arguments[::2]) == {
            "--source",
            "--max-source-bytes",
            "--baseline",
            "--candidate",
            "--output",
        }
        assert not {"review", "render", "truth"}.intersection(journal["artifacts"])
        assert digest(directory / "baseline.json") == journal["artifacts"]["baseline"]["sha256"]
        assert digest(directory / "candidate.json") == journal["artifacts"]["candidate"]["sha256"]
        events = [entry["event"] for entry in journal["events"]]
        assert events.index("baseline_persisted") < events.index("candidate_persisted")
        if journal["state"] != "completed":
            assert "fused" not in journal["artifacts"]


def test_selected_spans_reference_computed_raw_code_and_font_evidence(evaluation):
    output, _ = evaluation
    selected = 0
    for journal_file in jobs(output):
        directory = journal_file.parent
        final = directory / "fused.json"
        if not final.exists():
            continue
        projection = json.loads(final.read_bytes())
        baseline = json.loads((directory / "baseline.json").read_bytes())
        candidate = json.loads((directory / "candidate.json").read_bytes())
        assert projection["policy"] == "source_declared_unicode_v1"
        assert "accepted_review" not in projection
        assert projection["source_evidence"]["source"]["sha256"] == digest(directory / "source.pdf")
        for page in projection["pages"]:
            for span in page["spans"]:
                if span["selected"] == span["baseline"]:
                    continue
                selected += 1
                locator = span["source_witness"]
                witness = projection["source_evidence"]["pages"][locator["source_page_index"]][
                    "witnesses"
                ][locator["witness_index"]]
                origin = witness["origin"]
                assert bytes.fromhex(origin["raw_codes_hex"])
                assert origin["character_codes"] == origin["glyph_ids"]
                assert origin["font_object"] and origin["to_unicode_object"]
                assert origin["font_program_sha256"] and origin["to_unicode_sha256"]
                assert span["text"] == witness["unicode"]
                candidate_locator = span["selected"]
                assert (
                    span["text"]
                    == candidate["pages"][candidate_locator["page_index"]]["spans"][
                        candidate_locator["span_index"]
                    ]["text"]
                )
                assert candidate_locator["artifact"]["sha256"] == digest(
                    directory / "candidate.json"
                )
        if baseline["status"] == "partial":
            assert projection["evidence"]["baseline"]["outcome"]["state"] == "partial"
            assert json.loads(journal_file.read_bytes())["exit_code"] == 1
    assert selected >= 2


@pytest.mark.parametrize("mutation", ["longer_wrong_text", "duplicate_geometry"])
def test_real_candidate_mutations_cannot_authorize_selection(tools, evaluation, tmp_path, mutation):
    output, _ = evaluation
    for journal_file in jobs(output):
        directory = journal_file.parent
        final = directory / "fused.json"
        if final.exists():
            projection = json.loads(final.read_bytes())
            selected = [
                span
                for page in projection["pages"]
                for span in page["spans"]
                if span["selected"] != span["baseline"]
            ]
            if selected:
                break
    else:
        pytest.fail("required actual selected case missing")
    target = selected[0]["selected"]
    candidate = json.loads((directory / "candidate.json").read_bytes())
    spans = candidate["pages"][target["page_index"]]["spans"]
    if mutation == "longer_wrong_text":
        spans[target["span_index"]]["text"] += " INVENTED LONGER TEXT"
    else:
        spans.append(dict(spans[target["span_index"]]))
    changed = tmp_path / "changed-candidate.json"
    changed.write_text(json.dumps(candidate))
    receipt = tmp_path / "projection.json"
    result = subprocess.run(  # noqa: S603 -- Explicit local CLI and generated adversarial files.
        [
            str(tools[1]),
            "--source",
            str(directory / "source.pdf"),
            "--baseline",
            str(directory / "baseline.json"),
            "--candidate",
            str(changed),
            "--output",
            str(receipt),
        ],
        capture_output=True,
        timeout=30,
        check=False,
    )
    assert result.returncode == 0, result.stderr.decode()
    projection = json.loads(receipt.read_bytes())
    baseline_locator = selected[0]["baseline"]
    row = projection["pages"][baseline_locator["page_index"]]["spans"][
        baseline_locator["span_index"]
    ]
    assert row["selected"] == row["baseline"]
