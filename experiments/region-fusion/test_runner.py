"""Process-containment tests with fake extraction/fusion executables.

These exercise real Linux processes, signals, limits and durable artifacts. They
do not qualify PDF extraction accuracy or the Rust adjudication selector. Run:
  uv run --no-project python -m unittest discover -s experiments/region-fusion -p test_runner.py
"""

from __future__ import annotations

import hashlib
import json
import os
import signal
import subprocess
import sys
import tempfile
import time
import unittest
from pathlib import Path

RUNNER = Path(__file__).with_name("run_extraction.py")

FAKE_TPE = r"""
import ctypes
import hashlib
import json
import os
from pathlib import Path
import signal
import subprocess
import sys
import time

args = sys.argv[1:]
def arg(name):
    return args[args.index(name) + 1]
backend = arg("--backend")
source = Path(args[-1])
out = Path(arg("--out"))
scenario = os.environ.get("RUNNER_TEST_SCENARIO", "normal")
if backend == "lopdf" and scenario == "shared_deadline":
    time.sleep(0.3)
if backend == "pdfium":
    baseline = Path.cwd() / "baseline.json"
    journal = json.loads((Path.cwd() / "journal.json").read_text())
    baseline_digest = hashlib.sha256(baseline.read_bytes()).hexdigest()
    assert journal["artifacts"]["baseline"]["sha256"] == baseline_digest
    assert any(e["event"] == "baseline_persisted" for e in journal["events"])
    (Path.cwd() / "candidate-started").write_text(str(os.getpid()))
    if scenario in ("timeout", "shared_deadline"):
        time.sleep(10)
    if scenario == "crash":
        os.kill(os.getpid(), signal.SIGSEGV)
    if scenario == "output_flood":
        while True:
            os.write(1, b"x" * 65536)
    if scenario == "artifact_flood":
        (out / "huge.json").write_bytes(b"x" * 2000000)
    if scenario == "missing":
        sys.exit(0)
    if scenario == "mutate_source":
        source.chmod(0o600)
        source.write_bytes(b"changed source snapshot")
    if scenario in ("cancel", "parent_death"):
        if scenario == "cancel":
            # Real owned descendant, not a mocked wait/kill API.
            child = subprocess.Popen(
                [sys.executable, "-c", "import time; time.sleep(30)"], start_new_session=True
            )
            (Path.cwd() / "grandchild-pid").write_text(str(child.pid))
        time.sleep(30)
data = source.read_bytes()
digest = hashlib.sha256(data).hexdigest()
status = "partial" if backend == "lopdf" and scenario == "partial" else "complete"
if scenario == "candidate_partial" and backend == "pdfium":
    status = "partial"
if scenario == "invalid_status" and backend == "pdfium":
    status = "partial"
result = {
    "document": {"hash": digest, "size": len(data)},
    "backend": {"name": backend},
    "status": status,
    "pages": [{"text": "baseline neighbour" if backend == "lopdf" else "candidate"}],
    "warnings": ["unresolved baseline"] if status == "partial" else []
}
(out / (digest + ".json")).write_text(json.dumps(result))
print("saved " + backend)
sys.exit(1 if status == "partial" and scenario != "invalid_status" else 0)
"""

FAKE_FUSION = r"""
import json
import os
from pathlib import Path
import sys
args = sys.argv[1:]
output = Path(args[args.index("--output") + 1])
scenario = os.environ.get("RUNNER_TEST_SCENARIO", "normal")
if scenario == "fusion_crash":
    sys.exit(17)
state = "resource_limit" if scenario == "fusion_limit" else "evaluated"
output.write_text(json.dumps({"outcome": {"state": state}, "selection": "external adjudication"}))
"""


