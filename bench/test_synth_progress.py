"""Synthetic progress labels, with no real builds or snapshot mutations."""
import contextlib
import io
from types import SimpleNamespace
import unittest
from unittest import mock

from synth import run as runner


class ProgressTests(unittest.TestCase):
    def exercise(self, instructions):
        data = {"i": 0, "cmd": ["check"], "pre": ["check"], "prev": None,
                "edits": [], "expect_fail": False, "snap": "fixture", "cwd": ".",
                "shape": {"command": "check", "change": "body"}}
        plan = SimpleNamespace(point=SimpleNamespace(i=0), snap="fixture",
                               cmd=["check"], to_json=lambda: data)
        sample = {"wall": 1.25, "cpu": 1.0, "max_rss_mb": 64.0,
                  "instructions": instructions, "noisy": False, "exit": 0,
                  "compile_failed": False, "tests_failed": 0}
        output = io.StringIO()
        with contextlib.ExitStack() as ctx:
            ctx.enter_context(mock.patch.object(runner.gen, "_locked"))
            ctx.enter_context(mock.patch.object(runner.gen, "sample", return_value=([0], {"calls": 1})))
            ctx.enter_context(mock.patch.object(runner.gen, "plan", return_value=plan))
            ctx.enter_context(mock.patch.object(runner.gen, "fingerprint", return_value="fixture"))
            restore = ctx.enter_context(mock.patch.object(runner.gen, "restore_all"))
            measure = ctx.enter_context(mock.patch.object(runner, "run_point", return_value=sample))
            ctx.enter_context(mock.patch.object(runner, "checkpoint_identity", return_value={"schema": 1}))
            ctx.enter_context(contextlib.redirect_stderr(output))
            result = runner.run([runner.versus.Stack("suite", "suite", True)], 1, 1, 1, 0)
            measure.assert_called_once()
            self.assertEqual(restore.call_count, 2)
        # Presentation must not rewrite raw counters or memory measurements.
        self.assertEqual(result["results"][0]["samples"]["suite"], [sample])
        return output.getvalue()

    def test_billions_of_instructions_cannot_be_mistaken_for_memory(self):
        # These counts previously appeared as bare 139.1G and 292.0G and were
        # mistaken for impossible memory totals on a 64 GiB machine.
        for instructions in (139_100_000_000, 292_000_000_000):
            with self.subTest(instructions=instructions):
                output = self.exercise(instructions)
                self.assertIn(f"{instructions / 1e9:.1f}G instructions", output)
                self.assertIn("1.25s", output)
                self.assertNotIn("GiB", output)
                self.assertNotIn("GB", output)
                self.assertIn('"current": 1', output)

    def test_missing_or_zero_instruction_counts_are_not_invented(self):
        for instructions in (None, 0):
            with self.subTest(instructions=instructions):
                output = self.exercise(instructions)
                self.assertIn("1.25s", output)
                self.assertNotIn("G instructions", output)


if __name__ == "__main__":
    unittest.main()
