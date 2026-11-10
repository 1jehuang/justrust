"""Tiny Python-only CPU-noise fixtures. Run on hosted, never compile Rust."""
import json
import os
from pathlib import Path
import sys
import tempfile
import unittest
from unittest.mock import patch

import versus


class CpuNoiseTests(unittest.TestCase):
    def measure(self, busy=1000, tree=10, steal=0):
        return versus.measured_other_cpu((0,) * 8,
            (busy, 0, 0, 100, 0, 0, 0, steal), 1, tree, 100)

    def test_own_cpu_heavy_is_quiet(self):
        row = self.measure()
        self.assertEqual(row["system_busy_cores"], 10)
        self.assertEqual(row["tree_cores"], 10)
        self.assertEqual(row["avg_other_cores"], 0)
        self.assertFalse(versus.sample_noisy(row, 0))

    def test_external_busy_and_unchanged_threshold(self):
        self.assertEqual(versus.suite.NOISY_CORES, 4)
        self.assertTrue(versus.sample_noisy(self.measure(1500), 0))
        self.assertFalse(versus.sample_noisy(self.measure(1400), 0))
        self.assertTrue(versus.sample_noisy(self.measure(), 1))

    def test_steal_cannot_be_subtracted_as_own_cpu(self):
        row = self.measure(1000, tree=20, steal=500)
        self.assertEqual(row["steal_cores"], 5)
        self.assertEqual(row["avg_other_cores"], 5)
        self.assertTrue(versus.sample_noisy(row, 0))

    def test_guest_not_double_counted_and_iowait_not_busy(self):
        with patch.object(Path, "read_text", return_value="cpu 100 20 30 40 50 6 7 8 90 10\ncpu0 0\n"):
            ticks = versus.read_cpu_ticks()
        row = versus.measured_other_cpu((0,) * 8, ticks, 1, 0, 100)
        self.assertAlmostEqual(row["system_busy_cores"], 1.71)

    def test_missing_malformed_negative_nonfinite_fail_closed(self):
        for text in ("", "cpu 1 2", "cpu 1 2 3 4 5 6 7 nan", "cpu 1 2 3 4 5 6 7 -1", "no 1 2 3 4 5 6 7 8"):
            with self.subTest(text=text), patch.object(Path, "read_text", return_value=text):
                self.assertIsNone(versus.read_cpu_ticks())
        with patch.object(Path, "read_text", side_effect=OSError("missing")):
            self.assertIsNone(versus.read_cpu_ticks())
        good = (100,) * 8
        cases = [(None, good, 1, 0, 100), (good, None, 1, 0, 100),
                 (good, (99,) * 8, 1, 0, 100), (good, good, 1, 0, 100),
                 ((-1,) * 8, good, 1, 0, 100), ((float("nan"),) * 8, good, 1, 0, 100),
                 ((0,) * 8, good, 0, 0, 100), ((0,) * 8, good, 1, 0, 0)]
        for index in (2, 3, 4):
            for invalid in (float("nan"), float("inf"), -1):
                case = [(0,) * 8, good, 1, 0, 100]
                case[index] = invalid
                cases.append(tuple(case))
        for case in cases:
            with self.subTest(case=case):
                row = versus.measured_other_cpu(*case)
                self.assertFalse(row["cpu_noise_valid"])
                self.assertIsNone(row["avg_other_cores"])
                self.assertTrue(versus.sample_noisy(row, 0))
        for other in (None, float("nan"), float("inf"), -1, "quiet"):
            self.assertTrue(versus.sample_noisy({"cpu_noise_valid": True, "avg_other_cores": other}, 0))

    def test_comparison_rejects_different_noise_policy(self):
        from types import SimpleNamespace
        with tempfile.TemporaryDirectory() as directory:
            a, b = Path(directory) / "a.json", Path(directory) / "b.json"
            a.write_text(json.dumps({"env": {"noise_policy": "old"}}))
            b.write_text(json.dumps({"env": {"noise_policy": "proc-stat-wait4-v1"}}))
            with self.assertRaisesRegex(SystemExit, "different noise_policy"):
                versus.cmd_compare(SimpleNamespace(a=str(a), b=str(b)))

    def test_named_run_prior_load_high_actual_quiet(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            # Only the public attributes run_one consumes, no snapshots/builds.
            from types import SimpleNamespace
            scenario = SimpleNamespace(name="fixture", snapshot="fixture", cmd=["test"], expect_fail=False)
            def run(argv, cwd, env, log):
                log.write_text("test result: ok. 1 passed")
                return {"exit": 0, **self.measure()}
            with patch.object(versus, "WORK", root), patch.object(versus, "stack_env", return_value={}), \
                 patch.dict(versus.suite.SNAPSHOTS, {"fixture": {"cwd": "."}}), \
                 patch.object(versus.suite, "foreign_load", return_value=(27.47, 0)), \
                 patch.object(versus, "run_counted", side_effect=run):
                row = versus.run_one(versus.STACKS[0], scenario, root, None, [])
            self.assertEqual(row["load_before"], 27.47)
            self.assertFalse(row["noisy"])

    def test_child_failure_preserves_output_exit_and_invalid_metadata(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            log = root / "child.log"
            with patch.object(versus, "read_cpu_ticks", return_value=None), \
                 patch.object(versus, "open_instruction_counters", return_value=[]):
                row = versus._run_counted([sys.executable, "-c",
                    "import sys; print('stdout fixture', flush=True); print('stderr fixture', file=sys.stderr); sys.exit(7)"],
                    root, dict(os.environ), log)
            self.assertEqual(row["exit"], 7)
            self.assertIn("stdout fixture", log.read_text())
            self.assertIn("stderr fixture", log.read_text())
            self.assertFalse(row["cpu_noise_valid"])
            self.assertTrue(versus.sample_noisy(row, 0))

    def test_remote_cpu_scope_is_not_full_build_coverage(self):
        row = self.measure()
        versus._mark_remote(versus.REMOTE, row, "")
        self.assertEqual(row["cpu_noise_scope"], "client-host-only")
        self.assertTrue(versus.sample_noisy(row, 0))

    def test_real_python_child_cpu_accounting(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            row = versus._run_counted([sys.executable, "-c",
                "import time; end=time.process_time()+0.15\nwhile time.process_time()<end: pass"],
                root, dict(os.environ), root / "python.log")
        self.assertEqual(row["exit"], 0)
        self.assertTrue(row["cpu_noise_valid"])
        self.assertGreater(row["tree_cores"], 0)
        print("hosted Python CPU accounting " + json.dumps({"load_after": os.getloadavg()[0], **row}, sort_keys=True))


if __name__ == "__main__":
    unittest.main()