@unittest.skipUnless(sys.platform == "linux", "Linux process containment pilot")
class RunnerProcessTests(unittest.TestCase):
    def setUp(self):
        self.temporary = tempfile.TemporaryDirectory(prefix="region-runner-test-")
        self.addCleanup(self.temporary.cleanup)
        self.root = Path(self.temporary.name)
        self.source = self.root / "original.pdf"
        self.source.write_bytes(b"%PDF-1.4\nmock input; no extraction accuracy claim\n")
        self.original = self.source.read_bytes()
        self.review = self.root / "review.json"
        self.review.write_text('{"review": "external test adjudication"}\n')
        self.render = self.root / "render.bin"
        self.render.write_bytes(b"test raster proof")
        self.tpe = self.executable("fake-tpe", FAKE_TPE)
        self.fusion = self.executable("fake-fusion", FAKE_FUSION)
        self.job = self.root / "job"

    def executable(self, name, body):
        path = self.root / name
        path.write_text(f"#!{sys.executable}\n" + body)
        path.chmod(0o700)
        return path

    def command(self, *, pdfium=True, timeout=5000, output_cap=65536):
        args = [
            sys.executable,
            str(RUNNER),
            "--source",
            str(self.source),
            "--out",
            str(self.job),
            "--tpe",
            str(self.tpe),
            "--fusion",
            str(self.fusion),
            "--timeout-ms",
            str(timeout),
            "--max-output-bytes",
            str(output_cap),
            "--max-memory-growth-mib",
            "64",
        ]
        if pdfium:
            args.extend(
                [
                    "--pdfium",
                    "--review",
                    str(self.review),
                    "--trusted-review-sha256",
                    hashlib.sha256(self.review.read_bytes()).hexdigest(),
                    "--trusted-by",
                    "unit-test external adjudicator",
                    "--render",
                    str(self.render),
                ]
            )
        return args

    def run_job(self, scenario="normal", **kwargs):
        env = os.environ.copy()
        env.pop("PDFIUM_DYNAMIC_LIB_PATH", None)
        env["RUNNER_TEST_SCENARIO"] = scenario
        result = subprocess.run(  # noqa: S603
            self.command(**kwargs), env=env, capture_output=True, timeout=10, check=False
        )
        self.assertEqual(self.source.read_bytes(), self.original)
        return result

    def journal(self):
        return json.loads((self.job / "journal.json").read_text())

    def assert_baseline_retained(self):
        journal = self.journal()
        original = (
            self.job / "baseline-pass" / (hashlib.sha256(self.original).hexdigest() + ".json")
        )
        baseline = self.job / "baseline.json"
        self.assertEqual(baseline.read_bytes(), original.read_bytes())
        self.assertEqual(
            hashlib.sha256(baseline.read_bytes()).hexdigest(),
            journal["artifacts"]["baseline"]["sha256"],
        )
        self.assertEqual(baseline.stat().st_mode & 0o222, 0)
        self.assertTrue(any(e["event"] == "baseline_persisted" for e in journal["events"]))

    def wait_for_file(self, path, process):
        until = time.monotonic() + 5
        while time.monotonic() < until:
            if path.exists() and path.stat().st_size:
                return
            if process.poll() is not None:
                self.fail(f"runner exited before {path.name}: {process.returncode}")
            time.sleep(0.01)
        self.fail(f"timed out waiting for {path}")

    def assert_dead(self, pid):
        until = time.monotonic() + 2
        while time.monotonic() < until:
            path = Path(f"/proc/{pid}/stat")
            if not path.exists():
                return
            # Abrupt supervisor death leaves reaping to the environment's init.
            if path.read_text().split()[2] == "Z":
                return
            time.sleep(0.01)
        self.fail(f"process {pid} is still alive")

    def start_job(self, scenario):
        env = os.environ.copy()
        env.pop("PDFIUM_DYNAMIC_LIB_PATH", None)
        env["RUNNER_TEST_SCENARIO"] = scenario
        process = subprocess.Popen(  # noqa: S603
            self.command(timeout=10000),
            env=env,
            stdout=subprocess.DEVNULL,
            stderr=subprocess.DEVNULL,
        )

        def cleanup():
            if process.poll() is None:
                process.kill()
            process.wait()

        self.addCleanup(cleanup)
        return process

    def test_completed_baseline_precedes_candidate_and_fusion(self):
        result = self.run_job()
        self.assertEqual(result.returncode, 0, result.stderr)
        journal = self.journal()
        self.assertEqual(journal["state"], "completed")
        self.assertEqual(
            [a["name"] for a in journal["attempts"]], ["baseline", "candidate", "fusion"]
        )
        self.assertTrue(all(a["state"] == "completed" for a in journal["attempts"]))
        self.assertEqual((self.job / "source.pdf").read_bytes(), self.original)
        self.assertEqual((self.job / "source.pdf").stat().st_mode & 0o222, 0)
        self.assert_baseline_retained()
        for attempt in journal["attempts"][:2]:
            self.assertNotIn("--json", attempt["argv"])
            self.assertIn("--max-memory-growth-mib", attempt["argv"])
            self.assertIn("--max-output-bytes", attempt["argv"])

    def test_baseline_only_requires_no_review_or_candidate(self):
        result = self.run_job(pdfium=False)
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertEqual([a["name"] for a in self.journal()["attempts"]], ["baseline"])
        self.assertFalse((self.job / "candidate.json").exists())
        self.assert_baseline_retained()

    def test_partial_baseline_status_and_warnings_not_upgraded(self):
        result = self.run_job("partial")
        self.assertEqual(result.returncode, 1)
        self.assertEqual(self.journal()["state"], "completed")
        self.assertEqual(self.journal()["baseline_status"], "partial")
        self.assertTrue((self.job / "fused.json").exists())
        self.assertEqual(
            json.loads((self.job / "baseline.json").read_text())["warnings"],
            ["unresolved baseline"],
        )
        self.assert_baseline_retained()

    def test_candidate_crash_cannot_erase_baseline(self):
        result = self.run_job("crash")
        self.assertEqual(result.returncode, 1)
        self.assertEqual(self.journal()["state"], "child_failed")
        self.assertEqual(self.journal()["attempts"][-1]["returncode"], -signal.SIGSEGV)
        self.assert_baseline_retained()

    def test_partial_candidate_keeps_completed_baseline_but_returns_nonzero(self):
        result = self.run_job("candidate_partial")
        self.assertEqual(result.returncode, 1)
        journal = self.journal()
        self.assertEqual(journal["state"], "completed")
        self.assertEqual(journal["baseline_status"], "complete")
        self.assertEqual(journal["candidate_status"], "partial")
        self.assertTrue((self.job / "fused.json").exists())
        self.assert_baseline_retained()

    def test_shared_deadline_is_not_reset_for_candidate(self):
        started = time.monotonic()
        result = self.run_job("shared_deadline", timeout=650)
        self.assertEqual(result.returncode, 1)
        self.assertLess(time.monotonic() - started, 2)
        journal = self.journal()
        self.assertEqual(journal["state"], "timed_out")
        limits = []
        for attempt in journal["attempts"]:
            argv = attempt["argv"]
            limits.append(int(argv[argv.index("--timeout-ms") + 1]))
        self.assertEqual(len(limits), 2)
        self.assertLess(limits[1], limits[0] - 200)
        self.assert_baseline_retained()

    def test_stdout_flood_is_bounded_and_baseline_retained(self):
        result = self.run_job("output_flood", output_cap=1024)
        self.assertEqual(result.returncode, 1)
        self.assertEqual(self.journal()["state"], "output_limit")
        self.assertLessEqual((self.job / "candidate.stdout.log").stat().st_size, 1024)
        self.assert_baseline_retained()

    def test_artifact_file_hard_cap_stops_child(self):
        result = self.run_job("artifact_flood", output_cap=1024)
        self.assertEqual(result.returncode, 1)
        self.assertNotEqual(self.journal()["state"], "completed")
        self.assertLessEqual((self.job / "candidate-pass" / "huge.json").stat().st_size, 1024)
        self.assert_baseline_retained()

    def test_completed_process_must_produce_matching_valid_status(self):
        result = self.run_job("invalid_status")
        self.assertEqual(result.returncode, 1)
        self.assertEqual(self.journal()["state"], "invalid_artifact")
        self.assert_baseline_retained()

    def test_missing_candidate_is_failure_with_baseline(self):
        result = self.run_job("missing")
        self.assertEqual(result.returncode, 1)
        self.assertEqual(self.journal()["state"], "failed")
        self.assert_baseline_retained()

    def test_fusion_crash_is_failure_with_baseline(self):
        result = self.run_job("fusion_crash")
        self.assertEqual(result.returncode, 1)
        self.assertEqual(self.journal()["state"], "child_failed")
        self.assert_baseline_retained()

    def test_fusion_resource_limit_is_not_reported_as_success(self):
        result = self.run_job("fusion_limit")
        self.assertEqual(result.returncode, 1)
        self.assertEqual(self.journal()["state"], "fusion_incomplete")
        self.assertFalse((self.job / "fused.json").exists())
        self.assertNotIn("fused", self.journal()["artifacts"])
        self.assert_baseline_retained()

    def test_explicit_source_cap_is_enforced_before_child_work(self):
        command = [*self.command(), "--max-source-bytes", "1"]
        result = subprocess.run(command, capture_output=True, timeout=10, check=False)  # noqa: S603
        self.assertEqual(result.returncode, 1)
        self.assertEqual(self.journal()["state"], "input_limit")
        self.assertEqual(self.journal()["attempts"], [])
        self.assertEqual(self.source.read_bytes(), self.original)

    def test_source_size_is_explicitly_passed_to_tpe(self):
        self.assertEqual(self.run_job(pdfium=False).returncode, 0)
        argv = self.journal()["attempts"][0]["argv"]
        self.assertEqual(int(argv[argv.index("--max-bytes") + 1]), len(self.original))
        self.assertIsNone(self.journal()["limits"]["max_source_bytes"])

    def test_unapproved_review_digest_blocks_candidate_after_baseline(self):
        command = self.command()
        command[command.index("--trusted-review-sha256") + 1] = "0" * 64
        result = subprocess.run(command, capture_output=True, timeout=10, check=False)  # noqa: S603
        self.assertEqual(result.returncode, 1)
        self.assertEqual(self.journal()["state"], "review_rejected")
        self.assertEqual(len(self.journal()["attempts"]), 1)
        self.assert_baseline_retained()

    def test_existing_directory_is_never_reused(self):
        self.job.mkdir()
        sentinel = self.job / "keep.txt"
        sentinel.write_text("existing data")
        result = self.run_job()
        self.assertEqual(result.returncode, 1)
        self.assertEqual(sentinel.read_text(), "existing data")
        self.assertFalse((self.job / "journal.json").exists())

    def test_source_alias_as_output_is_rejected_without_write(self):
        self.job.symlink_to(self.source)
        result = self.run_job()
        self.assertEqual(result.returncode, 1)
        self.assertEqual(self.source.read_bytes(), self.original)

    def test_snapshot_tampering_is_detected(self):
        result = self.run_job("mutate_source")
        self.assertEqual(result.returncode, 1)
        self.assertEqual(self.journal()["state"], "artifact_changed")
        self.assert_baseline_retained()

    def test_cancellation_kills_and_reaps_owned_process_group(self):
        process = self.start_job("cancel")
        self.wait_for_file(self.job / "grandchild-pid", process)
        child_pid = int((self.job / "candidate-started").read_text())
        grandchild_pid = int((self.job / "grandchild-pid").read_text())
        process.send_signal(signal.SIGTERM)
        self.assertEqual(process.wait(timeout=5), 128 + signal.SIGTERM)
        self.assertEqual(self.journal()["state"], "cancelled")
        self.assert_dead(child_pid)
        self.assert_dead(grandchild_pid)
        self.assertFalse(
            Path(f"/proc/{grandchild_pid}").exists(), "owned grandchild must be reaped"
        )
        self.assert_baseline_retained()

    def test_sigint_also_stops_new_work_and_preserves_baseline(self):
        process = self.start_job("cancel")
        self.wait_for_file(self.job / "grandchild-pid", process)
        process.send_signal(signal.SIGINT)
        self.assertEqual(process.wait(timeout=5), 128 + signal.SIGINT)
        self.assertEqual(self.journal()["state"], "cancelled")
        self.assertFalse((self.job / "fused.json").exists())
        self.assert_baseline_retained()

    def test_signal_during_final_commit_demotes_fused_output(self):
        harness = self.root / "commit-signal.py"
        harness.write_text(
            "import importlib.util,json,os,signal,sys\n"
            f"spec=importlib.util.spec_from_file_location('runner', {str(RUNNER)!r})\n"
            "module=importlib.util.module_from_spec(spec);spec.loader.exec_module(module)\n"
            "original=module.sync_directory\n"
            "sent=False\n"
            "def during_commit(path):\n"
            "    global sent\n"
            "    original(path)\n"
            "    journal=path/'journal.json'\n"
            "    if journal.exists() and not sent:\n"
            "        if json.loads(journal.read_text()).get('state') == 'completed':\n"
            "            sent=True\n"
            "            os.kill(os.getpid(),signal.SIGTERM)\n"
            "module.sync_directory=during_commit\n"
            "sys.exit(module.main())\n"
        )
        command = self.command()
        command[1] = str(harness)
        env = os.environ.copy()
        env.pop("PDFIUM_DYNAMIC_LIB_PATH", None)
        result = subprocess.run(  # noqa: S603
            command, env=env, capture_output=True, timeout=10, check=False
        )
        self.assertEqual(result.returncode, 128 + signal.SIGTERM, result.stderr)
        self.assertEqual(self.journal()["state"], "cancelled")
        self.assertFalse((self.job / "fused.json").exists())
        self.assertTrue((self.job / "uncommitted-fused.json").exists())
        self.assertNotIn("fused", self.journal()["artifacts"])
        self.assert_baseline_retained()

    def test_abrupt_parent_death_kills_controller_and_keeps_durable_baseline(self):
        process = self.start_job("parent_death")
        self.wait_for_file(self.job / "candidate-started", process)
        child_pid = int((self.job / "candidate-started").read_text())
        process.kill()
        process.wait(timeout=5)
        self.assert_dead(child_pid)
        # SIGKILL cannot run journal cleanup. The durable running receipt is
        # intentionally honest about that interrupted attempt.
        self.assertEqual(self.journal()["attempts"][-1]["state"], "running")
        self.assert_baseline_retained()


if __name__ == "__main__":
    unittest.main()
