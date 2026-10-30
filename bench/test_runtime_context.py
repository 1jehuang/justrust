"""Focused runtime-context tests, no compiler or benchmark suite required."""
import json
import os
from pathlib import Path
import subprocess
import sys
import tempfile
import unittest
from unittest.mock import patch

import runtime_context as runtime
import versus


class RuntimeContextTests(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory()
        self.addCleanup(self.temp.cleanup)
        self.cwd = Path(self.temp.name) / "archive"
        self.cwd.mkdir()
        self.source = self.cwd / "source.rs"
        self.source.write_text("unchanged source\n")
        self.env = dict(os.environ, HOME=str(Path(self.temp.name) / "real-home"),
                        JCODE_SOCKET="live", JCODE_SESSION_ID="live-session",
                        ANTHROPIC_API_KEY="secret", OPENAI_API_KEY="secret",
                        RUSTFLAGS="-Zprivate-query-flag", JUSTRUST_TOOLCHAIN="frozen-native",
                        JUSTRUST_HOME="frozen-home", CARGO_HOME="explicit-cargo")

    def test_disabled_is_identity_and_no_files_created(self):
        with patch.dict(os.environ, {}, clear=True):
            with runtime.runtime_context(self.env, self.cwd) as child:
                self.assertIs(child, self.env)
                self.assertFalse((self.cwd / ".git").exists())

    def test_isolation_real_exec_git_discovery_and_cleanup(self):
        with patch.dict(os.environ, {runtime.SELECTOR: runtime.POLICY}, clear=True):
            with runtime.runtime_context(self.env, self.cwd) as child:
                home = Path(child["HOME"])
                self.assertTrue(home.is_dir())
                for key in ("JCODE_SOCKET", "JCODE_SESSION_ID", "ANTHROPIC_API_KEY", "OPENAI_API_KEY"):
                    self.assertNotIn(key, child)
                for key in ("RUSTFLAGS", "JUSTRUST_TOOLCHAIN", "JUSTRUST_HOME", "CARGO_HOME"):
                    self.assertEqual(child[key], self.env[key])
                self.assertEqual(child["RUSTUP_HOME"], self.env["HOME"] + "/.rustup")
                self.assertEqual(subprocess.check_output(["git", "rev-parse", "--show-toplevel"],
                                                        cwd=self.cwd, env=child, text=True).strip(), str(self.cwd))
                actual = json.loads(subprocess.check_output([sys.executable, "-c", "import os,json; print(json.dumps(dict(os.environ)))"], env=child))
                self.assertEqual(actual["HOME"], str(home))
                self.assertNotIn("ANTHROPIC_API_KEY", actual)
                (home / "created-state").write_text("discard")
            self.assertFalse(home.exists())
            self.assertFalse((self.cwd / ".git").exists())
            self.assertEqual(self.source.read_text(), "unchanged source\n")
            with runtime.runtime_context(self.env, self.cwd) as next_child:
                self.assertNotEqual(next_child["HOME"], str(home))
                self.assertFalse((Path(next_child["HOME"]) / "created-state").exists())

    def test_existing_git_metadata_untouched_even_after_failure(self):
        marker = self.cwd / ".git"
        marker.mkdir()
        with patch.dict(os.environ, {runtime.SELECTOR: runtime.POLICY}, clear=True):
            with self.assertRaisesRegex(RuntimeError, "test failure"):
                with runtime.runtime_context(self.env, self.cwd):
                    raise RuntimeError("test failure")
        self.assertTrue(marker.is_dir())

    def test_new_marker_cleaned_after_failure(self):
        with patch.dict(os.environ, {runtime.SELECTOR: runtime.POLICY}, clear=True):
            with self.assertRaisesRegex(RuntimeError, "test failure"):
                with runtime.runtime_context(self.env, self.cwd):
                    raise RuntimeError("test failure")
        self.assertFalse((self.cwd / ".git").exists())

    def test_unknown_policy_fails_closed(self):
        with patch.dict(os.environ, {runtime.SELECTOR: "typo"}, clear=True):
            with self.assertRaises(ValueError):
                with runtime.runtime_context(self.env, self.cwd):
                    self.fail("unknown policy executed")

    def test_comparisons_reject_changed_context_but_accept_legacy_inherited(self):
        legacy = {"env": {}}
        inherited = {"env": {"runtime_context": {"policy": "inherited"}}}
        selected = {"env": {"runtime_context": {"policy": runtime.POLICY}}}
        runtime.require_matching_context(legacy, inherited)
        runtime.require_matching_context(selected, selected)
        with self.assertRaisesRegex(ValueError, "runtime contexts differ"):
            runtime.require_matching_context(legacy, selected)

    def test_counted_execution_records_policy_without_secrets_or_paths(self):
        with patch.dict(os.environ, {runtime.SELECTOR: runtime.POLICY}, clear=True):
            log = Path(self.temp.name) / "output.log"
            result = versus.run_counted([sys.executable, "-c", "import os; print(os.environ['JCODE_HOME'])"],
                                        self.cwd, self.env, log)
        self.assertEqual(result["exit"], 0)
        self.assertEqual(result["runtime_context"]["policy"], runtime.POLICY)
        public = json.dumps(result["runtime_context"])
        self.assertNotIn(self.temp.name, public)
        self.assertNotIn("secret", public)


if __name__ == "__main__":
    unittest.main()
