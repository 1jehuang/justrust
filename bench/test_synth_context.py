"""Synthetic suite controls must describe the environment actually executed."""
import argparse
import contextlib
import hashlib
import io
import json
import os
from pathlib import Path
import subprocess
import sys
import tempfile
import unittest
from unittest import mock

import suite
import versus


class SyntheticContextTests(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory()
        self.addCleanup(self.temp.cleanup)
        self.root = Path(self.temp.name)
        self.home = self.root / "private-home"
        self.hints = self.root / "costs.json"
        self.hints.write_text('{"exact-package":42}\n')
        self.scenario = suite.Scenario("tiny", "unused", ["check"], None, "boundary")

    def synthetic_child(self, toolchain="nightly-test"):
        captured = []

        def run(stacks, *args, **kwargs):
            captured.append(versus.stack_env(stacks[0], self.scenario,
                                            self.root / "target", "own-run"))
            return {"stack_names": [stacks[0].name], "results": []}

        runner = mock.Mock()
        runner.run.side_effect = run
        runner.table.return_value = "synthetic fixture"
        args = argparse.Namespace(synth=1, points=40, n=1, quiet_wait=0,
                                  resume=False, out=str(self.root / "result.json"))
        with mock.patch.object(suite, "JR_HOME", self.home), \
                mock.patch.object(suite, "TOOLCHAIN", toolchain), \
                mock.patch.object(suite, "_synth", return_value=(runner, versus)), \
                mock.patch.object(suite, "env_info", return_value={"optimization_env": suite.optimization_env()}), \
                mock.patch.object(suite, "verify_binary_identity"), \
                contextlib.redirect_stdout(io.StringIO()):
            suite.cmd_run_synth(args)
        self.assertEqual(len(captured), 1)
        return captured[0], json.loads((self.root / "result.json").read_text())

    def controls(self):
        return {
            "JUSTRUST_HOME": str(self.home),
            "JUSTRUST_CARGO_FORK": "1",
            "JUSTRUST_CARGO_DAEMON": "0",
            "JUSTRUST_CARGO_CRITICAL_PATH": "1",
            "JUSTRUST_CARGO_CRITICAL_PATH_COSTS": str(self.hints),
            "JUSTRUST_SHADOW": "0",
            "CARGO_BUILD_JOBS": "24",
            "JUSTRUST_NO_UPDATE": "1",
            "JUSTRUST_TOOLCHAIN_AUTO_INSTALL": "0",
            "JUSTRUST_REMOTE": "force",
            "JUSTRUST_API_KEY": "never-forward-this",
            "JUSTRUST_UNRECOGNIZED_EXPERIMENT": "not-approved",
        }

    def test_actual_synthetic_stack_preserves_declared_controls_and_home(self):
        with mock.patch.dict(os.environ, self.controls(), clear=True):
            child, result = self.synthetic_child()
        for key, value in self.controls().items():
            if key in {"JUSTRUST_REMOTE", "JUSTRUST_API_KEY", "JUSTRUST_UNRECOGNIZED_EXPERIMENT"}:
                continue
            self.assertEqual(child.get(key), value, key)
        self.assertEqual(child["JUSTRUST_TOOLCHAIN"], "nightly-test")
        self.assertEqual(child["JUSTRUST_REMOTE"], "0")
        self.assertEqual(child["JUSTRUST_RUN_ID_OVERRIDE"], "own-run")
        self.assertNotIn("JUSTRUST_API_KEY", child)
        self.assertNotIn("JUSTRUST_UNRECOGNIZED_EXPERIMENT", child)
        public = json.dumps(result)
        self.assertNotIn(str(self.home), public)
        self.assertNotIn(str(self.hints), public)
        self.assertNotIn("never-forward-this", public)
        self.assertEqual(result["env"]["optimization_env"]["JUSTRUST_CARGO_CRITICAL_PATH_COSTS_SHA256"],
                         hashlib.sha256(self.hints.read_bytes()).hexdigest())
        self.assertNotIn("JUSTRUST_CARGO_CRITICAL_PATH_COSTS_SHA256", child)

    def test_controls_reach_a_real_exec_child(self):
        with mock.patch.dict(os.environ, self.controls(), clear=True):
            child, _ = self.synthetic_child()
        output = subprocess.check_output([sys.executable, "-c",
                                          "import json,os; print(json.dumps(dict(os.environ)))"], env=child)
        executed = json.loads(output)
        self.assertEqual(executed.get("JUSTRUST_HOME"), str(self.home))
        self.assertEqual(executed.get("JUSTRUST_CARGO_CRITICAL_PATH"), "1")
        self.assertEqual(executed.get("CARGO_BUILD_JOBS"), "24")
        self.assertEqual(executed.get("JUSTRUST_CARGO_CRITICAL_PATH_COSTS"), str(self.hints))

    def test_versus_default_keeps_home_but_does_not_inherit_optimization(self):
        with mock.patch.dict(os.environ, self.controls(), clear=True), \
                mock.patch.object(suite, "JR_HOME", self.home):
            child = versus.stack_env(versus.Stack("default", "control", True),
                                     self.scenario, self.root / "target", "run")
        self.assertEqual(child.get("JUSTRUST_HOME"), str(self.home))
        for name in ("JUSTRUST_CARGO_CRITICAL_PATH", "JUSTRUST_CARGO_FORK", "CARGO_BUILD_JOBS",
                     "JUSTRUST_CARGO_CRITICAL_PATH_COSTS", "JUSTRUST_API_KEY"):
            self.assertNotIn(name, child)

    def test_stock_has_no_justrust_context_or_global_cargo_controls(self):
        with mock.patch.dict(os.environ, self.controls(), clear=True):
            child = versus.stack_env(versus.Stack("stock", "baseline", False),
                                     self.scenario, self.root / "target", "run")
        self.assertFalse(any(key.startswith("JUSTRUST_") for key in child))
        self.assertNotIn("CARGO_BUILD_JOBS", child)
        self.assertEqual(child["CARGO_TARGET_DIR"], str(self.root / "target"))

    def test_explicit_stack_overrides_scenario_and_default_home(self):
        self.scenario.env = {"JUSTRUST_HOME": "scenario-home", "JUSTRUST_CARGO_CRITICAL_PATH": "0"}
        stack = versus.Stack("explicit", "candidate", True,
                             {"JUSTRUST_HOME": "stack-home", "JUSTRUST_CARGO_CRITICAL_PATH": "1"})
        with mock.patch.object(suite, "JR_HOME", self.home):
            child = versus.stack_env(stack, self.scenario, self.root / "target", "run")
        self.assertEqual(child["JUSTRUST_HOME"], "stack-home")
        self.assertEqual(child["JUSTRUST_CARGO_CRITICAL_PATH"], "1")

    def test_system_toolchain_does_not_leak_global_toolchain_selection(self):
        with mock.patch.dict(os.environ, {"JUSTRUST_TOOLCHAIN": "unintended-nightly"}, clear=True):
            child, _ = self.synthetic_child(toolchain="system")
        self.assertNotIn("JUSTRUST_TOOLCHAIN", child)

    def test_invalid_optional_hints_still_forward_to_cargo_fallback_without_claimed_digest(self):
        with mock.patch.dict(os.environ, {"JUSTRUST_CARGO_CRITICAL_PATH_COSTS": str(self.root / "missing")}, clear=True):
            child, result = self.synthetic_child()
        self.assertEqual(child.get("JUSTRUST_CARGO_CRITICAL_PATH_COSTS"), str(self.root / "missing"))
        self.assertNotIn("JUSTRUST_CARGO_CRITICAL_PATH_COSTS_SHA256", result["env"]["optimization_env"])

    def test_unset_controls_are_not_invented(self):
        with mock.patch.dict(os.environ, {}, clear=True):
            child, _ = self.synthetic_child()
        for name in ("JUSTRUST_CARGO_CRITICAL_PATH", "JUSTRUST_SHADOW", "CARGO_BUILD_JOBS",
                     "JUSTRUST_NO_UPDATE", "JUSTRUST_TOOLCHAIN_AUTO_INSTALL"):
            self.assertNotIn(name, child)

    def test_all_reported_controls_are_passed_including_explicit_zero(self):
        controls = {name: "0" for name in suite.OPTIMIZATION_CONTROLS}
        with mock.patch.dict(os.environ, controls, clear=True):
            child, result = self.synthetic_child()
        for name, value in controls.items():
            self.assertEqual(child[name], value, name)
            self.assertEqual(result["env"]["optimization_env"][name], value, name)


if __name__ == "__main__":
    unittest.main()
