"""Test execution is not cache invalidation, but unknown/rebuilt work stays suspect.

Regression phase values come from recorded run 20261117-053052822-167953578:
zero compiled, 753 fresh, 1755 tests, 31.767s tests in a 32.824s invocation.
No measured wall, workload point, or failed result is removed or rewritten.
"""
import contextlib
import copy
import json
import tempfile
from pathlib import Path
from types import SimpleNamespace
import unittest
from unittest.mock import patch

import suite

runner, _ = suite._synth()
RID = "20261117-053052822-167953578"
COMMAND = ["test", "--workspace"]
CWD = Path("/fixture/jcode-desktop")
WALL = 32.839125606999914


def recorded():
    return {"id": RID, "args": list(COMMAND), "cwd": str(CWD), "place": "local",
            "exit": 0, "wall": 32.82391715049744,
            "units": {"compiled": 0, "fresh": 753},
            "phases": {"startup": 0.6990385055541992, "lock_wait": 0.0,
                       "compile": 0.0, "build_gaps": 0.0,
                       "test_run": 31.767080068588257, "tail": 0.35779857635498047},
            "tests": {"passed": 1755, "failed": 0}}


class RerunEvidenceTests(unittest.TestCase):
    def verify(self, summary, wall=WALL, command=None, cwd=CWD):
        return runner._verified_test_time(summary, RID, command or COMMAND, cwd, wall)

    def test_run_attaches_only_exact_successful_zero_compile_record(self):
        for mutation, accepted in (({}, True), ({"id": "stale"}, False),
                                   ({"units": {"compiled": 1, "fresh": 753}}, False),
                                   ({"place": "remote"}, False), ({"exit": 101}, False)):
            with self.subTest(mutation=mutation), tempfile.TemporaryDirectory() as tmp:
                root = Path(tmp)
                summary = recorded()
                summary.update(mutation)
                summary["cwd"] = str(root / "jcode-desktop")
                destination = root / "runs" / RID / "summary.json"
                destination.parent.mkdir(parents=True)
                destination.write_text(json.dumps(summary))
                plan = SimpleNamespace(snap="fixture", cwd="jcode-desktop",
                                       point=SimpleNamespace(i=28), expect_fail=False)
                stack = SimpleNamespace(justrust=True, name="fixture")
                def execute(argv, cwd, env, log):
                    log.write_text("")
                    return {"exit": 0, "wall": WALL}
                with contextlib.ExitStack() as context:
                    for target, name, value in (
                        (runner.gen, "snapshot_root", root),
                        (runner.suite, "new_id", RID),
                        (runner.suite, "foreign_load", (0, [])),
                        (runner.versus, "stack_env", {}),
                        (runner.versus, "sample_noisy", False)):
                        context.enter_context(patch.object(target, name, return_value=value))
                    context.enter_context(patch.object(runner.suite, "JR_HOME", root))
                    context.enter_context(patch.object(runner.versus, "WORK", root))
                    context.enter_context(patch.object(runner.versus, "run_counted", side_effect=execute))
                    context.enter_context(patch.object(runner.versus, "_mark_remote"))
                    result = runner._run(stack, plan, COMMAND, "run")
                self.assertEqual("warm_rerun_evidence" in result, accepted)
                self.assertEqual(result["wall"], WALL)
                self.assertEqual(result["id"], RID)

    def test_exact_record_proves_slow_test_runtime_without_rebuild(self):
        self.assertEqual(self.verify(recorded()), 31.767080068588257)

    def test_stale_wrong_remote_or_failed_record_is_not_evidence(self):
        for key, value in (("id", "stale"), ("args", ["check"]), ("cwd", "/wrong"),
                           ("place", "remote"), ("exit", 101), ("exit", False)):
            with self.subTest(key=key, value=value):
                summary = recorded()
                summary[key] = value
                self.assertIsNone(self.verify(summary))

    def test_compile_or_absent_fresh_units_cannot_exempt_cache_invalidation(self):
        for field, value in (("compiled", 1), ("compiled", False), ("compiled", None),
                             ("fresh", 0), ("fresh", True), ("fresh", None)):
            with self.subTest(field=field, value=value):
                summary = recorded()
                summary["units"][field] = value
                self.assertIsNone(self.verify(summary))

    def test_missing_inconsistent_or_nonfinite_phases_are_rejected(self):
        for field, value in (("compile", 0.1), ("startup", None), ("test_run", 40),
                             ("test_run", -1), ("test_run", float("nan")),
                             ("test_run", float("inf")), ("tail", True), ("tail", 10)):
            with self.subTest(field=field, value=value):
                summary = recorded()
                summary["phases"][field] = value
                self.assertIsNone(self.verify(summary))
        for value in (None, False, -1, float("nan"), float("inf"), 100):
            with self.subTest(wall=value):
                self.assertIsNone(self.verify(recorded(), wall=value))

    def test_missing_failed_or_empty_tests_are_not_proof_of_test_time(self):
        for field, value in (("passed", 0), ("passed", True), ("failed", 1), ("failed", None)):
            with self.subTest(field=field, value=value):
                summary = recorded()
                summary["tests"][field] = value
                self.assertIsNone(self.verify(summary))
        for key in ("units", "phases", "tests"):
            summary = recorded()
            del summary[key]
            self.assertIsNone(self.verify(summary))

    def test_build_check_or_no_run_commands_never_receive_test_exemption(self):
        for command in (["build"], ["check"], ["test", "--no-run"]):
            summary = recorded()
            summary["args"] = command
            self.assertIsNone(self.verify(summary, command=command))

    def run_rerun(self, evidence=None, wall=WALL, pre_wall=60.599):
        plan = SimpleNamespace(snap="fixture", pre=["test", "--workspace", "--no-run"],
                               cmd=COMMAND, prev=None, expect_fail=False, edits=[],
                               point=SimpleNamespace(shape={"change": "rerun"}))
        pre = {"exit": 0, "wall": pre_wall, "log": "/pre"}
        measured = {"exit": 0, "wall": wall, "log": "/measured",
                    "tests_failed": 0, "compile_failed": False}
        if evidence is not None:
            measured["warm_rerun_evidence"] = copy.deepcopy(evidence)
        with contextlib.ExitStack() as context:
            for target, name in ((runner.gen, "_locked"), (runner.gen, "restore_all"),
                                 (runner.gen, "restore"), (runner.os, "sync"),
                                 (runner, "_keep_log")):
                context.enter_context(patch.object(target, name))
            context.enter_context(patch.object(runner.gen, "apply", return_value=[]))
            context.enter_context(patch.object(runner, "_run", side_effect=[pre, measured]))
            return runner.run_point(SimpleNamespace(), plan, 0)

    def test_proven_test_runtime_does_not_change_measured_wall(self):
        test_time = self.verify(recorded())
        result = self.run_rerun({"record_id": RID, "compiled": 0, "fresh": 753,
                                 "test_secs": test_time})
        self.assertNotIn("suspect", result)
        self.assertEqual(result["wall"], WALL)
        self.assertEqual(result["pre_wall"], 60.599)

    def test_unknown_or_actual_rebuild_keeps_original_strict_suspicion(self):
        self.assertIn("suspect", self.run_rerun())
        summary = recorded()
        summary["units"]["compiled"] = 1
        self.assertIsNone(self.verify(summary))
        self.assertIn("suspect", self.run_rerun())

    def test_slow_non_test_overhead_stays_suspect_even_with_zero_rebuild(self):
        result = self.run_rerun({"record_id": RID, "compiled": 0, "fresh": 753,
                                 "test_secs": 20}, wall=40, pre_wall=10)
        self.assertIn("suspect", result)
        self.assertEqual(result["wall"], 40)


if __name__ == "__main__":
    unittest.main()
