import copy
import importlib.util
from pathlib import Path
import sys
import unittest
from unittest.mock import patch

import suite


spec = importlib.util.spec_from_file_location(
    "cleanroom_test_profile", Path(__file__).with_name("cleanroom-test-profile.py")
)
profile = importlib.util.module_from_spec(spec)
spec.loader.exec_module(profile)


class CleanroomProfileTests(unittest.TestCase):
    def setUp(self):
        snapshots = patch.dict(suite.OPTIONAL_SNAPSHOTS, copy.deepcopy(suite.OPTIONAL_SNAPSHOTS))
        snapshots.start()
        self.addCleanup(snapshots.stop)

    def test_profiles_change_only_debug_info(self):
        with patch.object(suite, "SCENARIOS", []), patch.object(suite, "BY_NAME", {}):
            scenarios = []
            for level in (0, 1, 2):
                profile.configure(level)
                scenarios.append(suite.BY_NAME[profile.NAME])
            for level, scenario in enumerate(scenarios):
                self.assertIn(f"profile.dev.debug={level}", scenario.cmd)
                self.assertEqual(scenario.edit, scenarios[0].edit)
                rendered = scenario.edit.template.format(n=42)
                self.assertIn("fn latency_probe() {", rendered)
                self.assertIn("black_box(42u64), 42u64", rendered)
                self.assertNotIn("opt-level=0", scenario.cmd)

    def test_entrypoint_always_selects_the_scoped_scenario(self):
        with patch.object(sys, "argv", ["profile", "--debug-info", "1", "run", "--remote", "-n", "3"]), \
                patch.object(profile, "configure") as configure, \
                patch.object(suite, "main") as main:
            profile.main()
            configure.assert_called_once_with(1, None)
            main.assert_called_once_with()
            self.assertEqual(sys.argv[1:], ["run", profile.NAME, "--remote", "-n", "3"])

    def test_lto_override_is_scoped_to_test_profile(self):
        with patch.object(suite, "SCENARIOS", []), patch.object(suite, "BY_NAME", {}):
            profile.configure(2, "off")
            scenario = suite.BY_NAME[profile.NAME]
            self.assertEqual(scenario.cmd[-2:], ["--config", 'profile.test.lto="off"'])
            self.assertIn("profile.dev.debug=2", scenario.cmd)
            self.assertNotIn("profile.release", " ".join(scenario.cmd))

    def test_rejects_unrelated_commands(self):
        with patch.object(sys, "argv", ["profile", "--debug-info", "1", "list"]), \
                patch.object(profile, "configure"), patch.object(suite, "main") as main:
            with self.assertRaises(SystemExit):
                profile.main()
            main.assert_not_called()


if __name__ == "__main__":
    unittest.main()
