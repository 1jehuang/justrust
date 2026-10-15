"""Safety checks for the read-only A2 diagnostic runner, no Rust builds."""
import json
import os
from pathlib import Path
import sys
import tempfile
import unittest
from unittest import mock

import audit_a2_failures as audit


class AuditSafetyTests(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory()
        self.addCleanup(self.temp.cleanup)
        self.root = Path(self.temp.name)
        self.binary = self.root / "fixture"

    def program(self, source):
        self.binary.write_text(f"#!{sys.executable}\n" + source)
        self.binary.chmod(0o700)
        return self.binary

    def test_child_has_only_disposable_state_and_no_inherited_credentials(self):
        self.program("import json,os\nprint('running 1 test')\nprint(json.dumps(dict(os.environ)))\n")
        with mock.patch.dict(os.environ, {"ANTHROPIC_API_KEY": "never-forward",
                                          "OPENAI_API_KEY": "never-forward",
                                          "DBUS_SESSION_BUS_ADDRESS": "never-forward",
                                          "DISPLAY": "never-forward"}):
            result = audit.invoke(self.binary, "marker", self.root, self.root, "fixture")
        self.assertEqual(result["exit"], 0)
        self.assertTrue(result["selected_one_test"])
        env = json.loads(result["output"].splitlines()[1])
        for key in ["ANTHROPIC_API_KEY", "OPENAI_API_KEY", "DBUS_SESSION_BUS_ADDRESS", "DISPLAY"]:
            self.assertNotIn(key, env)
        for key in ["HOME", "JCODE_HOME", "JCODE_RUNTIME_DIR", "XDG_RUNTIME_DIR", "TMPDIR"]:
            self.assertTrue(env[key].startswith("$CASE"))
        self.assertEqual(env["JCODE_NO_BROWSER"], "1")
        self.assertEqual(env["JCODE_NO_TELEMETRY"], "1")
        self.assertEqual(env["PYTHONDONTWRITEBYTECODE"], "1")
        self.assertNotIn("never-forward", result["output"])
        self.assertEqual([p.name for p in self.root.iterdir()], ["fixture"])

    def test_synthetic_files_exist_only_inside_disposable_home(self):
        self.program("import os,pathlib\nprint('running 1 test')\n"
                     "print((pathlib.Path(os.environ['JCODE_HOME'])/'auth.json').read_text())\n")
        result = audit.invoke(self.binary, "marker", self.root, self.root, "fixture",
                              {".jcode/auth.json": "fake-only"})
        self.assertEqual(result["exit"], 0)
        self.assertIn("fake-only", result["output"])
        self.assertFalse((self.root / ".jcode").exists())

    def test_rejects_synthetic_file_escape_before_starting_child(self):
        self.program("raise RuntimeError('must not start')\n")
        for path in ["../escape", str(self.root / "absolute")]:
            with self.assertRaisesRegex(ValueError, "escapes"):
                audit.invoke(self.binary, "marker", self.root, self.root, "fixture", {path: "bad"})
        self.assertFalse((self.root / "absolute").exists())

    def test_timeout_kills_only_its_own_process_group_and_cleans_state(self):
        self.program("import time\nprint('running 1 test',flush=True)\ntime.sleep(30)\n")
        result = audit.invoke(self.binary, "marker", self.root, self.root, "timeout", timeout=0.1)
        self.assertTrue(result["timed_out"])
        self.assertLess(result["exit"], 0)
        self.assertEqual([p.name for p in self.root.iterdir()], ["fixture"])

    def test_tree_manifest_detects_content_modes_and_added_directories(self):
        source = self.root / "source.rs"
        source.write_text("original")
        before = audit.snapshot_manifest(self.root)
        self.assertEqual(before, audit.snapshot_manifest(self.root))
        source.write_text("modified")
        self.assertNotEqual(before, audit.snapshot_manifest(self.root))
        source.write_text("original")
        self.assertEqual(before, audit.snapshot_manifest(self.root))
        source.chmod(0o700)
        self.assertNotEqual(before, audit.snapshot_manifest(self.root))
        (self.root / "new-directory").mkdir()
        self.assertIn("new-directory", audit.snapshot_manifest(self.root))


if __name__ == "__main__":
    unittest.main()
