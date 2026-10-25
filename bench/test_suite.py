#!/usr/bin/env python3
"""Safety checks for hosted benchmark accounting (no network or builds)."""
import argparse
import contextlib
import hashlib
import io
import json
from pathlib import Path
import tempfile
import unittest
from unittest.mock import patch

import suite


class HostedMeasurements(unittest.TestCase):
    def test_compare_never_claims_speed_from_noisy_or_unknown_samples(self):
        for noisy_a, noisy_b in [(True, False), (False, True), (None, False),
                                 (False, None), (False, False)]:
            with self.subTest(noisy_a=noisy_a, noisy_b=noisy_b), tempfile.TemporaryDirectory() as tmp:
                paths = []
                for name, wall, noisy in [('a', 5.0, noisy_a), ('b', 1.0, noisy_b)]:
                    sample = {'wall': wall, 'recorded_wall': wall - .1}
                    if noisy is not None:
                        sample['noisy'] = noisy
                    result = {'env': {'snapshots': {}, 'justrust_rev': 'test'},
                              'scenarios': {'edit': {'samples': [sample], 'stats': {
                                  'median': wall, 'min': wall, 'max': wall}}}}
                    path = Path(tmp) / (name + '.json')
                    path.write_text(json.dumps(result))
                    paths.append(str(path))
                output = io.StringIO()
                with contextlib.redirect_stdout(output):
                    suite.cmd_compare(argparse.Namespace(a=paths[0], b=paths[1], floor=10))
                if noisy_a is False and noisy_b is False:
                    self.assertIn('faster', output.getvalue())
                    self.assertNotIn('uncontrolled', output.getvalue())
                else:
                    self.assertIn('uncontrolled', output.getvalue())
                    self.assertNotIn('faster', output.getvalue())

    def test_shadow_control_provenance_never_includes_internal_or_secret_environment(self):
        with patch.dict(suite.os.environ, {"JUSTRUST_SHADOW": "1",
                                          "JUSTRUST_SHADOW_CHILD": "1",
                                          "CACHE_PUBLISH_TOKEN": "secret"}, clear=True):
            self.assertEqual(suite.optimization_env(), {"JUSTRUST_SHADOW": "1"})

    def test_scheduler_control_provenance_omits_hint_paths_and_credentials(self):
        with patch.dict(suite.os.environ, {"JUSTRUST_CARGO_CRITICAL_PATH": "1",
                                          "JUSTRUST_CARGO_CRITICAL_PATH_COSTS": "/private/project/costs.json",
                                          "CACHE_PUBLISH_TOKEN": "secret"}, clear=True):
            self.assertEqual(suite.optimization_env(), {"JUSTRUST_CARGO_CRITICAL_PATH": "1"})

    def test_workspace_cache_controls_recorded_without_machine_local_paths(self):
        controls = {"JUSTRUST_LOCALCACHE": "1", "JUSTRUST_LOCALCACHE_TARGET_BOUND": "1"}
        with patch.dict(suite.os.environ, {**controls,
                                          "JUSTRUST_LOCALCACHE_DIR": "/private/cache",
                                          "CACHE_PUBLISH_TOKEN": "secret"}, clear=True):
            self.assertEqual(suite.optimization_env(), controls)
            self.assertEqual(suite.synthetic_stack_controls(), controls)

    PHASE_LINE = ("justrust trace: admission identity 54.0 ms | charge 225.0 ms | "
                  "identity_wait 0.0 ms | queue 67.0 ms | total 292.0 ms "
                  "(awaited paths, identity overlaps)")

    def test_remote_trace_retains_actual_client_and_overlapping_admission_phases(self):
        text = "\n".join(("justrust trace:     0.1 ms roots", self.PHASE_LINE,
                          "justrust trace:   413.4 ms charged", "justrust trace:   700.2 ms flushed",
                          "justrust trace:   844.8 ms first output", "justrust trace:   900.0 ms exit",
                          "justrust trace:   900.1 ms recording"))
        self.assertEqual(suite.remote_trace_timings(text), {
            "client_elapsed_ms": {"roots": 0.1, "charged": 413.4, "flushed": 700.2,
                                  "first_output": 844.8, "exit": 900.0, "recording": 900.1},
            "admission_awaited_ms": {"identity": 54.0, "charge": 225.0, "identity_wait": 0.0,
                                     "queue": 67.0, "total": 292.0},
        })

    def test_remote_trace_never_retains_arbitrary_text_or_invalid_numeric_fields(self):
        for line in ("", "Authorization: Bearer secret", "justrust trace: 1.0 ms secret",
                     "justrust trace: NaN ms charged", "justrust trace: inf ms charged",
                     "justrust trace: -1 ms charged", "justrust trace: 86400001 ms charged",
                     "justrust trace: 1 ms charged credential=secret", "x" * 513,
                     self.PHASE_LINE.replace("54.0", "60000.1"),
                     self.PHASE_LINE.replace("54.0", "1e2"),
                     self.PHASE_LINE.replace("identity", "account_id", 1),
                     self.PHASE_LINE + " credential=secret"):
            with self.subTest(line=line):
                self.assertEqual(suite.remote_trace_timings(line), {})

    def test_remote_trace_omits_duplicate_or_nonmonotonic_measurements(self):
        self.assertEqual(suite.remote_trace_timings(self.PHASE_LINE + "\n" + self.PHASE_LINE), {})
        self.assertEqual(suite.remote_trace_timings("justrust trace: 5 ms charged\n"
                                                  "justrust trace: 6 ms charged"), {})
        self.assertEqual(suite.remote_trace_timings("justrust trace: 5 ms roots\n"
                                                  "justrust trace: 4 ms charged"), {})
        self.assertEqual(suite.remote_trace_timings("justrust trace: 0 ms roots"),
                         {"client_elapsed_ms": {"roots": 0.0}})

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

    def test_remote_source_revision_fallback_is_not_a_binary_revision(self):
        with tempfile.TemporaryDirectory() as tmp:
            binary = Path(tmp) / "justrust"
            binary.write_bytes(b"fixture executable")
            def no_git(command):
                if command[0] == "git":
                    raise RuntimeError("remote sync has no .git")
                return "fixture version"
            with patch.object(suite, "J", str(binary)), patch.object(suite, "sh", side_effect=no_git), \
                    patch.dict(suite.os.environ, {"JUSTRUST_BENCH_REPO_REV": "source-only"}, clear=True):
                info = suite.env_info()
            self.assertEqual(info["benchmark_repo_rev"], "source-only")
            self.assertEqual(info["justrust_rev"], "source-only")
            self.assertIsNone(info["justrust_binary_revision"])
            self.assertEqual(info["justrust_revision_source"], "repository HEAD, not binary revision")

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


