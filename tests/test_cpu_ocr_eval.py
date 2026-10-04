"""Lightweight evidence-store regressions; no OCR corpus or model is run in CI."""

import importlib.util
import json
import os
import sqlite3
import subprocess
import sys
from pathlib import Path

import pytest

SCRIPT = Path(__file__).resolve().parents[1] / "scripts" / "cpu_ocr_eval.py"
SPEC = importlib.util.spec_from_file_location("cpu_ocr_eval", SCRIPT)
evaluation = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(evaluation)


def prepared_db(tmp_path, pages=1, expected="success"):
    db = evaluation.create_database(tmp_path / "evidence.sqlite")
    fixture = {
        "name": "unit_fixture",
        "kind": "pdf",
        "content": b"%PDF-synthetic-unit-test",
        "sizes": [(100, 100)] * pages,
        "truth": ["one two three"] * pages,
        "generation": {"unit_test": True},
        "expected_outcome": expected,
    }
    fixture_id = evaluation.register_fixture(db, fixture)
    environment_id = evaluation.insert(
        db,
        "environments",
        captured_at="2026-10-04T00:00:00Z",
        executor="unit test",
        snapshot_json="{}",
    )
    run_id = evaluation.insert(
        db,
        "runs",
        environment_id=environment_id,
        phase="cold",
        repetition=0,
        phase_definition=evaluation.PHASE_DEFINITION,
        started_at="2026-10-04T00:00:00Z",
        status="running",
        command_json="[]",
        config_json="{}",
    )
    return db, run_id, fixture_id, fixture


def result(events, exit_code=0, stderr=b""):
    return {
        "stdout": ("\n".join(json.dumps(event) for event in events) + "\n").encode(),
        "stderr": stderr,
        "started_at": "2026-10-04T00:00:00Z",
        "elapsed_seconds": 0.5,
        "exit_code": exit_code,
        "command": ["unit-test-only"],
        "interruption": None,
        "resources": {"process_max_rss_kib": 1234},
    }


def page_event(tmp_path, text="one two", page=1):
    tsv = tmp_path / f"{page}.tsv"
    tsv.write_bytes(b"raw TSV bytes\n")
    return {
        "event": "page",
        "page": page,
        "status": "partial",
        "text": text,
        "raster": {"width": 100, "height": 100, "pdf_transform": None},
        "timing_seconds": {"ocr": 0.1, "total": 0.2},
        "artifacts": {"tsv": {"path": str(tsv), "sha256": evaluation.digest(tsv.read_bytes())}},
        "words": [
            {
                "text": "one",
                "confidence": 91.5,
                "bbox": [1, 2, 3, 4],
                "block": 1,
                "paragraph": 1,
                "line": 1,
            }
        ],
        "warnings": ["ocr_is_inferred_text_not_verified_complete"],
        "resources": {
            "child_max_rss_kib": 1000,
            "worker_max_rss_kib": 500,
            "rss_scope": "separate_process_maxima_not_simultaneous_tree_peak",
        },
    }


def test_store_preserves_original_raw_outputs_regions_metrics_and_foreign_keys(tmp_path):
    db, run_id, fixture_id, fixture = prepared_db(tmp_path)
    events = [
        {"event": "start", "source": {"sha256": evaluation.digest(fixture["content"])}},
        page_event(tmp_path),
        {"event": "summary", "outcome": "success"},
    ]
    evaluation.record_attempt(db, run_id, fixture_id, fixture, result(events))
    row = db.execute(
        "SELECT a.content,a.sha256 FROM sources s JOIN artifacts a ON s.original_artifact_id=a.id"
    ).fetchone()
    assert row["content"] == fixture["content"]
    assert row["sha256"] == evaluation.digest(fixture["content"])
    assert db.execute("SELECT raw_text FROM page_outputs").fetchone()[0] == "one two"
    assert (
        db.execute(
            "SELECT a.content FROM page_outputs p JOIN artifacts a ON p.tsv_artifact_id=a.id"
        ).fetchone()[0]
        == b"raw TSV bytes\n"
    )
    assert tuple(db.execute("SELECT x,y,width,height FROM ocr_regions").fetchone()) == (1, 2, 3, 4)
    score = db.execute("SELECT * FROM page_scores").fetchone()
    assert score["omitted_words"] == 1
    assert score["wer"] == pytest.approx(1 / 3)
    assert db.execute("SELECT count(*) FROM resource_measurements").fetchone()[0] == 3
    assert db.execute("PRAGMA integrity_check").fetchone()[0] == "ok"
    assert not db.execute("PRAGMA foreign_key_check").fetchall()
    with pytest.raises(sqlite3.IntegrityError):
        db.execute("INSERT INTO run_components VALUES (999, 999)")


