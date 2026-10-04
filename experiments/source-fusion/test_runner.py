"""Focused adapter tests using frozen test helpers, not inherited test cases.

Fake tool outputs test orchestration only. Run the original region-fusion runner
suite separately to exercise the unchanged Linux lifecycle helpers.
"""

from __future__ import annotations

import hashlib
import importlib.util
import os
import signal
import subprocess
import sys
import unittest
from pathlib import Path

RUNNER = Path(__file__).with_name("run_extraction.py")
_HELPER_TESTS = RUNNER.resolve().parents[1] / "region-fusion" / "test_runner.py"
_SPEC = importlib.util.spec_from_file_location("_frozen_runner_test_helpers", _HELPER_TESTS)
if _SPEC is None or _SPEC.loader is None:
    raise RuntimeError("frozen test helpers unavailable")
_previous = importlib.util.module_from_spec(_SPEC)
_SPEC.loader.exec_module(_previous)

FAKE_SOURCE_FUSION = r"""
import json
import os
from pathlib import Path
import sys
args = sys.argv[1:]
allowed = {"--source", "--baseline", "--candidate", "--output", "--max-source-bytes"}
assert set(args[::2]) == allowed, args
assert len(args) == len(allowed) * 2
assert not (Path.cwd() / "review.json").exists()
assert not (Path.cwd() / "render-proof.bin").exists()
scenario = os.environ.get("RUNNER_TEST_SCENARIO", "normal")
if scenario == "verifier_error":
    sys.exit(19)
state = scenario if scenario in (
    "unsupported_source", "source_mismatch", "ineligible_baseline", "resource_limit", "cancelled"
) else "evaluated"
output = Path(args[args.index("--output") + 1])
result = {"policy": "source_declared_unicode_v1", "outcome": {"status": state}}
output.write_text(json.dumps(result))
"""


