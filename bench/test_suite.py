#!/usr/bin/env python3
"""Safety checks for hosted benchmark accounting (no network or builds)."""
import argparse
from pathlib import Path
import tempfile
import unittest
from unittest.mock import patch

import suite


class HostedMeasurements(unittest.TestCase):
    def test_remote_sample_reports_transport_and_host_separately(self):
        sample = suite.remote_measurement({"wall": 2.0, "remote": {
            "machine": "hosted", "client_wall": 2.8, "sync_ms": 120.0,
        }})
        self.assertEqual(sample["host_wall"], 2.0)
        self.assertEqual(sample["client_wall"], 2.8)
        self.assertEqual(sample["sync_ms"], 120.0)
        self.assertEqual(sample["target_state"], "persistent remote target")

    def test_silent_local_fallback_is_never_a_remote_measurement(self):
        for remote in (None, {}, {"client_wall": 1}, "hosted"):
            with self.subTest(remote=remote), self.assertRaises(ValueError):
                suite.remote_measurement({"wall": 1.0, "remote": remote})

    def test_remote_compiler_fallback_is_not_an_optimized_measurement(self):
        expected = "jr-b7907f1fe8f7"
        with patch.object(suite, "TOOLCHAIN", expected):
            for actual in (None, "system", "nightly-2026-10-08", "jr-1234567"):
                with self.subTest(actual=actual), self.assertRaises(ValueError):
                    suite.remote_measurement({"wall": 1.0, "toolchain": actual,
                                              "remote": {"machine": "hosted"}})
            sample = suite.remote_measurement({"wall": 1.0, "toolchain": expected,
                                              "remote": {"machine": "hosted"}})
            self.assertEqual(sample["toolchain"], expected)

    def test_environment_finds_staged_compiler_and_prefers_activated(self):
        with tempfile.TemporaryDirectory() as tmp, patch.object(suite, "JR_HOME", Path(tmp)):
            spec = "jr-b7907f1fe8f7"
            staged = Path(tmp) / "toolchains" / "staged" / spec / "bin" / "rustc"
            staged.parent.mkdir(parents=True)
            staged.touch()
            self.assertEqual(suite.toolchain_rustc(spec), staged)
            active = Path(tmp) / "toolchains" / spec / "bin" / "rustc"
            active.parent.mkdir(parents=True)
            active.touch()
            self.assertEqual(suite.toolchain_rustc(spec), active)

    def test_local_target_reset_is_not_a_cold_hosted_build(self):
        args = argparse.Namespace(toolchain="system", remote=True, synth=None,
                                  scenarios=["desktop-cold-check"], cold=False)
        with patch.object(suite, "prepare") as prepare, self.assertRaises(SystemExit) as err:
            suite.cmd_run(args)
        self.assertIn("not a cold hosted build", str(err.exception))
        prepare.assert_not_called()

    def test_remote_synthetic_mode_requires_the_existing_versus_harness(self):
        args = argparse.Namespace(toolchain="system", remote=True, synth=1)
        with self.assertRaises(SystemExit) as err:
            suite.cmd_run(args)
        self.assertIn("versus.py", str(err.exception))

    def tearDown(self):
        suite.REMOTE = False
        suite.TOOLCHAIN = "system"


if __name__ == "__main__":
    unittest.main()
