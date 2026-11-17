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
                        JCODE_SOCKET="live", JCODE_SESSION_ID="live-session", JCODE_HOME="live-home",
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
                for key in ("JCODE_SOCKET", "JCODE_SESSION_ID", "JCODE_HOME", "ANTHROPIC_API_KEY", "OPENAI_API_KEY"):
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

    def test_implicit_compiler_homes_survive_repeated_isolated_commands(self):
        env = {"HOME": self.env["HOME"], "PATH": os.environ["PATH"]}
        original = dict(env)
        compiler_home = Path(env["HOME"]) / ".justrust"
        compiler_home.mkdir(parents=True)
        (compiler_home / "owned-compiler").write_text("already installed")
        homes = []
        with patch.dict(os.environ, {runtime.SELECTOR: runtime.POLICY}, clear=True):
            for _ in range(2):
                with runtime.runtime_context(env, self.cwd) as child:
                    homes.append(child["HOME"])
                    self.assertEqual(child["JUSTRUST_HOME"], str(compiler_home))
                    self.assertEqual(child["CARGO_HOME"], env["HOME"] + "/.cargo")
                    self.assertEqual(child["RUSTUP_HOME"], env["HOME"] + "/.rustup")
                    actual = subprocess.check_output([
                        sys.executable, "-c",
                        "import os,pathlib; print((pathlib.Path(os.environ['JUSTRUST_HOME']) / 'owned-compiler').read_text())"
                    ], env=child, text=True)
                    self.assertEqual(actual.strip(), "already installed")
        self.assertNotEqual(homes[0], homes[1])
        self.assertEqual(env, original)
        self.assertTrue(compiler_home.exists())

    def test_caller_config_identity_survives_but_other_git_settings_do_not(self):
        home = Path(self.env["HOME"])
        home.mkdir()
        config = home / ".gitconfig"
        original = '[user]\n name = Caller Configured Identity\n email = caller@example.invalid\n[commit]\n gpgsign = true\n[credential]\n helper = forbidden-helper\n'
        config.write_text(original)
        env = {"PATH": os.environ["PATH"], "HOME": str(home),
               "GIT_CONFIG_NOSYSTEM": "1", "GIT_TRACE": "1"}
        with patch.dict(os.environ, {runtime.SELECTOR: runtime.POLICY}, clear=True):
            with runtime.runtime_context(env, self.cwd) as child:
                for role in ("AUTHOR", "COMMITTER"):
                    self.assertEqual(child[f"GIT_{role}_NAME"], "Caller Configured Identity")
                    self.assertEqual(child[f"GIT_{role}_EMAIL"], "caller@example.invalid")
                self.assertNotIn("GIT_TRACE", child)
                self.assertFalse((Path(child["HOME"]) / ".gitconfig").exists())
                for key in ("commit.gpgsign", "credential.helper"):
                    result = subprocess.run(["git", "config", "--global", "--get", key],
                                            cwd=self.cwd, env=child, capture_output=True)
                    self.assertEqual(result.returncode, 1)
        self.assertEqual(config.read_text(), original)

    def test_explicit_distinct_author_committer_identity_wins_over_config(self):
        env = {"PATH": os.environ["PATH"], "HOME": self.env["HOME"],
               "GIT_CONFIG_NOSYSTEM": "1",
               "GIT_AUTHOR_NAME": "Configured Author", "GIT_AUTHOR_EMAIL": "author@example.invalid",
               "GIT_COMMITTER_NAME": "Configured Committer", "GIT_COMMITTER_EMAIL": "committer@example.invalid",
               "GIT_AUTHOR_DATE": "1234567890 +0000", "GIT_COMMITTER_DATE": "1234567890 +0000"}
        with patch.dict(os.environ, {runtime.SELECTOR: runtime.POLICY}, clear=True):
            with runtime.runtime_context(env, self.cwd) as child:
                for role in ("AUTHOR", "COMMITTER"):
                    for field in ("NAME", "EMAIL"):
                        key = f"GIT_{role}_{field}"
                        self.assertEqual(child[key], env[key])
                    self.assertNotIn(f"GIT_{role}_DATE", child)

    def test_v1_history_is_readable_but_not_comparable_or_selectable_as_v2(self):
        old = {"env": {"runtime_context": {"policy": "jcode-isolated-v1"}}}
        new = {"env": {"runtime_context": {"policy": runtime.POLICY}}}
        runtime.require_matching_context(old, old)
        with self.assertRaisesRegex(ValueError, "runtime contexts differ"):
            runtime.require_matching_context(old, new)
        with patch.dict(os.environ, {runtime.SELECTOR: "jcode-isolated-v1"}, clear=True):
            with self.assertRaisesRegex(ValueError, "unsupported"):
                with runtime.runtime_context(self.env, self.cwd):
                    self.fail("obsolete runtime policy executed")

    def test_v2_history_is_readable_but_cannot_mix_with_corrected_policy(self):
        old = {"env": {"runtime_context": {"policy": "jcode-isolated-v2"}}}
        new = {"env": {"runtime_context": {"policy": runtime.POLICY}}}
        runtime.require_matching_context(old, old)
        with self.assertRaisesRegex(ValueError, "runtime contexts differ"):
            runtime.require_matching_context(old, new)
        with patch.dict(os.environ, {runtime.SELECTOR: "jcode-isolated-v2"}, clear=True):
            with self.assertRaisesRegex(ValueError, "unsupported"):
                with runtime.runtime_context(self.env, self.cwd):
                    self.fail("obsolete runtime policy executed")

    def test_existing_git_metadata_untouched_even_after_failure(self):
        marker = self.cwd / ".git"
        marker.mkdir()
        with patch.dict(os.environ, {runtime.SELECTOR: runtime.POLICY}, clear=True):
            with self.assertRaisesRegex(RuntimeError, "test failure"):
                with runtime.runtime_context(self.env, self.cwd):
                    raise RuntimeError("test failure")
        self.assertTrue(marker.is_dir())

    def test_v3_history_remains_readable_but_not_selectable_or_comparable(self):
        old = {"env": {"runtime_context": {"policy": "jcode-isolated-v3"}}}
        current = {"env": {"runtime_context": runtime.provenance()}}
        runtime.require_matching_context(old, old)
        with self.assertRaisesRegex(ValueError, "runtime contexts differ"):
            runtime.require_matching_context(old, current)
        with patch.dict(os.environ, {runtime.SELECTOR: "jcode-isolated-v3"}, clear=True):
            with self.assertRaisesRegex(ValueError, "unsupported"):
                with runtime.runtime_context(self.env, self.cwd):
                    self.fail("obsolete runtime policy executed")

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

    def test_missing_or_null_environment_defers_to_synthetic_reporting(self):
        for legacy in ({}, {"env": None}, {"env": {}}):
            with self.subTest(legacy=legacy):
                runtime.require_matching_context(legacy, {"env": {}})
                runtime.require_matching_context(legacy, legacy)
                with self.assertRaisesRegex(ValueError, "runtime contexts differ"):
                    runtime.require_matching_context(legacy, {"env": {"runtime_context": {"policy": runtime.POLICY}}})

    def test_explicit_malformed_context_is_rejected_even_if_both_match(self):
        for context in (None, False, [], "inherited", {}, {"policy": None},
                        {"policy": []}, {"policy": "unknown-v2"}):
            result = {"env": {"runtime_context": context}}
            with self.subTest(context=context), self.assertRaisesRegex(ValueError, "runtime context provenance"):
                runtime.require_matching_context(result, result)

    def test_malformed_result_or_environment_raises_valueerror_not_attributeerror(self):
        for result in (None, [], {"env": []}, {"env": "invalid"}):
            with self.subTest(result=result), self.assertRaisesRegex(ValueError, "provenance"):
                runtime.require_matching_context(result, result)

    def test_counted_execution_records_policy_without_secrets_or_paths(self):
        with patch.dict(os.environ, {runtime.SELECTOR: runtime.POLICY}, clear=True):
            log = Path(self.temp.name) / "output.log"
            result = versus.run_counted([sys.executable, "-c", "import os; print(os.environ['HOME'])"],
                                        self.cwd, self.env, log)
        self.assertEqual(result["exit"], 0)
        self.assertEqual(result["runtime_context"]["policy"], runtime.POLICY)
        public = json.dumps(result["runtime_context"])
        self.assertNotIn(self.temp.name, public)
        self.assertNotIn("secret", public)


if __name__ == "__main__":
    unittest.main()