@unittest.skipUnless(sys.platform == "linux", "Linux adapter")
class SourceRunnerTests(unittest.TestCase):
    def setUp(self):
        # Reuse fixture/process helpers only. No previous test class is inherited
        # or imported into this module's unittest discovery namespace.
        self.fixture = _previous.RunnerProcessTests()
        self.fixture.setUp()
        self.addCleanup(self.fixture.doCleanups)
        self.original_command = self.fixture.command
        self.fixture.command = self.command
        self.fixture.fusion = self.fixture.executable("fake-fusion", FAKE_SOURCE_FUSION)

    def command(self, *, pdfium=True, **kwargs):
        command = self.original_command(pdfium=False, **kwargs)
        command[1] = str(RUNNER)
        if pdfium:
            command.append("--pdfium")
        return command

    def test_source_only_inputs_and_frozen_dependency_receipt(self):
        result = self.fixture.run_job()
        self.assertEqual(result.returncode, 0, result.stderr)
        journal = self.fixture.journal()
        self.assertEqual(journal["state"], "completed")
        self.assertEqual(journal["selection_lane"], "automatic_source_concordance")
        self.assertEqual(set(journal["artifacts"]), {"source", "baseline", "candidate", "fused"})
        dependency = Path(journal["runner_dependency"]["path"])
        self.assertEqual(
            hashlib.sha256(dependency.read_bytes()).hexdigest(),
            journal["runner_dependency"]["sha256"],
        )
        argv = journal["attempts"][-1]["argv"]
        self.assertEqual(
            set(argv[1::2]),
            {"--source", "--baseline", "--candidate", "--output", "--max-source-bytes"},
        )
        self.assertNotIn("trusted_by", journal)
        self.fixture.assert_baseline_retained()

    def test_baseline_only_does_not_invoke_source_verifier(self):
        result = self.fixture.run_job(pdfium=False)
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertEqual([a["name"] for a in self.fixture.journal()["attempts"]], ["baseline"])
        self.assertFalse((self.fixture.job / "fused.json").exists())
        self.fixture.assert_baseline_retained()

    def test_review_truth_transcript_and_render_options_are_rejected(self):
        for option in (
            "--review",
            "--trusted-review-sha256",
            "--trusted-by",
            "--truth",
            "--transcript",
            "--render",
            "--rev",
        ):
            with self.subTest(option=option):
                result = subprocess.run(  # noqa: S603
                    [*self.command(), option, "untrusted"],
                    capture_output=True,
                    timeout=10,
                    check=False,
                )
                self.assertEqual(result.returncode, 2)
                self.assertIn(b"unrecognized arguments", result.stderr)
                self.assertFalse(self.fixture.job.exists())

    def test_non_evaluated_verifier_outcomes_never_publish_fused(self):
        for outcome in (
            "unsupported_source",
            "source_mismatch",
            "ineligible_baseline",
            "resource_limit",
            "cancelled",
        ):
            with self.subTest(outcome=outcome):
                self.fixture.job = self.fixture.root / outcome
                result = self.fixture.run_job(outcome)
                self.assertEqual(result.returncode, 1)
                self.assertEqual(self.fixture.journal()["state"], "fusion_incomplete")
                self.assertNotIn("fused", self.fixture.journal()["artifacts"])
                self.assertFalse((self.fixture.job / "fused.json").exists())
                self.assertTrue((self.fixture.job / "fusion-output.json").exists())
                self.fixture.assert_baseline_retained()

    def test_verifier_process_failure_preserves_baseline_without_fused(self):
        result = self.fixture.run_job("verifier_error")
        self.assertEqual(result.returncode, 1)
        self.assertEqual(self.fixture.journal()["state"], "child_failed")
        self.assertFalse((self.fixture.job / "fused.json").exists())
        self.fixture.assert_baseline_retained()

    def test_either_partial_extraction_remains_partial_and_nonzero(self):
        for scenario, key in (
            ("partial", "baseline_status"),
            ("candidate_partial", "candidate_status"),
        ):
            with self.subTest(scenario=scenario):
                self.fixture.job = self.fixture.root / scenario
                result = self.fixture.run_job(scenario)
                self.assertEqual(result.returncode, 1)
                self.assertEqual(self.fixture.journal()[key], "partial")
                self.assertEqual(self.fixture.journal()["state"], "completed")
                self.fixture.assert_baseline_retained()

    def test_new_orchestration_shares_deadline_with_candidate(self):
        result = self.fixture.run_job("shared_deadline", timeout=650)
        self.assertEqual(result.returncode, 1)
        self.assertEqual(self.fixture.journal()["state"], "timed_out")
        self.assertEqual(len(self.fixture.journal()["attempts"]), 2)
        self.assertFalse((self.fixture.job / "fused.json").exists())
        self.fixture.assert_baseline_retained()

    def test_signal_during_adapter_commit_demotes_fused(self):
        harness = self.fixture.root / "commit-signal.py"
        harness.write_text(
            "import importlib.util,json,os,signal,sys\n"
            f"spec=importlib.util.spec_from_file_location('runner', {str(RUNNER)!r})\n"
            "module=importlib.util.module_from_spec(spec);spec.loader.exec_module(module)\n"
            "original=module.helpers.sync_directory\n"
            "sent=False\n"
            "def during_commit(path):\n"
            "    global sent\n"
            "    original(path)\n"
            "    journal=path/'journal.json'\n"
            "    if journal.exists() and not sent:\n"
            "        if json.loads(journal.read_text()).get('state') == 'completed':\n"
            "            sent=True\n"
            "            os.kill(os.getpid(),signal.SIGTERM)\n"
            "module.helpers.sync_directory=during_commit\n"
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
        self.assertEqual(self.fixture.journal()["state"], "cancelled")
        self.assertFalse((self.fixture.job / "fused.json").exists())
        self.assertNotIn("fused", self.fixture.journal()["artifacts"])
        self.fixture.assert_baseline_retained()


if __name__ == "__main__":
    unittest.main()
