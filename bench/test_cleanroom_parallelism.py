import importlib.util
from pathlib import Path
import sys
import unittest
from unittest.mock import patch

import suite

spec = importlib.util.spec_from_file_location(
    "cleanroom_parallelism", Path(__file__).with_name("cleanroom-parallelism.py")
)
profile = importlib.util.module_from_spec(spec)
spec.loader.exec_module(profile)


class CleanroomParallelismTests(unittest.TestCase):
    def test_changes_only_incremental_control(self):
        with patch.object(suite, "SCENARIOS", []), patch.object(suite, "BY_NAME", {}), \
                patch.dict(suite.OPTIONAL_SNAPSHOTS, {"rewrite": {"repos": {}}}):
            scenarios = []
            for incremental in (False, True):
                profile.configure(incremental)
                scenarios.append(suite.BY_NAME[profile.NAME])
            self.assertEqual(scenarios[0].edit, scenarios[1].edit)
            self.assertEqual(scenarios[0].cmd[:-1], scenarios[1].cmd[:-1])
            self.assertEqual(scenarios[0].cmd[-1], "profile.dev.incremental=false")
            self.assertEqual(scenarios[1].cmd[-1], "profile.dev.incremental=true")
            self.assertIn("const _: u64 = 42;", scenarios[0].edit.template.format(n=42))

    def test_entrypoint_selects_only_diagnostic(self):
        with patch.object(sys, "argv", ["profile", "--incremental", "off", "run", "--remote", "-n", "3"]), \
                patch.object(profile, "configure") as configure, patch.object(suite, "main") as main:
            profile.main()
            configure.assert_called_once_with(False)
            main.assert_called_once_with()
            self.assertEqual(sys.argv[1:], ["run", profile.NAME, "--remote", "-n", "3"])

    def test_rejects_unrelated_commands(self):
        with patch.object(sys, "argv", ["profile", "list"]), \
                patch.object(profile, "configure"), patch.object(suite, "main") as main:
            with self.assertRaises(SystemExit):
                profile.main()
            main.assert_not_called()


if __name__ == "__main__":
    unittest.main()
