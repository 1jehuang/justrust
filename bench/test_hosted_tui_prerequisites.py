#!/usr/bin/env python3
"""Verify hosted TUI fixture prerequisites against the exact pinned sources."""
import io
import itertools
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


def fixture_bodies(source):
    """Unindented test items in the affected app fixture modules."""
    return dict(re.findall(r"#\[test\]\nfn (\w+)\(\) \{\n(.*?)\n\}", source, re.S))


def assert_reader_coverage(case, path, source):
    """Derive coverage from fixture operations, not the latest failing names."""
    bodies = fixture_bodies(source)
    readers = {}
    for name, body in bodies.items():
        if path.endswith("/remote_tests.rs"):
            reads_environment = "create_test_app()" in body
            scoped = "crate::tui::app::tests::with_local_jcode_home"
        else:
            reads_environment = bool(re.search(
                r"create_\w*(?:test_app|model_app)\(|App::new_for_test_harness\(", body))
            scoped = "with_local_jcode_home"
        if not reads_environment:
            continue
        readers[name] = body
        # Existing owned fixtures keep their original single lock. New readers
        # must hold a scoped home for the entire body, including async polling.
        if "crate::storage::lock_test_env()" in body:
            case.assertNotIn(f"{scoped}(|| {{", body, name)
        elif not re.match(r"\s*(?:use [^;]+;\s*)*with_temp_jcode_home\(\|\| \{", body):
            case.assertRegex(body, rf"\A\s*(?://[^\n]*\n\s*)*{re.escape(scoped)}\(\|\| \{{", name)
            case.assertTrue(body.rstrip().endswith("});"), name)
            case.assertNotRegex(body, r"(?:lock_test_env|lock_shared_state)\(", name)
    case.assertEqual(len(readers), 26 if path.endswith("/remote_tests.rs") else 52)
    return readers


def assert_copy_reader_coverage(case, source):
    """The env guard spans constructor generation capture through key dispatch."""
    readers = {name: body for name, body in fixture_bodies(source).items()
               if "create_copy_test_app()" in body}
    for name, body in readers.items():
        if "crate::storage::lock_test_env()" in body:
            case.assertNotIn("with_local_jcode_home(|| {", body, name)
            case.assertLess(body.index("lock_test_env()"), body.index("scroll_render_test_lock()"), name)
        else:
            case.assertTrue(body.startswith("    with_local_jcode_home(|| {\n"), name)
            case.assertTrue(body.endswith("    });"), name)
            case.assertNotRegex(body, r"(?:lock_test_env|lock_shared_state)\(", name)
    return readers


