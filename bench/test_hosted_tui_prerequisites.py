#!/usr/bin/env python3
"""Verify hosted TUI fixture prerequisites against the exact pinned sources."""
import io
import re
import subprocess
import tarfile
import tempfile
import unittest
from pathlib import Path
from unittest.mock import patch

import suite

PATCH = "bench/patches/jcode-hosted-tui-prerequisites.patch"
PREVIOUS = "bench/patches/jcode-tui-fixture-state.patch"


def assertions(source):
    """Return complete assertion macros, including their original messages."""
    result = []
    tokens = list(re.finditer(
        r'//[^\n]*|/\*.*?\*/|r(?P<hashes>#+)".*?"(?P=hashes)|'
        r'"(?:\\.|[^"\\])*"|\'(?:\\.|[^\'\\])\'|\bassert(?:_eq|_ne)?!\s*\(|[()]',
        source, re.S))
    for index, match in enumerate(tokens):
        if not re.match(r"assert(?:_eq|_ne)?!", match.group()):
            continue
        depth = 1
        for token in tokens[index + 1:]:
            if token.group() == "(":
                depth += 1
            elif token.group() == ")":
                depth -= 1
                if depth == 0:
                    result.append(source[match.start():token.end()])
                    break
        else:
            raise AssertionError("unterminated assertion")
    return result


def test_names(source):
    return re.findall(r"#\[test\]\s*(?:#\[[^\]]+\]\s*)*fn\s+(\w+)", source)


