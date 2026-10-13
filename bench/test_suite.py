#!/usr/bin/env python3
"""Safety checks for hosted benchmark accounting (no network or builds)."""
import argparse
import hashlib
import json
from pathlib import Path
import tempfile
import unittest
from unittest.mock import patch

import suite


class HostedMeasurements(unittest.TestCase):
    def test_hosted_early_requires_real_shown_confirmed_nonfinal_diagnostics(self):
        good = {"ready": True, "shown": True, "matched": 1, "early_only": 0, "exited": False}
        self.assertEqual(suite.hosted_early_provenance({"early": good}), good)
        for change in ({"ready": False}, {"shown": False}, {"matched": 0},
                       {"early_only": 1}, {"exited": True}):
            with self.subTest(change=change), self.assertRaises(ValueError):
                suite.hosted_early_provenance({"early": {**good, **change}})
        with self.assertRaises(ValueError):
            suite.hosted_early_provenance({})

    def test_hosted_early_rejects_local_startup_and_ignored_flags_before_install(self):
        for remote, names, requested in ((True, ["desktop-type-error-early"], False),
                                       (False, ["desktop-type-error"], True),
                                       (True, ["desktop-body-check"], True),
                                       (True, ["missing"], True), (True, None, True)):
            args = argparse.Namespace(toolchain="nightly-2026-10-08", remote=remote, synth=None,
                                      scenarios=names, hosted_early=requested)
            with self.subTest(args=args), patch.object(suite.subprocess, "run") as run, \
                    self.assertRaises(SystemExit):
                suite.cmd_run(args)
            run.assert_not_called()

    def test_explicit_binary_alias_is_absolute_and_never_ignored(self):
        with tempfile.TemporaryDirectory() as tmp:
            binary = Path(tmp) / "justrust"
            binary.write_text("#!/bin/sh\nexit 0\n")
            binary.chmod(0o755)
            for key in ("JUSTRUST", "JUSTRUST_BIN"):
                self.assertEqual(suite.resolve_justrust_binary({key: str(binary)}), str(binary.resolve()))
            alias = Path(tmp) / "alias"
            alias.symlink_to(binary)
            self.assertEqual(suite.resolve_justrust_binary({"JUSTRUST": str(binary),
                                                           "JUSTRUST_BIN": str(alias)}), str(binary.resolve()))

    def test_invalid_or_conflicting_binary_overrides_fail_before_install(self):
        with tempfile.TemporaryDirectory() as tmp:
            binaries = [Path(tmp) / name for name in ("a", "b")]
            for binary in binaries:
                binary.write_text("#!/bin/sh\nexit 0\n")
                binary.chmod(0o755)
            for env in ({"JUSTRUST_BIN": ""}, {"JUSTRUST": str(Path(tmp) / "missing")},
                        {"JUSTRUST": str(binaries[0]), "JUSTRUST_BIN": str(binaries[1])}):
                with self.subTest(env=env), patch.object(suite.subprocess, "run") as run, \
                        self.assertRaises(ValueError):
                    suite.resolve_justrust_binary(env)
                run.assert_not_called()
            binaries[0].chmod(0o644)
            with self.assertRaises(ValueError):
                suite.resolve_justrust_binary({"JUSTRUST_BIN": str(binaries[0])})

    def test_atomic_binary_replacement_rejects_mixed_results(self):
        with tempfile.TemporaryDirectory() as tmp:
            binary, replacement = (Path(tmp) / name for name in ("justrust", "replacement"))
            binary.write_bytes(b"first executable")
            with patch.object(suite, "J", str(binary)):
                info = {"justrust_binary_sha256": suite.binary_sha256()}
                self.assertEqual(info["justrust_binary_sha256"], hashlib.sha256(binary.read_bytes()).hexdigest())
                suite.verify_binary_identity(info)
                replacement.write_bytes(b"second executable")
                replacement.replace(binary)
                with self.assertRaisesRegex(ValueError, "changed during the benchmark"):
                    suite.verify_binary_identity(info)
                binary.unlink()
                with self.assertRaisesRegex(ValueError, "cannot identify executable"):
                    suite.binary_sha256()

    def test_binary_revision_is_not_invented_from_repository_head(self):
        with tempfile.TemporaryDirectory() as tmp:
            binary = Path(tmp) / "justrust"
            binary.write_bytes(b"fixture executable")
            with patch.object(suite, "J", str(binary)), patch.object(suite, "sh", return_value="repo-rev"), \
                    patch.dict(suite.os.environ, {}, clear=True):
                info = suite.env_info()
                self.assertEqual(info["benchmark_repo_rev"], "repo-rev")
                self.assertIsNone(info["justrust_binary_revision"])
                self.assertEqual(info["justrust_revision_source"], "repository HEAD, not binary revision")
                with patch.dict(suite.os.environ, {"JUSTRUST_BENCH_REV": "binary-rev"}):
                    declared = suite.env_info()
                self.assertEqual(declared["justrust_binary_revision"], "binary-rev")
                self.assertEqual(declared["benchmark_repo_rev"], "repo-rev")
                self.assertEqual(declared["justrust_binary_sha256"], info["justrust_binary_sha256"])

    def test_changed_binary_never_writes_a_completed_suite_result(self):
        with tempfile.TemporaryDirectory() as tmp:
            binary, replacement, out = (Path(tmp) / name for name in ("binary", "replacement", "result.json"))
            binary.write_bytes(b"first")
            info = {"justrust_binary_sha256": suite.binary_sha256(str(binary))}
            def replace_during_sample(*args):
                replacement.write_bytes(b"second")
                replacement.replace(binary)
                return {}
            args = argparse.Namespace(toolchain="system", remote=False, synth=None, cold=False,
                                      scenarios=["desktop-noop-test"], n=1, warmup=0,
                                      quiet_wait=0, out=str(out))
            with patch.object(suite, "J", str(binary)), patch.object(suite, "env_info", return_value=info), \
                    patch.object(suite, "run_scenario", side_effect=replace_during_sample), \
                    self.assertRaisesRegex(ValueError, "changed during the benchmark"):
                suite.cmd_run(args)
            self.assertFalse(out.exists())

    def test_requested_pool_fallback_is_not_a_pooled_measurement(self):
        for text in ("", "justrust hosted: HTTP pool unavailable, using curl", "justrust hosted: HTTP pool bypassed for custom proxy/certificate configuration"):
            with self.subTest(text=text), self.assertRaises(ValueError):
                suite.admission_transport(text, True)
        self.assertEqual(suite.admission_transport("justrust trace: pooled hosted admission", True), "pooled")
        self.assertEqual(suite.admission_transport("", False), "curl")

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

    def test_compiler_config_is_saved_before_runtime_filter_without_mutation(self):
        original = suite.BY_NAME["desktop-test-edit"]
        command = list(original.cmd)
        configured = suite.compiler_scenario(original, 8, True)
        self.assertEqual(original.cmd, command)
        self.assertIn('build.rustflags=["-Zthreads=8"]', configured.cmd)
        self.assertIn('env.JUSTRUST_PASSES="on"', configured.cmd)
        self.assertLess(configured.cmd.index("--config"), configured.cmd.index("--"))
        self.assertEqual(configured.cmd[-1], "fps_counter")
        self.assertIs(suite.compiler_scenario(original, None, False), original)

    def test_workspace_threads_force_existing_wrapper_only_before_runtime_filter(self):
        original = suite.BY_NAME["desktop-test-edit"]
        command = list(original.cmd)
        configured = suite.compiler_scenario(original, None, False, 6)
        self.assertEqual(original.cmd, command)
        self.assertIn('env.JCODE_FRONTEND_THREADS.value="6"', configured.cmd)
        self.assertIn("env.JCODE_FRONTEND_THREADS.force=true", configured.cmd)
        self.assertFalse(any("build.rustflags" in arg for arg in configured.cmd))
        self.assertLess(configured.cmd.index("env.JCODE_FRONTEND_THREADS.force=true"),
                        configured.cmd.index("--"))
        self.assertEqual(configured.cmd[-1], "fps_counter")
        with self.assertRaises(ValueError):
            suite.compiler_scenario(original, 8, False, 6)

    def test_workspace_thread_provenance_rejects_silent_wrapper_fallback(self):
        unit = {"local": True, "name": "jcode_desktop_ui (test)", "frontend_threads": 6}
        self.assertEqual(suite.workspace_thread_provenance({"top_units": [unit]}, 6), 6)
        for observed in (None, 8):
            with self.subTest(observed=observed), self.assertRaises(ValueError):
                suite.workspace_thread_provenance(
                    {"top_units": [dict(unit, frontend_threads=observed)]}, 6)
        self.assertIsNone(suite.workspace_thread_provenance({"top_units": []}, 6))
        self.assertIsNone(suite.workspace_thread_provenance(
            {"top_units": [dict(unit, local=False, frontend_threads=None)]}, 6))

    def test_workspace_thread_options_reject_ignored_controls_before_install(self):
        for scenarios, toolchain, global_threads in [
                (["desktop-test-edit"], "system", None),
                (["desktop-test-edit"], "jr-b7907f1fe8f7", 8),
                (["justrust-body-test"], "jr-b7907f1fe8f7", None),
                (None, "jr-b7907f1fe8f7", None),
                (["missing"], "jr-b7907f1fe8f7", None)]:
            args = argparse.Namespace(toolchain=toolchain, remote=True, synth=None,
                                      rustc_threads=global_threads, workspace_threads=6,
                                      pass_times=False, scenarios=scenarios)
            with self.subTest(args=args), patch.object(suite.subprocess, "run") as run, \
                    self.assertRaises(SystemExit):
                suite.cmd_run(args)
            run.assert_not_called()

    def test_thread_options_require_nightly_before_any_build_or_install(self):
        args = argparse.Namespace(toolchain="system", remote=True, synth=None, rustc_threads=8, pass_times=True)
        with patch.object(suite.subprocess, "run") as run, self.assertRaises(SystemExit):
            suite.cmd_run(args)
        run.assert_not_called()

    def test_different_source_pins_never_produce_speed_comparison(self):
        with tempfile.TemporaryDirectory() as tmp:
            paths = [Path(tmp) / "a.json", Path(tmp) / "b.json"]
            for path, pin in zip(paths, ("old", "new")):
                path.write_text(json.dumps({"env": {"snapshots": {"desktop": pin}}, "scenarios": {}}))
            with self.assertRaises(SystemExit) as err:
                suite.cmd_compare(argparse.Namespace(a=str(paths[0]), b=str(paths[1]), floor=10))
            self.assertIn("not comparable", str(err.exception))

    def test_optimization_provenance_excludes_secrets(self):
        env = {"JUSTRUST_TEST_FILTER_COMPILE": "1", "JUSTRUST_CARGO_DAEMON": "0",
               "JCODE_API_KEY": "secret", "AWS_SECRET_ACCESS_KEY": "secret"}
        with patch.dict(suite.os.environ, env, clear=True):
            self.assertEqual(suite.optimization_env(), {
                "JUSTRUST_TEST_FILTER_COMPILE": "1", "JUSTRUST_CARGO_DAEMON": "0"})
        with patch.dict(suite.os.environ, {}, clear=True):
            self.assertEqual(suite.optimization_env(), {})

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
        suite.RUSTC_THREADS = None
        suite.WORKSPACE_THREADS = None
        suite.PASS_TIMES = False


if __name__ == "__main__":
    unittest.main()
