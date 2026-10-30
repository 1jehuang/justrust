"""Small harness-only tests. Never starts rustc, Cargo or justrust."""
import contextlib
import importlib.util
import io
import json
from pathlib import Path
import subprocess
import sys
import tempfile
import unittest
from unittest.mock import patch

SPEC = importlib.util.spec_from_file_location("reuse_lint_accept", Path(__file__).with_name("reuse-lint-accept.py"))
harness = importlib.util.module_from_spec(SPEC)
sys.modules[SPEC.name] = harness
SPEC.loader.exec_module(harness)


class HarnessTests(unittest.TestCase):
    def test_complete_diagnostics_retained(self):
        diag = {"code": {"code": "unused_variables"}, "level": "warning",
                "children": [{"message": "suggestion"}], "spans": [{"byte_start": 1}],
                "rendered": "warning at main.rs"}
        self.assertEqual(harness.diagnostics(json.dumps(diag) + "\n"), [diag])
        with self.assertRaises(json.JSONDecodeError):
            harness.diagnostics("unknown unstable option")

    def test_expected_status_and_codes(self):
        step = harness.Step("deny", "", ("unused_variables",), True)
        diag = {"code": {"code": "unused_variables"}, "level": "error"}
        harness.check_expected(step, (1, [diag]))
        for result in [(0, [diag]), (1, []), (1, [diag, diag]), (-11, [diag]), (101, [diag])]:
            with self.assertRaises(AssertionError):
                harness.check_expected(step, result)
        with self.assertRaises(AssertionError):
            harness.check_expected(harness.Step("clean", ""), (0, [diag]))

    def test_commands_explicitly_toggle_tracked_option(self):
        off = harness.command(Path("/compiler"), Path("/work"), Path("/cache"), False)
        on = harness.command(Path("/compiler"), Path("/work"), Path("/cache"), True)
        self.assertIn(harness.FLAG + "no", off)
        self.assertIn(harness.FLAG + "yes", on)
        self.assertEqual(off[:-1], on[:-1])
        self.assertIn("-Cincremental=/cache", on)
        self.assertIn("-Zincremental-verify-ich", on)

    def test_edit_matrix_and_persistent_cache(self):
        calls = []
        def fake_run(command, **kwargs):
            calls.append((command, Path(command[1]).read_text()))
            return subprocess.CompletedProcess(command, 0, "", "")
        with tempfile.TemporaryDirectory() as tmp:
            with patch.object(harness, "steps", return_value=iter([
                harness.Step("a", "first"), harness.Step("b", "second")
            ])), patch.object(harness.subprocess, "run", side_effect=fake_run), contextlib.redirect_stdout(io.StringIO()):
                report = {"observations": [], "completed_cases": 0}
                harness.run(Path("/not-executed"), Path(tmp), report)
                self.assertEqual(report["completed_cases"], 2)
                self.assertEqual(len(report["observations"]), 2)
                self.assertEqual(len(report["observations"][0]["invocations"]), 4)
        self.assertEqual(len(calls), 8)
        self.assertEqual([src for _, src in calls], ["first"] * 4 + ["second"] * 4)
        caches = [next(arg for arg in cmd if arg.startswith("-Cincremental=")) for cmd, _ in calls]
        self.assertEqual(caches[0], caches[4])
        self.assertEqual(caches[2], caches[6])
        self.assertNotEqual(caches[1], caches[5])
        self.assertNotEqual(caches[0], caches[2])

    def test_diagnostic_drift_fails_matrix(self):
        sequence = [subprocess.CompletedProcess([], 0, "", ""),
                    subprocess.CompletedProcess([], 0, "", '{"level":"note","message":"drift"}\n')]
        with tempfile.TemporaryDirectory() as tmp:
            with patch.object(harness, "steps", return_value=iter([harness.Step("a", "src")])), patch.object(harness.subprocess, "run", side_effect=sequence * 2):
                with self.assertRaises(AssertionError):
                    harness.run(Path("/not-executed"), Path(tmp))

    def test_required_transitions_present(self):
        names = [step.name for step in harness.steps()]
        self.assertEqual(len(names), len(set(names)))
        for name in ("crate-forbid", "parent-forbid", "command-deny", "body-attribute",
                     "expect-fulfilled", "expect-unfulfilled", "nested-owners",
                     "macro-empty", "macro-attribute", "macro-reverted", "type-error",
                     "parse-error", "recovery", "attribute-added", "attribute-removed"):
            self.assertIn(name, names)

    def test_capture_failure_retains_partial_evidence(self):
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            with patch.object(harness, "digest", side_effect=OSError("identity unavailable")), contextlib.redirect_stderr(io.StringIO()):
                with self.assertRaisesRegex(OSError, "identity unavailable"):
                    harness.capture(Path("/not-executed"), root)
            report = json.loads((root / "acceptance.json").read_text())
            self.assertEqual(report["verdict"], "failed")
            self.assertEqual(report["completed_cases"], 0)
            self.assertTrue(report["verify_ich"])
            self.assertIn("identity unavailable", report["error"])


if __name__ == "__main__":
    unittest.main()
