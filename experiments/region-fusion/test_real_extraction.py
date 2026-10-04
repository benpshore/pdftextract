"""Required real-engine pilot checks; missing executables/runtime fail this suite.

The ordinary repository suite does not collect this directory. Invoke explicitly
with REGION_TPE_BIN, REGION_FUSION_BIN and PDFIUM_DYNAMIC_LIB_PATH provisioned.
Review-assisted selection is tested; this is not blind accuracy measurement.
"""

# ruff: noqa: S101 -- Assertions are the checks in this pytest qualification suite.

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
SOURCE = FIXTURES / "positive-stream-cmap.pdf"
SOURCE_SHA = "97779d74bc9f286de6f4578ce86f47a266d7f263a28ce37b955b61b7333f2a1f"
REVIEW = FIXTURES / "approved-review.json"
REVIEW_SHA = "926b144c886d90734390df8e951026cc0115610fdb68fb28660ddea7cd48889b"
RENDER = FIXTURES / "captured/positive-stream-cmap/page-1-mupdf.png"
GOOD = "BASELINE REGION RETAINS THIS TEXT."
RECOVERED = "RECOVER ALPHA 2026"


def digest(path: Path) -> str:
    return hashlib.sha256(path.read_bytes()).hexdigest()


@pytest.fixture(scope="module")
def tools() -> tuple[Path, Path]:
    paths = []
    for name in ("REGION_TPE_BIN", "REGION_FUSION_BIN"):
        value = os.environ.get(name)
        assert value, f"required real-engine qualification input missing: {name}"
        path = Path(value).resolve(strict=True)
        assert path.is_file() and os.access(path, os.X_OK), name
        paths.append(path)
    native = os.environ.get("PDFIUM_DYNAMIC_LIB_PATH")
    assert native, "real PDFium runtime must be explicitly configured"
    runtime = Path(native)
    if runtime.is_dir():
        runtime /= "libpdfium.so"
    assert digest(runtime) == "7670b3c597b02dfa3f98b23b49c3bb52536312f1ea686b739321731b6011f5a9"
    assert digest(SOURCE) == SOURCE_SHA
    assert digest(REVIEW) == REVIEW_SHA  # Previously approved constant, not self-authorization.
    return paths[0], paths[1]


def run_job(tools, output: Path, *, pdfium=True, trusted_digest=REVIEW_SHA, tpe=None):
    command = [
        sys.executable,
        str(HERE / "run_extraction.py"),
        "--source",
        str(SOURCE),
        "--out",
        str(output),
        "--tpe",
        str(tpe or tools[0]),
        "--fusion",
        str(tools[1]),
        "--timeout-ms",
        "30000",
    ]
    if pdfium:
        command += [
            "--pdfium",
            "--review",
            str(REVIEW),
            "--trusted-review-sha256",
            trusted_digest,
            "--trusted-by",
            "pinned-source-and-render-fixture-review",
            "--render",
            str(RENDER),
        ]
    result = subprocess.run(command, capture_output=True, timeout=40, check=False)  # noqa: S603
    journal = json.loads((output / "journal.json").read_text())
    return result, journal


@pytest.fixture(scope="module")
def completed_job(tools, tmp_path_factory):
    output = tmp_path_factory.mktemp("real-region-parent") / "job"
    result, journal = run_job(tools, output)
    assert result.returncode == 1, result.stderr.decode()  # Baseline remains Partial.
    assert journal["state"] == "completed", journal
    return output, journal


def test_actual_lopdf_first_fuses_only_the_reviewed_region(completed_job):
    output, journal = completed_job
    baseline_bytes = (output / "baseline.json").read_bytes()
    baseline = json.loads(baseline_bytes)
    candidate = json.loads((output / "candidate.json").read_bytes())
    fused = json.loads((output / "fused.json").read_bytes())
    assert baseline["backend"]["name"] == "lopdf"
    assert candidate["backend"]["name"] == "pdfium"
    assert baseline["status"] == journal["baseline_status"] == "partial"
    assert baseline["pages"][0]["spans"][0]["text"] == GOOD
    assert baseline["pages"][0]["spans"][1]["text"] == "\ufffd" * 18
    assert candidate["pages"][0]["spans"][1]["text"] == RECOVERED
    assert fused["outcome"]["state"] == "evaluated"
    spans = fused["pages"][0]["spans"]
    assert [span["text"] for span in spans] == [GOOD, RECOVERED]
    assert [span["selected"]["attempt_id"] for span in spans] == ["baseline", "candidate"]
    assert spans[0]["selected"] == spans[0]["baseline"]
    assert fused["evidence"]["baseline"]["outcome"]["state"] == "partial"
    assert baseline["warnings"]  # Recovery must not erase the original gap evidence.
    actual_hash = hashlib.sha256(baseline_bytes).hexdigest()
    assert actual_hash == journal["artifacts"]["baseline"]["sha256"]
    assert spans[1]["baseline"]["artifact"]["sha256"] == actual_hash
    assert spans[1]["selected"]["artifact"]["sha256"] == digest(output / "candidate.json")
    events = [event["event"] for event in journal["events"]]
    assert events.index("baseline_persisted") < events.index("candidate_persisted")
    assert [attempt["name"] for attempt in journal["attempts"]] == [
        "baseline",
        "candidate",
        "fusion",
    ]
    assert digest(SOURCE) == digest(output / "source.pdf") == SOURCE_SHA


