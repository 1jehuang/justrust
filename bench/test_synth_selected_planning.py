"""Exercise runner planning without snapshots, build processes or timed samples."""
import contextlib
import io
from types import SimpleNamespace
import unittest
from unittest import mock

from synth import run as runner


class SelectedPlanningTests(unittest.TestCase):
    def test_plan_calls_and_result_order(self):
        seed = 123
        points = [SimpleNamespace(i=i) for i in reversed(range(40))]
        info = {"calls": 1000}

        def plan(point, seed):
            return SimpleNamespace(point=point, snap="fixture", to_json=lambda: {"i": point.i})

        for only in ({17}, None, set(), set(range(40)), {3, 1, 99}, {99}):
            with self.subTest(only=only), contextlib.ExitStack() as ctx:
                patch = lambda name, **kw: ctx.enter_context(mock.patch.object(runner.gen, name, **kw))
                patch("_locked")
                sample = patch("sample", return_value=(points, info))
                planner = patch("plan", side_effect=plan)
                patch("restore_all")
                patch("fingerprint", return_value="fixture")
                ctx.enter_context(contextlib.redirect_stderr(io.StringIO()))
                result = runner.run([], seed, 40, 0, 0, only=only)
                expected = [p for p in points if not only or p.i in only]
                sample.assert_called_once_with(seed, 40)
                self.assertEqual(result["sample_info"], info)
                self.assertEqual([row["i"] for row in result["results"]], [p.i for p in expected])
                self.assertEqual(planner.call_count, len(expected))
                self.assertEqual(planner.call_args_list, [mock.call(p, seed) for p in expected])
