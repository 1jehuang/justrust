"""Opt-in invalid synthetic gates save/upload original evidence before failure."""
import argparse
import contextlib
import io
import json
from pathlib import Path
import tempfile
import unittest
from unittest import mock

import suite
import versus


class StopInvalidCliTests(unittest.TestCase):
    def test_named_and_resumed_runs_rejected_before_preflight_or_install(self):
        for synth, resume in ((None, False), (1, True)):
            with self.subTest(synth=synth, resume=resume), \
                    mock.patch.object(suite, "preflight_runtime") as preflight, \
                    mock.patch.object(suite.subprocess, "run") as child:
                args = argparse.Namespace(stop_on_invalid=True, synth=synth, resume=resume)
                with self.assertRaises(SystemExit):
                    suite.cmd_run(args)
                preflight.assert_not_called()
                child.assert_not_called()

    def exercise(self, enabled, stopped):
        with tempfile.TemporaryDirectory() as directory:
            out = Path(directory) / "original.json"
            original = {"points": 40, "results": [{"i": 0, "errors": {"suite": "original error"}}]}
            if stopped:
                original["stopped_early"] = "point 0: original error"
            runner = mock.Mock()
            runner.run.return_value = original
            runner.table.return_value = "original diagnostic"
            args = argparse.Namespace(synth=1, points=40, n=1, quiet_wait=0,
                                      resume=False, out=str(out), stop_on_invalid=enabled)
            uploaded = []

            def upload(path):
                self.assertTrue(path.exists())
                uploaded.append(json.loads(path.read_text()))

            with mock.patch.object(suite, "TOOLCHAIN", "system"), \
                    mock.patch.object(suite, "_synth", return_value=(runner, versus)), \
                    mock.patch.object(suite, "env_info", return_value={"fixture": True}), \
                    mock.patch.object(suite, "verify_binary_identity"), \
                    mock.patch.object(suite, "upload_result", side_effect=upload), \
                    contextlib.redirect_stdout(io.StringIO()), \
                    contextlib.redirect_stderr(io.StringIO()):
                if enabled and stopped:
                    with self.assertRaises(SystemExit) as raised:
                        suite.cmd_run_synth(args)
                    self.assertEqual(raised.exception.code, 2)
                else:
                    suite.cmd_run_synth(args)
            self.assertEqual(json.loads(out.read_text())["synth"], original)
            self.assertEqual(len(uploaded), 1)
            self.assertEqual(uploaded[0]["synth"], original)
            kwargs = runner.run.call_args.kwargs
            if enabled:
                self.assertIs(kwargs["stop_on_invalid"], True)
            else:
                self.assertNotIn("stop_on_invalid", kwargs)

    def test_stopped_gate_saves_and_uploads_before_nonzero_exit(self):
        self.exercise(True, True)

    def test_enabled_valid_gate_keeps_success_exit(self):
        self.exercise(True, False)

    def test_default_diagnostic_behavior_is_unchanged(self):
        self.exercise(False, True)

    def test_parser_exposes_opt_in_without_defaulting_it_on(self):
        for enabled in (False, True):
            argv = ["suite.py", "run", "--synth", "1"]
            if enabled:
                argv.append("--stop-on-invalid")
            with self.subTest(enabled=enabled), mock.patch("sys.argv", argv), \
                    mock.patch.object(suite, "cmd_run") as run:
                suite.main()
                self.assertEqual(run.call_args.args[0].stop_on_invalid, enabled)


if __name__ == "__main__":
    unittest.main()
