"""Hosted-only runtime prerequisite and child-permission regressions."""
import argparse
import hashlib
import json
import os
from pathlib import Path
import subprocess
import sys
import tempfile
import unittest
from unittest.mock import patch

import prerequisites
import suite
import versus


class RuntimePrerequisiteTests(unittest.TestCase):
    def test_missing_sqlite_fails_without_installing_anything(self):
        with patch.object(prerequisites.shutil, "which", side_effect=lambda name: "/usr/bin/git" if name == "git" else None), \
                patch.object(prerequisites.subprocess, "run") as run, \
                self.assertRaisesRegex(ValueError, "not only libsqlite3"):
            prerequisites.validate(require_sqlite=True)
        run.assert_not_called()

    def test_private_sqlite_is_usable_and_recorded_without_private_path(self):
        with tempfile.TemporaryDirectory() as directory:
            executable = Path(directory) / "sqlite3"
            executable.write_text("#!/bin/sh\nprintf '3.45.1 fixture-build\\n'\n")
            executable.chmod(0o755)
            with patch.dict(os.environ, {"PATH": directory + os.pathsep + os.environ["PATH"]}):
                info = prerequisites.validate(require_sqlite=True)
            self.assertEqual(info["sqlite3"], {"version": "3.45.1 fixture-build",
                             "sha256": hashlib.sha256(executable.read_bytes()).hexdigest()})
            self.assertNotIn(directory, json.dumps(info))
            self.assertEqual(info["child_umask"], "0022")

    def test_unusable_sqlite_is_not_treated_as_installed(self):
        with patch.object(prerequisites.shutil, "which", return_value=sys.executable), \
                patch.object(prerequisites.subprocess, "run", side_effect=subprocess.CalledProcessError(1, "sqlite3")), \
                self.assertRaisesRegex(ValueError, "could not report"):
            prerequisites.validate(require_sqlite=True)

    def test_missing_prerequisite_stops_before_toolchain_install_or_build(self):
        args = argparse.Namespace(toolchain="jr-fixture", synth=1, remote=False, scenarios=None)
        with patch.object(suite.prerequisites, "validate", side_effect=ValueError("missing fixture prerequisite")), \
                patch.object(suite.subprocess, "run") as run, \
                self.assertRaisesRegex(SystemExit, "missing fixture prerequisite"):
            suite.cmd_run(args)
        run.assert_not_called()

    def test_counted_child_pins_umask_without_changing_parent(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            original = os.umask(0o002)
            try:
                result = versus._run_counted([sys.executable, "-c",
                    "import os; os.mkdir('managed'); print(oct(os.stat('managed').st_mode & 0o777))"],
                    root, dict(os.environ), root / "child.log")
                self.assertEqual(result["exit"], 0)
                self.assertEqual((root / "child.log").read_text().strip(), "0o755")
                observed = os.umask(0o002)
                self.assertEqual(observed, 0o002)
            finally:
                os.umask(original)

    def test_named_runner_pins_umask_at_process_boundary(self):
        with patch.object(suite, "new_id", return_value="fixture"), \
                patch.object(suite, "foreign_load", return_value=(0, 0)), \
                patch.object(suite.subprocess, "Popen", side_effect=RuntimeError("observed process boundary")) as popen, \
                self.assertRaisesRegex(RuntimeError, "observed process boundary"):
            suite.run_once(suite.BY_NAME["desktop-body-check"], Path("/fixture"), Path("/fixture/target"))
        self.assertEqual(popen.call_args.kwargs["umask"], 0o022)

    def test_comparisons_reject_runtime_prerequisite_changes(self):
        with tempfile.TemporaryDirectory() as directory:
            paths = [Path(directory) / name for name in ("a.json", "b.json")]
            for path, value in zip(paths, (None, prerequisites.provenance())):
                path.write_text(json.dumps({"env": {"snapshots": {}, "runtime_prerequisites": value},
                                            "scenarios": {}}))
            for module in (suite, versus):
                with self.subTest(module=module.__name__), self.assertRaisesRegex(SystemExit, "different runtime"):
                    module.cmd_compare(argparse.Namespace(a=str(paths[0]), b=str(paths[1]), floor=10))


if __name__ == "__main__":
    unittest.main()
