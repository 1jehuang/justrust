"""Regression coverage for the clean release gate's snapshot shell prerequisite."""
import importlib.util
from pathlib import Path
import shutil
import subprocess
import unittest
from unittest import mock

ROOT = Path(__file__).resolve().parents[1]
spec = importlib.util.spec_from_file_location("shell_check", ROOT / "ci/check-shell-environment.py")
shell_check = importlib.util.module_from_spec(spec)
spec.loader.exec_module(shell_check)


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
        workflow = (ROOT / ".github/workflows/source-sync-release-gate.yml").read_text()
        self.assertIn("sudo ln -sf /bin/bash /bin/sh", workflow)
        self.assertIn("python3 ci/check-shell-environment.py", workflow)


if __name__ == "__main__":
    unittest.main()