class NativeFrontendMeasurements(unittest.TestCase):
    def args(self, root):
        import native_frontend
        parser = argparse.ArgumentParser()
        sub = parser.add_subparsers()
        native_frontend.add_parser(sub)
        return parser.parse_args(['native-frontend', '--binary', str(root / 'compiler'),
                                  '--source', str(root / 'project/src/lib.rs'), '--lib',
                                  '--input-root', str(root / 'project'), '--scratch', str(root),
                                  '-n', '3', '-o', str(root / 'result.json')])

    def fixture(self, root):
        (root / 'project/src').mkdir(parents=True)
        (root / 'project/src/lib.rs').write_text('mod dependency;')
        (root / 'project/src/dependency.rs').write_text('pub fn value() {}')
        (root / 'project/Cargo.toml').write_text('[package]\nname="p"\n')
        (root / 'compiler').write_text('pinned binary fixture')

    def collect(self, root, mutate=None, failed=False):
        import native_frontend
        commands = []
        def invoke(cmd, **kwargs):
            commands.append(cmd)
            self.assertEqual(kwargs['env'], {'PATH': '/usr/bin:/bin', 'LANG': 'C',
                                             'LC_ALL': 'C', 'JR_OPT': 'off'})
            if mutate:
                mutate()
            if failed:
                return native_frontend.subprocess.CompletedProcess(cmd, 1, b'', b'unsupported')
            mode = 'check' if '--check' in cmd else 'build'
            if mode == 'build':
                output = Path(cmd[cmd.index('-o') + 1])
                self.assertFalse(output.exists())
                output.write_bytes(b'artifact')
            summary = {'mode': mode, 'workers': int(cmd[cmd.index('--workers') + 1]),
                       'failures': 0, 'capture_complete': True, 'events_dropped': 0}
            return native_frontend.subprocess.CompletedProcess(cmd, 0, json.dumps(summary).encode(), b'')
        with patch.object(native_frontend.subprocess, 'run', side_effect=invoke), contextlib.redirect_stdout(io.StringIO()):
            native_frontend.run(self.args(root))
        return json.loads((root / 'result.json').read_text()), commands

    def test_all_cells_warmup_fresh_outputs_and_dependency_identity(self):
        import native_frontend
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            self.fixture(root)
            result, commands = self.collect(root)
            self.assertEqual(len(commands), 16)
            self.assertEqual(len(result['cells']), 4)
            self.assertIn(str(root / 'project/src/dependency.rs'), result['contract']['inputs'])
            self.assertIn(str(root / 'project/Cargo.toml'), result['contract']['inputs'])
            self.assertEqual(result['binary_sha256'], native_frontend.digest(root / 'compiler'))
            self.assertIn('uncontrolled', native_frontend.compare(result, result))
            with self.assertRaisesRegex(ValueError, 'overwrite'):
                native_frontend.run(self.args(root))

    def test_changes_during_run_abort_without_result(self):
        for filename in ('compiler', 'project/src/dependency.rs'):
            with self.subTest(filename=filename), tempfile.TemporaryDirectory() as tmp:
                root = Path(tmp)
                self.fixture(root)
                with self.assertRaisesRegex(ValueError, 'identity changed'):
                    self.collect(root, mutate=lambda: (root / filename).write_text('changed'))
                self.assertFalse((root / 'result.json').exists())

    def test_comparison_refuses_changed_contract_missing_cells_or_failed_warmups(self):
        import copy
        import native_frontend
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            self.fixture(root)
            result, _ = self.collect(root)
            for key in ('inputs', 'opt', 'edition', 'execution', 'host', 'environment', 'consumer_extern'):
                altered = copy.deepcopy(result)
                altered['contract'][key] = 'different'
                with self.subTest(key=key), self.assertRaisesRegex(ValueError, 'incompatible'):
                    native_frontend.compare(result, altered)
            altered = copy.deepcopy(result)
            altered['cells'].pop()
            with self.assertRaisesRegex(ValueError, 'cells'):
                native_frontend.compare(result, altered)
            for section in ('samples', 'warmups'):
                altered = copy.deepcopy(result)
                altered['cells'][0][section][0]['ok'] = False
                with self.assertRaisesRegex(ValueError, 'failed or incomplete'):
                    native_frontend.compare(result, altered)
            with self.assertRaisesRegex(ValueError, 'native frontend results only'):
                native_frontend.compare(result, {'synth': {}})

    def test_failed_native_compile_is_retained_not_speed_evidence(self):
        import native_frontend
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            self.fixture(root)
            result, _ = self.collect(root, failed=True)
            self.assertEqual(result['cells'][0]['samples'][0]['stderr'], 'unsupported')
            with self.assertRaisesRegex(ValueError, 'failed or incomplete'):
                native_frontend.compare(result, result)

    def test_consumer_extern_is_not_added_to_library_itself(self):
        import native_frontend
        args = argparse.Namespace(edition='2021', opt='off', debug_assertions='yes',
                                  cfg=['feature="std"'], extern=[], consumer_extern=['adler2=/p/lib.rs'])
        library = native_frontend.command('/compiler', '/p/lib.rs', True, 1, 'check', '/out', args)
        consumer = native_frontend.command('/compiler', '/consumer.rs', False, 8, 'build', '/out', args, consumer=True)
        self.assertNotIn('--extern', library)
        self.assertIn('adler2=/p/lib.rs', consumer)
        self.assertNotIn('--lib', consumer)


if __name__ == "__main__":
    unittest.main()
