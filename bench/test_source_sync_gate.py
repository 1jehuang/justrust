"""Regression coverage for the clean release gate's snapshot shell prerequisite."""
import importlib.util
from pathlib import Path
import shutil
import subprocess
import tempfile
import unittest
from unittest import mock

ROOT = Path(__file__).resolve().parents[1]
spec = importlib.util.spec_from_file_location("shell_check", ROOT / "ci/check-shell-environment.py")
shell_check = importlib.util.module_from_spec(spec)
spec.loader.exec_module(shell_check)
runtime_spec = importlib.util.spec_from_file_location("pinned_runtime_check", ROOT / "ci/check-pinned-runtime.py")
runtime_check = importlib.util.module_from_spec(runtime_spec)
runtime_spec.loader.exec_module(runtime_check)


class PinnedRuntimePrerequisiteTests(unittest.TestCase):
    def exercise(self, exit_code, fail_at=0):
        with tempfile.TemporaryDirectory() as temporary:
            work = Path(temporary)
            (work / "evidence").mkdir()
            target = work / "runtime-preflight-target"
            target.mkdir()

            def run(argv, cwd, env, log):
                expected = [
                    ["test", "-p", "jcode-provider-bedrock", "--lib"],
                    ["test", "-p", "jcode-provider-bedrock", "--lib"],
                    ["test", "-p", "jcode-provider-openai-runtime", "--lib"],
                    ["test", "-p", "jcode-provider-openai-runtime", "--lib"],
                    ["test", "--workspace"],
                    ["test", "--workspace"],
                ]
                index = int(log.stem.rsplit("-", 1)[1])
                self.assertEqual(argv[1:], expected[index])
                self.assertEqual(cwd, work / "snapshot" / ("jcode-desktop" if index == 5 else "jcode"))
                self.assertFalse(any("test-threads" in arg for arg in argv))
                self.assertEqual(env["CARGO_TARGET_DIR"], str(target))
                self.assertEqual(env["JUSTRUST_LOCAL"], "1")
                self.assertNotIn("JUSTRUST_REMOTE_REQUIRED", env)
                log.write_text("pinned test diagnostics\n")
                return {"exit": exit_code if int(log.stem.rsplit("-", 1)[1]) == fail_at else 0}

            with mock.patch.dict(runtime_check.os.environ, {}, clear=True), \
                    mock.patch.object(runtime_check.sys, "argv", ["check", "/candidate", str(work)]), \
                    mock.patch.object(runtime_check.suite, "prepare", return_value=work / "snapshot"), \
                    mock.patch.object(runtime_check.versus, "run_counted", side_effect=run) as execute:
                if exit_code:
                    with self.assertRaises(SystemExit) as failure:
                        runtime_check.main()
                    self.assertEqual(failure.exception.code, exit_code)
                    self.assertEqual(execute.call_count, fail_at + 1)
                    self.assertTrue(target.exists())
                else:
                    runtime_check.main()
                    self.assertEqual(execute.call_count, 6)
                    self.assertFalse(target.exists())

    def test_success_runs_whole_provider_binaries_and_workspace_before_comparison(self):
        self.exercise(0)

    def test_failure_blocks_measurements_and_preserves_diagnostics(self):
        for fail_at in range(6):
            with self.subTest(fail_at=fail_at):
                self.exercise(101, fail_at)


class ShellPrerequisiteTests(unittest.TestCase):
    def test_bash_retains_nonidentifier_environment_without_bin_path_injection(self):
        self.assertEqual(shell_check.check(shutil.which("bash")), str(Path(shutil.which("bash")).resolve()))

    @unittest.skipUnless(shutil.which("dash"), "dash is not installed")
    def test_dash_is_rejected_before_probe_or_measurement(self):
        with self.assertRaisesRegex(RuntimeError, "do not inject binary paths"):
            shell_check.check(shutil.which("dash"))

    def test_environment_loss_fails_closed(self):
        with mock.patch.object(shell_check.subprocess, "run", return_value=
                               subprocess.CompletedProcess([], 1, "", "")) as run:
            with self.assertRaisesRegex(RuntimeError, "hyphenated binary-path variables"):
                shell_check.check()
        env = run.call_args.kwargs["env"]
        self.assertEqual(env["JUSTRUST-shell-environment-probe"], "retained")
        self.assertFalse(any(k.startswith("CARGO_BIN_EXE_") for k in env
                             if k not in shell_check.os.environ))

    def test_gate_checks_shell_before_builds_and_preserves_full_workload(self):
        helper = (ROOT / "ci/source-sync-release-gate.sh").read_text()
        self.assertLess(helper.index('python3 "$root/ci/check-shell-environment.py"'),
                        helper.index('build_binary before "$BASELINE_REF"'))
        self.assertIn("run --synth 1 --points 40 -n 1 --stop-on-invalid", helper)
        self.assertLess(helper.index('python3 "$root/ci/check-pinned-runtime.py"'),
                        helper.index('  benchmark_binary before "$BASELINE_REF"'))
        self.assertIn("JUSTRUST_BENCH_RUNTIME_CONTEXT=jcode-isolated-v4", helper)
        workflow = (ROOT / ".github/workflows/source-sync-release-gate.yml").read_text()
        self.assertIn("sudo ln -sf /bin/bash /bin/sh", workflow)
        self.assertIn("python3 ci/check-shell-environment.py", workflow)


if __name__ == "__main__":
    unittest.main()