class HostedTuiPrerequisites(unittest.TestCase):
    def test_copy_reader_config_generation_interleavings(self):
        """Exhaust the two fixture schedules, with no sleeps or scheduler luck.

        This is a protocol model, not hosted Rust acceptance. Source anchors
        bind its generation capture, writer invalidation, and key dispatch to
        the pinned implementation rather than inventing an unrelated mutex.
        """
        repo, revision = suite.SNAPSHOTS["desktop"]["repos"]["jcode"]

        def source(path):
            return subprocess.check_output(
                ["git", "-C", str(repo), "show", f"{revision}:crates/{path}"], text=True)

        lifecycle = source("jcode-tui/src/tui/app/tui_lifecycle.rs")
        refresh = lifecycle.split("fn refresh_keybindings_if_config_reloaded", 1)[1].split("\n    }", 1)[0]
        self.assertIn("keybindings_config_generation: crate::config::config_reload_generation()", lifecycle)
        self.assertIn("if generation == self.keybindings_config_generation", refresh)
        self.assertIn('self.set_status_notice("Config reloaded from disk")', refresh)
        dispatch = source("jcode-tui/src/tui/app/input.rs").split("fn handle_key_press_event", 1)[1].split("\n    }", 1)[0]
        self.assertLess(dispatch.index("refresh_keybindings_if_config_reloaded()"), dispatch.index("handle_key_core("))
        config = source("jcode-base/src/config.rs")
        invalidation = config.split("pub fn invalidate_config_cache()", 1)[1].split("\n}", 1)[0]
        self.assertIn("notify_config_reloaded();", invalidation)
        self.assertIn("CONFIG_RELOAD_GENERATION.fetch_add(1", config)
        fixture = source("jcode-tui/src/tui/app/tests/support_failover/part_01.rs")
        writer = fixture.split("fn with_temp_jcode_home", 1)[1].split("\n}", 1)[0]
        self.assertLess(writer.index("lock_test_env()"), writer.index("invalidate_config_cache()"))
        self.assertLess(writer.index("invalidate_config_cache()"), writer.index("let result = f();"))
        render = source("jcode-tui/src/tui/app/tests/scroll_copy_01/part_01.rs")
        render_lock = render.split("fn scroll_render_test_lock()", 1)[1].split("\n}", 1)[0]
        self.assertIn("render_state_test_lock()", render_lock)
        self.assertNotIn("lock_test_env()", render_lock)

        def mismatches(scoped):
            outcomes = []
            # Reader: acquire, construct/capture, dispatch/compare, release.
            # Writer: acquire env lock, invalidate generation, release.
            for reader_slots in itertools.combinations(range(7), 4):
                events = iter(("r_lock", "capture", "dispatch", "r_unlock"))
                writes = iter(("w_lock", "invalidate", "w_unlock"))
                schedule = [next(events) if slot in reader_slots else next(writes) for slot in range(7)]
                held, generation, captured, mismatch = None, 0, None, False
                for event in schedule:
                    if event in ("r_lock", "w_lock"):
                        if event == "r_lock" and not scoped:
                            continue  # render lock is a different resource
                        if held is not None:
                            break  # this interleaving blocks on the shared lock
                        held = event[0]
                    elif event == "capture":
                        captured = generation
                    elif event == "invalidate":
                        generation += 1
                    elif event == "dispatch":
                        mismatch = captured != generation
                    elif event == "w_unlock" or scoped:
                        held = None
                else:
                    outcomes.append(mismatch)
            return outcomes

        self.assertIn(True, mismatches(scoped=False))
        self.assertTrue(mismatches(scoped=True))
        self.assertNotIn(True, mismatches(scoped=True))

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
        for name in ("test-retry-contention", "test-retry-storm"):
            scenario = next(s for s in suite.SCENARIOS if s.name == name)
            self.assertEqual(scenario.env["RUST_TEST_THREADS"], "16")

    def test_fixture_contracts(self):
        text = (suite.ROOT / PATCH).read_text()
        self.assertEqual(text.count("+        let _local = local_environment();"), 3)
        self.assertIn("let lock = lock_shared_state();", text)
        self.assertIn("impl Drop for LocalEnvironment", text)
        self.assertIn('Some(value) => crate::env::set_var("JCODE_SSH_REMOTE", value)', text)
        self.assertEqual(text.count("+    let git_fixture = create_real_git_repo_fixture();"), 2)
        self.assertEqual(text.count("+    with_temp_jcode_home(|| {"), 11)
        self.assertEqual(text.count("+    with_local_jcode_home(|| {"), 54)
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
        self.assertEqual(len(targets), 14)
        for path in targets:
            if path == "crates/jcode-tui/src/tui/ui_header.rs":
                continue  # Authorized exception, runtime-byte proof below.
            if path == "crates/jcode-tui/src/tui/ui_messages/tests.rs":
                continue  # Entire file is the existing cfg(test) module.
            self.assertTrue(path.startswith("crates/jcode-tui/src/tui/app/"), path)
            self.assertTrue("/tests/" in path or path.endswith(("/commands_tests.rs", "/remote_tests.rs", "/helpers_tests.rs")), path)
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
                if "/scroll_copy_" in path:
                    readers = assert_copy_reader_coverage(self, updated)
                    self.assertEqual(len(readers), 2 if "scroll_copy_01" in path else
                                     9 if path.endswith("part_01.rs") else 8)
                    # Only wrap previously unowned copy readers. Helpers,
                    # clipboard hooks, original locks, every test statement,
                    # and all non-copy tests remain byte-for-byte identical.
                    def scope_copy(match):
                        body = match.group(2)
                        if "create_copy_test_app()" not in body or "crate::storage::lock_test_env()" in body:
                            return match.group(0)
                        return match.group(1) + "    with_local_jcode_home(|| {\n" + body + "\n    });\n}"
                    expected = re.sub(r"(#\[test\]\nfn \w+\(\) \{\n)(.*?)(?=\n\})\n\}",
                                      scope_copy, original[path], flags=re.S)
                    self.assertEqual(expected, updated)
                    for name, body in readers.items():
                        if "with_local_jcode_home(|| {" not in body:
                            continue
                        anchor = f"fn {name}() {{\n    with_local_jcode_home(|| {{\n"
                        unguarded = updated.replace(anchor, f"fn {name}() {{\n")
                        with self.assertRaisesRegex(AssertionError, name):
                            assert_copy_reader_coverage(self, unguarded)
                        nested = updated.replace(anchor, anchor + "    let _nested = crate::storage::lock_test_env();\n")
                        with self.assertRaises(AssertionError):
                            assert_copy_reader_coverage(self, nested)
                if path.endswith("support_failover/part_01.rs"):
                    helper = updated.split("pub(super) fn with_local_jcode_home", 1)[1].split("\n}", 1)[0]
                    self.assertEqual(helper.count("with_temp_jcode_home(|| {"), 1)
                    self.assertNotIn("lock_test_env()", helper)
                    self.assertIn('EnvRestoreGuard::capture(["JCODE_SSH_REMOTE"])', helper)
                    self.assertIn('crate::env::remove_var("JCODE_SSH_REMOTE")', helper)
                if path.endswith(("/remote_tests.rs", "state_model_poke_03.rs")):
                    assert_reader_coverage(self, path, updated)
                    neighbor = ("idle_donut_pauses_while_unfocused" if path.endswith("/remote_tests.rs")
                                else "test_model_picker_reuses_cached_entries_until_invalidated")
                    helper = ("crate::tui::app::tests::with_local_jcode_home" if path.endswith("/remote_tests.rs")
                              else "with_local_jcode_home")
                    anchor = f"fn {neighbor}() {{\n    {helper}(|| {{\n"
                    self.assertEqual(updated.count(anchor), 1)
                    unguarded = updated.replace(anchor, f"fn {neighbor}() {{\n")
                    with self.assertRaisesRegex(AssertionError, neighbor):
                        assert_reader_coverage(self, path, unguarded)
                    nested = updated.replace(anchor, anchor +
                        "    let _nested = crate::storage::lock_test_env();\n")
                    with self.assertRaisesRegex(AssertionError, neighbor):
                        assert_reader_coverage(self, path, nested)
                if path.endswith("/helpers_tests.rs"):
                    # A tuple binding drops fields left to right, unlike local
                    # bindings. Unlocking first lets its late HOME restore
                    # overwrite the next fixture even when that fixture locks.
                    self.assertRegex(updated, r"fn pinned_resume_test_home\(\) -> \(\s*EnvVarGuard,\s*tempfile::TempDir,\s*std::sync::MutexGuard<'static, \(\)>,\s*\)")
                    self.assertIn("(home, temp, env_lock)", updated)
                    self.assertNotIn("(env_lock, temp, home)", updated)
                    self.assertEqual(updated.count("let _pinned = pinned_resume_test_home();"), 2)
                    expected = original[path].replace(
                        "    std::sync::MutexGuard<'static, ()>,\n    tempfile::TempDir,\n    EnvVarGuard,",
                        "    EnvVarGuard,\n    tempfile::TempDir,\n    std::sync::MutexGuard<'static, ()>,")
                    expected = expected.replace("    (env_lock, temp, home)",
                        "    // Aggregate fields drop in declaration order: restore HOME before unlock.\n    (home, temp, env_lock)")
                    self.assertEqual(expected, updated)
                if path.endswith("remote_startup_input_01/part_02.rs"):
                    anchor = "fn test_model_picker_remote_comtegra_model_uses_comtegra_route_not_copilot() {\n"
                    self.assertEqual(original[path].replace(anchor, anchor +
                        "    let _env_lock = crate::storage::lock_test_env();\n"), updated)
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
                                if not path.endswith(("/ui_messages/tests.rs", "/helpers_tests.rs",
                                                     "/remote_startup_input_01/part_02.rs"))
                                and "/scroll_copy_" not in path}
            self.assertEqual(sum(len(test_names(source)) for source in previous_targets.values()), 254)
            self.assertEqual(sum(len(assertions(source)) for source in previous_targets.values()), 1077)
            app = subprocess.check_output(
                ["git", "-C", str(repo), "show", f"{revision}:crates/jcode-tui/src/tui/app.rs"],
                text=True)
            self.assertRegex(app, r"#\[cfg\(test\)\]\s*mod tests;")
            includes = subprocess.check_output(
                ["git", "-C", str(repo), "show", f"{revision}:crates/jcode-tui/src/tui/app/tests.rs"],
                text=True)
            for path in targets:
                if "/scroll_copy_" in path:
                    relative = path.split("/app/", 1)[1]
                    self.assertIn(f'include!("{relative}");', includes)
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
