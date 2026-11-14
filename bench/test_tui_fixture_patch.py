#!/usr/bin/env python3
"""Guard the pinned TUI fixture patch without building the application."""
import re
import unittest
from collections import Counter
from unittest.mock import patch

import suite


PATCH = "bench/patches/jcode-tui-fixture-state.patch"


class TuiFixturePatch(unittest.TestCase):
    def test_patch_is_registered_and_fingerprinted(self):
        spec = suite.SNAPSHOTS["desktop"]
        self.assertIn(("jcode", PATCH), spec["patches"])
        before = suite.snapshot_fingerprint("desktop")
        without = dict(spec, patches=[item for item in spec["patches"] if item[1] != PATCH])
        with patch.dict(suite.SNAPSHOTS, {"desktop": without}):
            self.assertNotEqual(before, suite.snapshot_fingerprint("desktop"))
        self.assertEqual(spec["repos"]["jcode"][1], "abff94eeb")

    def test_only_test_sources_change_and_moved_lines_survive(self):
        text = (suite.ROOT / PATCH).read_text()
        sections = text.split("diff --git ")[1:]
        self.assertTrue(sections)
        for section in sections:
            path = section.splitlines()[0].split()[1].removeprefix("b/")
            self.assertTrue(path.startswith("crates/jcode-tui/src/tui/"), path)
            self.assertTrue("/tests/" in path or "/ui_tests/" in path, path)
            removed = [line[1:].strip() for line in section.splitlines()
                       if line.startswith("-") and not line.startswith("---") and line[1:].strip()]
            added = [line[1:].strip() for line in section.splitlines()
                     if line.startswith("+") and not line.startswith("+++") and line[1:].strip()]
            # Every removed line is only reindented inside an isolation scope.
            # This also preserves every assertion, test name and cleanup path.
            self.assertFalse(Counter(removed) - Counter(added), path)
            self.assertNotRegex(section, r"(?m)^\+\s*#\[(?:ignore|cfg)\b")

    def test_all_eight_fixture_contracts_are_explicit(self):
        text = (suite.ROOT / PATCH).read_text()
        for name in (
            "answering_no_on_continue_prompt_lands_on_regular_session",
            "test_local_model_picker_surfaces_antigravity_models_from_multiprovider",
            "test_local_model_picker_openrouter_bare_openai_route_uses_openai_catalog_prefix",
            "test_info_widget_local_direct_api_runtime_shows_cost_based_usage",
            "test_new_for_remote_restores_observe_mode_from_reload_state",
            "test_restore_session_adds_reload_message",
            "test_restore_session_with_selfdev_reload_tool_result_queues_continuation",
            "test_prompt_entry_shimmer_color_moves_across_positions",
        ):
            self.assertIn(name, text)
        self.assertEqual(len(re.findall(r"^\+\s*with_temp_jcode_home\(\|\| \{", text, re.M)), 5)
        self.assertIn("pin_truecolor_for_tests()", text)
        self.assertIn('"sk-onboarding-fixture"', text)
        self.assertIn('"sk-openai-widget-fixture"', text)
        self.assertIn('"sk-anthropic-widget-fixture"', text)


if __name__ == "__main__":
    unittest.main()
