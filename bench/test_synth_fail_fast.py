"""Mocked synthetic fail-fast gates, without builds or shared snapshots."""
import contextlib
import io
import json
from pathlib import Path
import tempfile
from types import SimpleNamespace
import unittest
from unittest import mock

from synth import run as runner


class FailFastTests(unittest.TestCase):
    def exercise(self, responses, *, stop=True, reps=2):
        with tempfile.TemporaryDirectory() as tmp, contextlib.ExitStack() as ctx:
            root = Path(tmp)
            source = root / "snapshot.rs"
            source.write_text("pristine")
            partial = root / "partial.json"
            plans = []
            for i in range(3):
                data = {"i": i, "cmd": ["check"], "pre": ["check"], "prev": None,
                        "edits": [], "expect_fail": False, "snap": "fixture", "cwd": ".",
                        "shape": {"command": "check", "change": "body"}}
                plans.append(SimpleNamespace(point=SimpleNamespace(i=i), snap="fixture",
                                             cmd=["check"], to_json=lambda data=data: data))
            stacks = [runner.versus.Stack(name, name, True) for name in ("a", "b")]
            restores = []

            def restore(snap):
                restores.append(snap)
                source.write_text("pristine")

            calls = []

            def measure(stack, plan, n):
                calls.append((plan.point.i, stack.name))
                source.write_text("edited")
                return dict(responses.get((plan.point.i, stack.name), self.sample()))

            ctx.enter_context(mock.patch.object(runner.gen, "_locked"))
            ctx.enter_context(mock.patch.object(runner.gen, "sample", return_value=(list(range(3)), {"calls": 3, "coverage": 1})))
            ctx.enter_context(mock.patch.object(runner.gen, "plan", side_effect=lambda p, seed: plans[p]))
            ctx.enter_context(mock.patch.object(runner.gen, "fingerprint", return_value="fixture"))
            ctx.enter_context(mock.patch.object(runner.gen, "restore_all", side_effect=restore))
            ctx.enter_context(mock.patch.object(runner, "run_point", side_effect=measure))
            ctx.enter_context(contextlib.redirect_stderr(io.StringIO()))
            result = runner.run(stacks, 1, 3, reps, 0, partial=partial, stop_on_invalid=stop)
            saved = json.loads(partial.read_text())
            self.assertEqual(source.read_text(), "pristine")
            self.assertEqual(restores, ["fixture", "fixture"])
            self.assertEqual(saved["results"], result["results"])
            self.assertEqual(saved["points"], 3)
            self.assertTrue(saved["partial"])
            self.assertEqual(saved.get("stopped_early"), result.get("stopped_early"))
            return result, calls

    @staticmethod
    def sample(noisy=False):
        return {"wall": 1.25, "cpu": 1.0, "max_rss_mb": 4.0, "instructions": 100, "noisy": noisy,
                "exit": 0, "compile_failed": False, "tests_failed": 0,
                "original_evidence": {"raw": [1, 2, 3]}}

    def test_error_finishes_point_and_preserves_original_error(self):
        result, calls = self.exercise({(0, "a"): {"error": "original failure: log /fixture/run.log"}})
        self.assertEqual(calls, [(0, "a"), (0, "b"), (0, "b")])
        self.assertEqual(result["results"][0]["errors"], {"a": "original failure: log /fixture/run.log"})
        self.assertEqual(result["results"][0]["samples"]["a"], [])
        self.assertEqual(result["results"][0]["samples"]["b"], [self.sample(), self.sample()])
        self.assertEqual(result["stopped_early"], "point 0: measurement errors")
        self.assertTrue(runner.comparison_issues(result, result))

    def test_noisy_finishes_all_replicates_and_preserves_samples(self):
        noisy = self.sample(True)
        result, calls = self.exercise({(0, "a"): noisy})
        self.assertEqual(len(calls), 4)
        self.assertEqual(result["results"][0]["samples"]["a"], [noisy, noisy])
        self.assertEqual(result["results"][0]["errors"], {})
        self.assertEqual(result["stopped_early"], "point 0: noisy samples")
        self.assertTrue(runner.comparison_issues(result, result))

    def test_error_and_noise_both_retained(self):
        result, _ = self.exercise({(0, "a"): {"error": "failed"}, (0, "b"): self.sample(True)})
        self.assertEqual(result["stopped_early"], "point 0: measurement errors, noisy samples")

    def test_later_failure_keeps_prior_points_and_full_expected_count(self):
        result, calls = self.exercise({(1, "a"): {"error": "later failure"}})
        self.assertEqual([p["i"] for p in result["results"]], [0, 1])
        self.assertEqual(result["points"], 3)
        self.assertFalse(any(i == 2 for i, _ in calls))
        self.assertEqual(result["results"][0]["samples"]["a"], [self.sample(), self.sample()])
        self.assertEqual(result["results"][1]["errors"], {"a": "later failure"})
        self.assertEqual(result["stopped_early"], "point 1: measurement errors")

    def test_valid_gate_runs_full_workload(self):
        result, calls = self.exercise({})
        self.assertEqual(len(calls), 12)
        self.assertEqual(len(result["results"]), 3)
        self.assertNotIn("stopped_early", result)
        self.assertEqual(runner.comparison_issues(result, result), [])

    def test_default_diagnostic_continues_through_errors_and_noise(self):
        result, calls = self.exercise({(0, "a"): {"error": "failed"}, (0, "b"): self.sample(True)}, stop=False)
        self.assertEqual(len(result["results"]), 3)
        self.assertTrue(any(i == 2 for i, _ in calls))
        self.assertNotIn("stopped_early", result)
        self.assertEqual(result["results"][0]["errors"], {"a": "failed"})
        self.assertTrue(result["results"][0]["samples"]["b"][0]["noisy"])

    def test_default_is_opt_in(self):
        import inspect
        self.assertIs(inspect.signature(runner.run).parameters["stop_on_invalid"].default, False)

    def test_resume_refused_before_lock_sampling_or_snapshot_mutation(self):
        with mock.patch.object(runner.gen, "_locked") as lock, \
                mock.patch.object(runner.gen, "sample") as sample, \
                mock.patch.object(runner.gen, "restore_all") as restore:
            with self.assertRaisesRegex(ValueError, "stop_on_invalid.*resume"):
                runner.run([], 1, 3, 2, 0, resume=True, stop_on_invalid=True)
            lock.assert_not_called()
            sample.assert_not_called()
            restore.assert_not_called()


if __name__ == "__main__":
    unittest.main()