def test_actual_partial_baseline_is_useful_without_candidate_opt_in(tools, tmp_path):
    result, journal = run_job(tools, tmp_path / "baseline-only", pdfium=False)
    assert result.returncode == 1 and journal["state"] == "completed"
    assert [attempt["name"] for attempt in journal["attempts"]] == ["baseline"]
    assert "baseline" in journal["artifacts"]
    assert "candidate" not in journal["artifacts"] and "fused" not in journal["artifacts"]


def test_unapproved_review_keeps_real_baseline_and_never_starts_pdfium(tools, tmp_path):
    result, journal = run_job(tools, tmp_path / "bad-review", trusted_digest="0" * 64)
    assert result.returncode != 0 and journal["state"] == "review_rejected"
    assert [attempt["name"] for attempt in journal["attempts"]] == ["baseline"]
    assert "baseline" in journal["artifacts"] and "fused" not in journal["artifacts"]


def test_injected_candidate_crash_preserves_real_durable_baseline(tools, tmp_path):
    # Only the candidate is fault-injected. The first invocation execs real TPE.
    wrapper = tmp_path / "fault-injected-tpe"
    wrapper.write_text(
        f"#!{sys.executable}\nimport os, signal, sys\n"
        "if '--backend' in sys.argv and sys.argv[sys.argv.index('--backend')+1] == 'pdfium':\n"
        "    os.kill(os.getpid(), signal.SIGABRT)\n"
        f"os.execv({str(tools[0])!r}, [{str(tools[0])!r}] + sys.argv[1:])\n"
    )
    wrapper.chmod(0o700)
    output = tmp_path / "crashed-candidate"
    result, journal = run_job(tools, output, tpe=wrapper)
    assert result.returncode != 0 and journal["state"] == "child_failed"
    assert [attempt["name"] for attempt in journal["attempts"]] == ["baseline", "candidate"]
    assert digest(output / "baseline.json") == journal["artifacts"]["baseline"]["sha256"]
    assert "fused" not in journal["artifacts"]
    assert digest(SOURCE) == SOURCE_SHA


@pytest.mark.parametrize("mutation", ["longer_wrong_text", "duplicate_geometry"])
def test_adversarial_mutations_of_real_candidate_abstain(tools, completed_job, tmp_path, mutation):
    original, _ = completed_job
    candidate = json.loads((original / "candidate.json").read_bytes())
    if mutation == "longer_wrong_text":
        candidate["pages"][0]["spans"][1]["text"] = RECOVERED + " INVENTED LONGER TEXT"
    else:
        candidate["pages"][0]["spans"].append(dict(candidate["pages"][0]["spans"][1]))
    modified = tmp_path / "candidate.json"
    modified.write_text(json.dumps(candidate))
    output = tmp_path / "projection.json"
    result = subprocess.run(  # noqa: S603
        [
            str(tools[1]),
            "--source",
            str(SOURCE),
            "--baseline",
            str(original / "baseline.json"),
            "--candidate",
            str(modified),
            "--review",
            str(REVIEW),
            "--trusted-review-sha256",
            REVIEW_SHA,
            "--trusted-by",
            "pinned-fixture-review",
            "--render",
            str(RENDER),
            "--output",
            str(output),
        ],
        capture_output=True,
        timeout=15,
        check=False,
    )
    assert result.returncode == 0, result.stderr.decode()
    projection = json.loads(output.read_bytes())
    assert all(span["selected"] == span["baseline"] for span in projection["pages"][0]["spans"])
    assert projection["pages"][0]["spans"][1]["text"] == "\ufffd" * 18
    assert projection["regions"][0]["decision"] == "abstained"
