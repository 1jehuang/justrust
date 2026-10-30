"""Named-suite correctness gates, with fake processes and no native workloads."""
import argparse
import contextlib
import io
import json
from pathlib import Path
import tempfile
import unittest
from unittest.mock import patch

import suite


class NamedFailureGateTests(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory()
        self.addCleanup(self.temp.cleanup)
        self.root = Path(self.temp.name)
        self.run = self.root / "runs" / "fixture-run"
        self.run.mkdir(parents=True)
        self.summary = {"wall": 1, "tests": {"passed": 10, "failed": 1},
                        "diagnostics": {"compile_failed": False},
                        "phases": {"startup": 0, "compile": 0, "test_run": 1},
                        "units": {"compiled": 0, "local": 0, "dependencies": 0}}
        self.write_summary()

    def write_summary(self):
        (self.run / "summary.json").write_text(json.dumps(self.summary))

    @contextlib.contextmanager
    def fake_process(self, status=101):
        class Process:
            stdout = io.StringIO("thread panicked at original assertion\n")
            stderr = io.StringIO("original failing test output\n")
            returncode = status

            def wait(self):
                return self.returncode

        with patch.object(suite, "JR_HOME", self.root), \
                patch.object(suite, "new_id", return_value="fixture-run"), \
                patch.object(suite, "foreign_load", return_value=(0, 0)), \
                patch.object(suite, "REMOTE", False), \
                patch.object(suite, "workspace_thread_provenance", return_value=None), \
                patch.object(suite.subprocess, "Popen", return_value=Process()) as popen:
            yield popen

    def scenario(self, expected=False):
        return suite.Scenario("fixture", "justrust", ["test"], None, "fixture", expect_test_fail=expected)

    def test_unexpected_failure_exits_with_run_and_original_error(self):
        with self.fake_process(), self.assertRaises(SystemExit) as error:
            suite.run_once(self.scenario(), self.root, self.root / "target")
        self.assertIn("unexpected failing test(s), sample rejected", str(error.exception))
        self.assertIn("justrust log fixture-run", str(error.exception))
        self.assertIn("original failing test output", str(error.exception))

    def test_failure_count_rejects_even_inconsistent_success_exit(self):
        with self.fake_process(status=0), self.assertRaisesRegex(SystemExit, "sample rejected"):
            suite.run_once(self.scenario(), self.root, self.root / "target")

    def test_expected_compile_error_remains_valid(self):
        self.summary["tests"]["failed"] = 0
        self.summary["diagnostics"]["compile_failed"] = True
        self.write_summary()
        scenario = suite.Scenario("fixture", "justrust", ["check"], None, "fixture", expect_fail=True)
        with self.fake_process():
            sample = suite.run_once(scenario, self.root, self.root / "target")
        self.assertEqual(sample["tests_failed"], 0)

    def test_expected_failure_remains_a_valid_diagnostic(self):
        with self.fake_process():
            sample = suite.run_once(self.scenario(expected=True), self.root, self.root / "target")
        self.assertEqual(sample["tests_failed"], 1)

    def test_expected_failure_must_actually_fail(self):
        self.summary["tests"]["failed"] = 0
        self.write_summary()
        with self.fake_process(status=0), self.assertRaisesRegex(SystemExit, "expected a failing test"):
            suite.run_once(self.scenario(expected=True), self.root, self.root / "target")

    def test_failed_warmup_does_not_write_completed_named_suite(self):
        output = self.root / "must-not-exist.json"
        args = argparse.Namespace(toolchain="system", remote=False, synth=None, cold=False,
                                  scenarios=["desktop-noop-test"], n=3, warmup=1,
                                  quiet_wait=0, out=str(output))
        with self.fake_process(), patch.object(suite, "prepare", return_value=self.root), \
                patch.object(suite, "env_info", return_value={}), \
                patch.object(suite, "restore") as restore, \
                self.assertRaisesRegex(SystemExit, "sample rejected"):
            suite.cmd_run(args)
        restore.assert_called()
        self.assertFalse(output.exists())

    def test_compare_rejects_any_failed_sample_or_aggregate_before_speed_output(self):
        for location in ("sample", "stats", "scenario"):
            for failed_side in ("a", "b"):
                with self.subTest(location=location, side=failed_side):
                    paths = []
                    for side, wall in (("a", 5), ("b", 1)):
                        scenario = {"samples": [{"wall": wall, "recorded_wall": wall, "noisy": False,
                                                 "tests_failed": 0}],
                                    "stats": {"median": wall, "min": wall, "max": wall}}
                        if side == failed_side:
                            target = scenario["samples"][0] if location == "sample" else scenario["stats"] if location == "stats" else scenario
                            target["tests_failed"] = 1
                        result = {"env": {"snapshots": {}, "justrust_rev": "fixture"},
                                  "scenarios": {"justrust-body-test": scenario}}
                        path = self.root / (side + ".json")
                        path.write_text(json.dumps(result))
                        paths.append(str(path))
                    output = io.StringIO()
                    with contextlib.redirect_stdout(output), self.assertRaisesRegex(SystemExit, "comparison blocked"):
                        suite.cmd_compare(argparse.Namespace(a=paths[0], b=paths[1], floor=10))
                    self.assertEqual(output.getvalue(), "")

    def test_later_noisy_failure_cannot_disappear_behind_clean_median(self):
        result = {"scenarios": {"justrust-body-test": {
            "samples": [{"tests_failed": 0, "noisy": False}, {"tests_failed": 1, "noisy": True}],
            "stats": {"tests_failed": 0}}}}
        self.assertIn("unexpected failing tests", suite.named_test_failure_issues(result)[0])

    def test_expected_failure_legacy_and_explicit_provenance_are_preserved(self):
        for explicit in (False, True):
            scenario = {"samples": [{"tests_failed": 3}]}
            if explicit:
                scenario["expect_test_fail"] = True
            result = {"scenarios": {"test-retry-storm": scenario}}
            self.assertEqual(suite.named_test_failure_issues(result), [])

    def test_relabeling_success_scenario_cannot_hide_failure(self):
        result = {"scenarios": {"justrust-body-test": {"expect_test_fail": True,
                                                     "samples": [{"tests_failed": 1}]}}}
        self.assertIn("contradicts", suite.named_test_failure_issues(result)[0])


if __name__ == "__main__":
    unittest.main()