@pytest.mark.parametrize("output", ["", "one two"])
def test_accuracy_counts_omissions_and_normalizes_without_hiding_words(output):
    scored = evaluation.score("One two three", output)
    assert scored["omitted_words"] == (3 if not output else 1)
    assert scored["wer"] == pytest.approx(1 if not output else 1 / 3)
    assert evaluation.score("\uff21lpha\n BETA", "alpha beta")["cer"] == 0
    assert evaluation.score("one one two", "one two")["omitted_words"] == 1


def test_failed_pages_and_truncated_jsonl_remain_in_denominator(tmp_path):
    db, run_id, fixture_id, fixture = prepared_db(tmp_path, pages=2)
    attempt = result([{"event": "page_error", "page": 1, "error": "timeout"}], exit_code=124)
    attempt["stdout"] += b'{"event":"page",'
    evaluation.record_attempt(db, run_id, fixture_id, fixture, attempt)
    scores = db.execute("SELECT * FROM page_scores").fetchall()
    assert len(scores) == 2
    assert all(row["wer"] == 1 and row["cer"] == 1 for row in scores)
    assert {row[0] for row in db.execute("SELECT category FROM errors")} >= {
        "invalid_jsonl",
        "page_error",
        "missing_page",
        "no_summary",
    }
    assert db.execute("SELECT expected_outcome_met FROM attempts").fetchone()[0] == 0
    stored = db.execute(
        "SELECT content FROM artifacts WHERE id=(SELECT stdout_artifact_id FROM attempts)"
    ).fetchone()[0]
    assert stored == attempt["stdout"]


@pytest.mark.parametrize("failure", ["missing", "hash", "malformed_region", "duplicate"])
def test_artifact_ingestion_errors_keep_attempt_and_raw_evidence(tmp_path, failure):
    db, run_id, fixture_id, fixture = prepared_db(tmp_path)
    event = page_event(tmp_path)
    if failure == "missing":
        Path(event["artifacts"]["tsv"]["path"]).unlink()
    elif failure == "hash":
        event["artifacts"]["tsv"]["sha256"] = "0" * 64
    elif failure == "malformed_region":
        event["words"][0]["bbox"] = [1]
    events = [event] + ([event] if failure == "duplicate" else [])
    events.append({"event": "summary", "outcome": "success"})
    evaluation.record_attempt(db, run_id, fixture_id, fixture, result(events))
    assert db.execute("SELECT count(*) FROM attempts").fetchone()[0] == 1
    assert db.execute("SELECT expected_outcome_met FROM attempts").fetchone()[0] == 0
    assert db.execute("SELECT count(*) FROM events").fetchone()[0] == len(events)
    assert db.execute("SELECT count(*) FROM errors").fetchone()[0] >= 1


def test_malformed_input_has_separate_expected_failure_denominator(tmp_path):
    db, run_id, fixture_id, fixture = prepared_db(tmp_path, pages=0, expected="failed")
    evaluation.record_attempt(
        db,
        run_id,
        fixture_id,
        fixture,
        result([{"event": "error", "error": "invalid_pdf"}], exit_code=1),
    )
    assert db.execute("SELECT expected_outcome_met FROM attempts").fetchone()[0] == 1
    assert db.execute("SELECT count(*) FROM page_scores").fetchone()[0] == 0
    assert db.execute("SELECT count(*) FROM errors").fetchone()[0] > 0


def test_existing_database_and_summary_are_never_overwritten(tmp_path, monkeypatch):
    path = tmp_path / "original.sqlite"
    path.write_bytes(b"original evidence")
    with pytest.raises(FileExistsError):
        evaluation.create_database(path)
    monkeypatch.setattr(
        sys,
        "argv",
        [
            str(SCRIPT),
            "--database",
            str(path),
            "--summary",
            str(path),
            "--tessdata-dir",
            str(tmp_path),
            "--pdftoppm",
            "/unused",
            "--pdfinfo",
            "/unused",
        ],
    )
    with pytest.raises(SystemExit) as exc:
        evaluation.main()
    assert exc.value.code == 2
    assert path.read_bytes() == b"original evidence"
    monkeypatch.setattr(
        sys,
        "argv",
        [
            str(SCRIPT),
            "--database",
            str(tmp_path / "new.sqlite"),
            "--summary",
            str(path),
            "--tessdata-dir",
            str(tmp_path),
            "--pdftoppm",
            "/unused",
            "--pdfinfo",
            "/unused",
        ],
    )
    with pytest.raises(FileExistsError):
        evaluation.main()
    assert path.read_bytes() == b"original evidence"
    assert not (tmp_path / "new.sqlite").exists()