class HostedTuiPrerequisites(unittest.TestCase):
    def test_registration_and_provenance(self):
        spec = suite.SNAPSHOTS["desktop"]
        self.assertEqual(spec["repos"]["jcode"][1], "abff94eeb")
        index = spec["patches"].index(("jcode", PATCH))
        self.assertEqual(spec["patches"][index - 1], ("jcode", PREVIOUS))
        fingerprint = suite.snapshot_fingerprint("desktop")
        old = dict(spec, patches=[p for p in spec["patches"] if p[1] != PATCH])
        with patch.dict(suite.SNAPSHOTS, {"desktop": old}):
            self.assertNotEqual(fingerprint, suite.snapshot_fingerprint("desktop"))
        # The suite publishes these fingerprints in each result's environment.
        source = (suite.ROOT / "bench/suite.py").read_text()
        self.assertIn('"snapshot_fingerprints": {name: snapshot_fingerprint(name)', source)
        self.assertIn('"patches": [(repo, path, hashlib.sha256', source)

    def test_fixture_contracts(self):
        text = (suite.ROOT / PATCH).read_text()
        self.assertEqual(text.count("+        let _local = local_environment();"), 3)
        self.assertIn("let lock = lock_shared_state();", text)
        self.assertIn("impl Drop for LocalEnvironment", text)
        self.assertIn('Some(value) => crate::env::set_var("JCODE_SSH_REMOTE", value)', text)
        self.assertEqual(text.count("+    let git_fixture = create_real_git_repo_fixture();"), 2)
        self.assertEqual(text.count("+    with_temp_jcode_home(|| {"), 11)
        self.assertEqual(text.count("+    with_local_jcode_home(|| {"), 12)
        self.assertIn("crate::auth::antigravity::save_tokens", text)
        self.assertIn("expires_at: i64::MAX", text)
        self.assertIn('let _ssh = EnvRestoreGuard::capture(["JCODE_SSH_REMOTE"]);', text)
        self.assertNotIn("RUST_TEST_THREADS", text)
        self.assertNotIn("#[serial", text)
        self.assertNotRegex(text, r"(?m)^\+.*(?:#\[ignore|#\[cfg|--ignored|set_current_dir)")

    def test_applies_after_canonical_patches_preserving_all_tests_and_assertions(self):
        spec = suite.SNAPSHOTS["desktop"]
        patches = [suite.ROOT / path for repo, path in spec["patches"] if repo == "jcode"]
        paths = sorted({line[6:] for p in patches for line in p.read_text().splitlines()
                        if line.startswith("+++ b/")})
        targets = [line[6:] for line in (suite.ROOT / PATCH).read_text().splitlines()
                   if line.startswith("+++ b/")]
        self.assertEqual(len(targets), 9)
        for path in targets:
            if path == "crates/jcode-tui/src/tui/ui_header.rs":
                continue  # Authorized exception, runtime-byte proof below.
            if path == "crates/jcode-tui/src/tui/ui_messages/tests.rs":
                continue  # Entire file is the existing cfg(test) module.
            self.assertTrue(path.startswith("crates/jcode-tui/src/tui/app/"), path)
            self.assertTrue("/tests/" in path or path.endswith(("/commands_tests.rs", "/remote_tests.rs")), path)
        repo, revision = spec["repos"]["jcode"]
        # Read the immutable object, never the live checkout or its Git metadata.
        archive = subprocess.check_output(["git", "-C", str(repo), "archive", revision, *paths])
        with tempfile.TemporaryDirectory(prefix="jcode-hosted-fixture-") as directory:
            root = Path(directory)
            with tarfile.open(fileobj=io.BytesIO(archive)) as tar:
                tar.extractall(root, filter="data")
            original = {path: (root / path).read_text() for path in targets}
            for p in patches:
                if p == suite.ROOT / PATCH:
                    original = {path: (root / path).read_text() for path in targets}
                result = subprocess.run(["git", "apply", "--check", str(p)], cwd=root,
                                        capture_output=True, text=True)
                self.assertEqual(result.returncode, 0, f"{p}: {result.stderr}")
                subprocess.run(["git", "apply", str(p)], cwd=root, check=True,
                               capture_output=True, text=True)
            for path in targets:
                updated = (root / path).read_text()
                self.assertEqual(test_names(original[path]), test_names(updated), path)
                self.assertEqual(assertions(original[path]), assertions(updated), path)
                self.assertEqual(original[path].count("#[ignore"), updated.count("#[ignore"), path)
                blocks = re.split(r"(?m)^#\[test\]\n", updated)[1:]
                if path.endswith("/commands_tests.rs"):
                    self.assertRegex(updated, r"fn colors_harmony_reports_a_score_and_named_criteria\(\) \{\s*let _local = local_environment\(\);")
                if path.endswith("/ui_messages/tests.rs"):
                    expected = original[path]
                    for name in ("render_system_message_uses_scheduled_task_card",
                                 "render_tool_message_uses_scheduled_card"):
                        anchor = f"fn {name}() {{\n"
                        self.assertEqual(expected.count(anchor), 1)
                        expected = expected.replace(anchor, anchor + "    let _glyph_env = system_glyph_env_lock();\n")
                    self.assertEqual(expected, updated)
                    self.assertRegex(updated, r"fn render_system_message_uses_width_stable_titles_on_kitty\(\) \{\s*let _guard = system_glyph_env_lock\(\);")
                if path.endswith("terminal_setup_command.rs"):
                    for block in blocks:
                        if "create_test_app()" in block:
                            self.assertRegex(block, r"fn \w+\(\) \{\s*with_local_jcode_home\(\|\| \{")
                if path.endswith("state_model_poke_03.rs"):
                    for block in blocks:
                        if any(name in block.splitlines()[0] for name in (
                            "test_open_model_picker_without_routes_shows_actionable_guidance",
                            "test_tui_openai_compatible_empty_catalog_does_not_switch_to_profile_default",
                            "test_login_completed_spawns_auth_refresh_when_runtime_is_available",
                            "test_model_picker_waits_for_async_post_login_catalog_activation",
                            "test_login_completed_surfaces_new_provider_models_in_local_model_picker",
                            "test_local_antigravity_model_picker_selection_preserves_antigravity_provider",
                            "test_agent_model_picker_openrouter_bare_openai_route_saves_openai_catalog_prefix",
                        )):
                            self.assertRegex(block, r"fn \w+\(\) \{\s*(?://[^\n]*\n\s*)*with_local_jcode_home\(\|\| \{")
                if path.endswith("/ui_header.rs"):
                    marker = "#[cfg(test)]\nmod tests {"
                    self.assertEqual(original[path].count(marker), 1)
                    self.assertEqual(updated.count(marker), 1)
                    self.assertEqual(original[path].split(marker)[0], updated.split(marker)[0])
                    # Stronger than a prefix check: the ONLY changed bytes are
                    # this guard inside the existing test helper. All runtime
                    # items, other test helpers and test bodies are identical.
                    anchor = (
                        "    fn ensure_test_jcode_home_if_unset() {\n"
                        "        static TEST_HOME: OnceLock<std::path::PathBuf> = OnceLock::new();\n\n"
                    )
                    insertion = (
                        "        // This helper is test-only. Do not overwrite a scoped fixture's home.\n"
                        "        let Ok(_env_lock) = crate::storage::test_env_lock().try_lock() else {\n"
                        "            return;\n"
                        "        };\n\n"
                    )
                    self.assertEqual(original[path].count(anchor), 1)
                    self.assertEqual(original[path].replace(anchor, anchor + insertion), updated)
                if path.endswith(("support_failover/part_01.rs", "/remote_tests.rs", "/ui_header.rs")):
                    helper = updated.split("fn ensure_test_jcode_home_if_unset() {", 1)[1].split("\n}", 1)[0]
                    self.assertRegex(helper, r"let Ok\(_env_lock\) = crate::storage::test_env_lock\(\).try_lock\(\) else \{\s*return;\s*\};")
                    self.assertLess(helper.index("let Ok(_env_lock)"), helper.index('crate::env::set_var("JCODE_HOME"'))
                if path.endswith("remote_startup_input_03/part_01.rs"):
                    roundtrips = []
                    for block in re.split(r"(?m)^#\[test\]\n", updated)[1:]:
                        if "save_input_for_reload(" not in block and "restore_input_for_reload(" not in block:
                            continue
                        name = re.search(r"fn (\w+)\(", block).group(1)
                        roundtrips.append(name)
                        self.assertRegex(block, r"fn \w+\(\) \{\s*(?://[^\n]*\n\s*)*with_temp_jcode_home\(\|\| \{", name)
                    self.assertIn("test_save_and_restore_reload_state_preserves_interleave_and_pending_retry", roundtrips)
                    self.assertGreaterEqual(len(roundtrips), 7)
                    helper = subprocess.check_output(
                        ["git", "-C", str(repo), "show",
                         f"{revision}:crates/jcode-tui/src/tui/app/tests/support_failover/part_01.rs"],
                        text=True)
                    self.assertRegex(helper, r"fn with_temp_jcode_home[^\n]+\n\s*let _guard = crate::storage::lock_test_env\(\)")
                    self.assertIn('EnvRestoreGuard::capture(["JCODE_HOME"])', helper)
            previous_targets = {path: source for path, source in original.items()
                                if not path.endswith("/ui_messages/tests.rs")}
            self.assertEqual(sum(len(test_names(source)) for source in previous_targets.values()), 254)
            self.assertEqual(sum(len(assertions(source)) for source in previous_targets.values()), 1077)
            ui_messages = subprocess.check_output(
                ["git", "-C", str(repo), "show", f"{revision}:crates/jcode-tui/src/tui/ui_messages.rs"],
                text=True)
            self.assertRegex(ui_messages, r'#\[cfg\(test\)\]\s*#\[path = "ui_messages/tests.rs"\]\s*mod tests;')
            commands = subprocess.check_output(
                ["git", "-C", str(repo), "show", f"{revision}:crates/jcode-tui/src/tui/app/commands.rs"],
                text=True)
            self.assertRegex(commands, r'#\[cfg\(test\)\]\s*#\[path = "commands_tests.rs"\]')


if __name__ == "__main__":
    unittest.main()
