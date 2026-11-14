#!/usr/bin/env python3
"""Harness and comparison gates. Real Cargo acceptance lives in contention.py."""
import copy
import json
import os
from pathlib import Path
from types import SimpleNamespace
import tempfile
import unittest
from unittest import mock

import contention
import suite


def summary(slot=None):
    return {"tests": {"passed": 2, "failed": 0}, "phases": {"lock_wait": 0},
            "slot": {"slot": slot}}


def result():
    return {"schema": "contention_suite_v1",
            "env": {"execution": "remote", "contention": {"hold_seconds": 20}},
            "scenarios": {contention.SCENARIO: {"samples": [{
                "shared_artifacts_unchanged": True, "tests_passed": 2, "tests_failed": 0,
                "holder_id": "holder", "followup_id": "followup",
                "followup_tests_passed": 2, "followup_units": 0,
                "followup_target": "shared default target",
                "filesystem_magic": "ef53", "remote": {"machine": "host"}}]}}}


class ContentionTests(unittest.TestCase):
    def test_target_overrides_removed_from_child_but_no_new_override(self):
        with mock.patch.dict(os.environ, {"CARGO_TARGET_DIR": "/bad", "CARGO_BUILD_TARGET_DIR": "/bad",
                                          "JUSTRUST_DISABLE": "1"}):
            env = contention.child_env("rid", True, "session")
        for key in ("CARGO_TARGET_DIR", "CARGO_BUILD_TARGET_DIR", "JUSTRUST_DISABLE"):
            self.assertNotIn(key, env)
        self.assertEqual(env["JUSTRUST_REMOTE"], "force")
        self.assertEqual(env["JUSTRUST_RUN_ID_OVERRIDE"], "rid")
        self.assertEqual(env["JUSTRUST_SLOT_SESSION"], "session")

    def test_fixture_is_content_pinned_and_prepare_owns_copy(self):
        with tempfile.TemporaryDirectory() as temp:
            fixture = Path(temp) / "fixture"
            fixture.mkdir()
            (fixture / "Cargo.toml").write_text("fixture")
            with mock.patch.object(contention, "FIXTURE", fixture), mock.patch.object(
                    suite, "benchmark_work_root", return_value=Path(temp) / "work"):
                first = contention.fixture_digest()
                root = contention.prepare(first)
                self.assertFalse((root / "Cargo.toml").is_symlink())
                (root / "Cargo.toml").write_text("changed snapshot")
                self.assertEqual((fixture / "Cargo.toml").read_text(), "fixture")
                contention.prepare(first)
                self.assertEqual((root / "Cargo.toml").read_text(), "fixture")
                (fixture / "Cargo.toml").write_text("different fixture")
                self.assertNotEqual(first, contention.fixture_digest())

    def test_isolated_acceptance_requires_ready_safe_tests_slot_and_overlap(self):
        holder = SimpleNamespace(lines=["BENCH_LOCK_READY filesystem_magic=ef53 files=12\n",
                                        "BENCH_SHARED_ARTIFACTS_UNCHANGED files=12\n"])
        self.assertEqual(contention.validate_acceptance(holder, summary(1), summary(), True, True), "ef53")
        for contender, followup, overlap in ((summary(), summary(), True),
                                            (summary(1), summary(), False),
                                            (summary(1), summary(1), True),
                                            (summary(1), {**summary(), "units": {"compiled": 1}}, True),
                                            ({**summary(1), "tests": {"passed": 1}}, summary(1), True),
                                            (summary(1), {**summary(1), "tests": {"passed": 1}}, True),
                                            ({**summary(1), "phases": {"lock_wait": 1}}, summary(1), True)):
            with self.subTest(contender=contender, overlap=overlap), self.assertRaises(ValueError):
                contention.validate_acceptance(holder, contender, followup, overlap, True)
        holder.lines.pop()
        with self.assertRaisesRegex(ValueError, "safety evidence"):
            contention.validate_acceptance(holder, summary(1), summary(), True, True)

    def test_baseline_need_not_be_isolated(self):
        holder = SimpleNamespace(lines=["BENCH_LOCK_READY filesystem_magic=ef53 files=12\n",
                                        "BENCH_SHARED_ARTIFACTS_UNCHANGED files=12\n"])
        self.assertEqual(contention.validate_acceptance(holder, summary(), summary(), False, False), "ef53")

    def test_comparison_requires_same_load_host_and_mode(self):
        before = result()
        self.assertEqual(contention.comparison_issues(before, copy.deepcopy(before)), [])
        for key, value in (("contention", {"hold_seconds": 10}), ("execution", "local"),
                           ("optimization_env", {"JUSTRUST_LOCALCACHE": "0"})):
            after = copy.deepcopy(before)
            after["env"][key] = value
            self.assertTrue(contention.comparison_issues(before, after))
        after = copy.deepcopy(before)
        after["scenarios"][contention.SCENARIO]["samples"][0]["remote"]["machine"] = "other"
        self.assertIn("different hosted machines", contention.comparison_issues(before, after))

    def test_comparison_rejects_missing_execution_and_safety_evidence(self):
        before = result()
        for key in ("holder_id", "followup_id", "tests_passed", "shared_artifacts_unchanged",
                    "remote", "filesystem_magic"):
            after = copy.deepcopy(before)
            del after["scenarios"][contention.SCENARIO]["samples"][0][key]
            with self.subTest(key=key):
                self.assertTrue(contention.comparison_issues(before, after))
        self.assertTrue(contention.comparison_issues(before, {"env": {}, "scenarios": {}}))

    def test_suite_compare_invokes_contention_protocol_gate(self):
        before = result()
        after = copy.deepcopy(before)
        after["env"]["contention"]["hold_seconds"] = 1
        with mock.patch.object(suite.Path, "read_text", side_effect=[json.dumps(before),
                                                                   json.dumps(after)]):
            with self.assertRaisesRegex(SystemExit, "contention comparison blocked"):
                suite.cmd_compare(SimpleNamespace(a="a", b="b"))


if __name__ == "__main__":
    unittest.main()