def test_explicit_fixture_font_preserves_bytes_and_uses_private_configuration(
    tmp_path, monkeypatch
):
    monkeypatch.setenv("FONTCONFIG_FILE", "/original/user/config-is-not-modified")
    source = tmp_path / "original-font.otf"
    source.write_bytes(b"unit-test font identity")
    work = tmp_path / "run-with-font"
    work.mkdir()
    copied = evaluation.prepare_fixture_font(work, source)
    assert copied.read_bytes() == source.read_bytes()
    config = Path(os.environ["FONTCONFIG_FILE"])
    assert config.parent == work
    assert str(copied.parent) in config.read_text()
    assert source.read_bytes() == b"unit-test font identity"


def test_public_manifest_verifies_hash_and_keeps_nullable_doi_and_upstream_provenance(tmp_path):
    image = tmp_path / "public.png"
    image.write_bytes(b"benign stand-in; this test does not OCR")
    manifest = {
        "dataset": {
            "repository_url": "https://example.org/official-corpus",
            "commit": "a" * 40,
            "repository_checksum": "b" * 64,
            "license": "CC0-1.0",
            "license_url": "https://example.org/official-corpus/LICENSE",
            "retrieved_at": "2026-10-04T00:00:00Z",
            "doi": None,
        },
        "fixtures": [
            {
                "name": "public",
                "path": "public.png",
                "sha256": evaluation.digest(image.read_bytes()),
                "ground_truth": [
                    {"text": "one two", "width_pixels": 100, "height_pixels": 50, "dpi": None}
                ],
                "source_uri": "https://example.org/official-corpus/public.png",
                "fixture_identifier": "public.png",
            }
        ],
    }
    path = tmp_path / "manifest.json"
    path.write_text(json.dumps(manifest))
    fixture = evaluation.load_public_fixtures(path)[0]
    db = evaluation.create_database(tmp_path / "public.sqlite")
    evaluation.register_fixture(db, fixture)
    source = db.execute("SELECT * FROM sources").fetchone()
    assert source["doi"] is None
    assert source["repository_commit"] == "a" * 40
    assert db.execute("SELECT dpi FROM fixture_pages").fetchone()[0] is None
    assert db.execute("SELECT count(*) FROM source_assets").fetchone()[0] == 1
    image.write_bytes(b"changed")
    with pytest.raises(ValueError, match="checksum mismatch"):
        evaluation.load_public_fixtures(path)


@pytest.mark.skipif(not hasattr(os, "wait4"), reason="Linux/POSIX evaluation supervisor")
def test_process_measurement_and_timeout_are_real_without_ocr(tmp_path):
    measured = evaluation.run_process(
        [sys.executable, "-c", "print('measured')"], tmp_path / "success", timeout=5
    )
    assert measured["stdout"] == b"measured\n"
    assert measured["exit_code"] == 0
    assert measured["resources"]["process_max_rss_kib"] > 0
    timed = evaluation.run_process(
        [sys.executable, "-c", "import time; time.sleep(30)"], tmp_path / "timeout", timeout=0.05
    )
    assert timed["interruption"] == "timeout"
    assert timed["elapsed_seconds"] < 3
    assert timed["exit_code"] != 0


@pytest.mark.skipif(not hasattr(os, "wait4"), reason="Linux/POSIX evaluation supervisor")
def test_sigterm_cancels_supervised_process_and_returns_evidence(tmp_path):
    # Use an isolated evaluator process; sending a signal must never interrupt pytest itself.
    code = (
        "import importlib.util,json,sys; from pathlib import Path; "
        f"s=importlib.util.spec_from_file_location('e', {str(SCRIPT)!r}); "
        "m=importlib.util.module_from_spec(s); s.loader.exec_module(m); "
        "r=m.run_process([sys.executable,'-c',"
        "'import os,signal,time; time.sleep(0.1); "
        "os.kill(os.getppid(),signal.SIGTERM); time.sleep(30)'],"
        f"Path({str(tmp_path / 'cancel')!r}),10); print(r['interruption'])"
    )
    completed = subprocess.run(  # noqa: S603
        [sys.executable, "-c", code],
        capture_output=True,
        text=True,
        timeout=5,
        check=True,
    )
    assert completed.stdout.strip() == "cancelled"
