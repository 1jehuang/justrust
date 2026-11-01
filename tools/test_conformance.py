#!/usr/bin/env python3
"""Adversarial tests of the compatibility evidence system, not compiler proofs."""
import copy
from pathlib import Path
import subprocess
import sys
import tempfile
import unittest
from unittest.mock import patch

import conformance as c


class EvidenceTests(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory(prefix="conformance-tests-")
        self.addCleanup(self.temp.cleanup)
        self.root = Path(self.temp.name)
        (self.root / "tests/support").mkdir(parents=True)
        (self.root / "tests/support/stock_rustc.rs").write_text("1.98.1 commit123 22.1.8")
        (self.root / "tests/a.rs").write_text("fn original_case() {} // unsupported discovery only\n")
        (self.root / "tests/b.rs").write_text("fn another_case() {}\n")
        self.data = {
            "schema_version": 1,
            "contract": {"edition": "2024", "target": "x86_64-unknown-linux-gnu",
                         "semantic_inventory": "incomplete", "oracle": {
                             "release": "1.98.1", "commit": "commit123", "llvm": "22.1.8",
                             "verifier": "tests/support/stock_rustc.rs"}},
            "gates": [{"id": "sample", "kind": "tests", "suites": ["a"], "profiles": ["smoke"]},
                      {"id": "all-tests", "kind": "all-tests", "profiles": ["full"]}],
            "requirements": [{"id": "ownership", "state": "partial", "gates": ["all-tests"],
                              "remaining": "General regions still open"}],
            "known_gaps": [{"id": "original", "requirement": "ownership", "suite": "a",
                            "test": "original_case", "classification": "valid-rejected"}]}
        self.identity = {"revision": "abc", "inputs_sha256": "def", "input_files": 3}

    def report(self, selected=None):
        selected = selected or ["sample"]
        entries = c.plan(self.data, "smoke", selected, "target/test")
        results = []
        for entry in entries:
            payload = b"justrust: ok: 2 tests passed in 1.0s\n"
            name = entry["id"] + ".log"
            (self.root / name).write_bytes(payload)
            results.append({**entry, "returncode": 0, "outcome": "passed", "timed_out": False,
                            "log": name, "log_sha256": c.digest(payload), "tests_passed": 2})
        return {"schema_version": 1, "manifest_sha256": c.digest(c.encoded(self.data)),
                "source_before": self.identity, "source_after": self.identity,
                "profile": "smoke", "selected": selected, "target_dir": "target/test",
                "integration_suites": ["a", "b"], "results": results}

    def assess(self, report):
        return c.assess(self.data, report, self.root, self.identity, root=self.root)

    def test_valid_scope_never_becomes_full_rust(self):
        c.validate(self.data, self.root)
        result = self.assess(self.report())
        self.assertTrue(result["selected_gates_passed"])
        self.assertFalse(result["full_gate_set_passed"])
        self.assertFalse(result["full_rust_support_established"])
        self.assertEqual(result["known_gaps"], self.data["known_gaps"])
        self.assertFalse(result["requirements"][0]["mapped_gates_passed"])

    def test_discovery_exposes_oracle_selection_without_claiming_semantic_coverage(self):
        (self.root / "tests/a.rs").write_text(
            'let a = Command::new("rustc");\n'
            'let b = std::env::var_os("RUSTC");\n'
            'let c = std::env::var("JUSTRUST_REAL_RUSTC");\n'
            'let d = Command::new(pinned_stock::stock_rustc());\n'
        )
        result = c.discover(self.root)
        self.assertEqual([row["line"] for row in result["oracle_selector_leads"]], [1, 2, 3])
        self.assertIn("require identity review", result["warning"])
        self.assertNotIn("full_rust_support_established", result)

    def test_even_green_all_tests_cannot_close_semantic_inventory(self):
        result = self.assess(self.report(["all-tests"]))
        self.assertTrue(result["full_gate_set_passed"])
        self.assertFalse(result["full_rust_support_established"])
        self.assertEqual(result["requirements"][0]["scope_state"], "partial")

    def test_missing_duplicate_reordered_and_extra_results_fail(self):
        for change in (lambda r: r["results"].clear(),
                       lambda r: r["results"].append(r["results"][0]),
                       lambda r: r["results"].reverse()):
            report = self.report(["sample", "all-tests"])
            change(report)
            self.assertFalse(self.assess(report)["selected_gates_passed"])

    def test_stale_revision_dirty_inputs_and_manifest_fail(self):
        for key in ("source_before", "source_after", "manifest_sha256"):
            report = self.report()
            report[key] = "stale"
            self.assertFalse(self.assess(report)["selected_gates_passed"])

    def test_changed_command_filters_and_suite_inventory_fail(self):
        report = self.report()
        report["results"][0]["command"].append("only_one_case")
        self.assertFalse(self.assess(report)["selected_gates_passed"])
        report = self.report()
        (self.root / "tests/new.rs").write_text("fn new_test() {}")
        self.assertFalse(self.assess(report)["selected_gates_passed"])

    def test_missing_tampered_and_escaping_logs_fail(self):
        report = self.report()
        (self.root / "sample.log").write_text("forged")
        self.assertFalse(self.assess(report)["selected_gates_passed"])
        report = self.report()
        report["results"][0]["log"] = "../elsewhere.log"
        self.assertFalse(self.assess(report)["selected_gates_passed"])
        report = self.report()
        (self.root / "sample.log").unlink()
        self.assertFalse(self.assess(report)["selected_gates_passed"])

    def test_symlink_log_outside_report_directory_fails(self):
        report = self.report()
        nested = self.root / "report"
        nested.mkdir()
        (nested / "sample.log").symlink_to(self.root / "sample.log")
        result = c.assess(self.data, report, nested, self.identity, self.root)
        self.assertFalse(result["selected_gates_passed"])

    def test_malformed_result_rows_and_unknown_report_schema_fail(self):
        report = self.report()
        report["results"] = [None]
        with self.assertRaises(c.InvalidEvidence):
            self.assess(report)
        report = self.report()
        report["schema_version"] = 200
        with self.assertRaises(c.InvalidEvidence):
            self.assess(report)

    def test_zero_match_silent_exit_and_failure_never_pass(self):
        for text in ("", "justrust: ok, but 0 tests matched the filter", "0 tests matched\n1 tests passed"):
            self.assertEqual(c.outcome(0, text, "tests"), "unverified")
        self.assertEqual(c.outcome(101, "unsupported", "tests"), "failed")
        self.assertEqual(c.outcome(None, "missing executable", "tests"), "infrastructure")
        self.assertEqual(c.outcome(None, "", "tests", True), "timeout")
        self.assertEqual(c.outcome(0, "", "fmt"), "passed")

    def test_rewritten_outcome_or_exit_code_fails(self):
        report = self.report()
        report["results"][0]["returncode"] = 101
        self.assertFalse(self.assess(report)["selected_gates_passed"])
        report = self.report()
        report["results"][0]["timed_out"] = True
        self.assertFalse(self.assess(report)["selected_gates_passed"])
        report = self.report()
        report["results"][0]["tests_passed"] = 1000000
        self.assertFalse(self.assess(report)["selected_gates_passed"])

    def test_manifest_requires_real_suites_unique_rows_and_mapped_requirements(self):
        variants = []
        data = copy.deepcopy(self.data)
        data["gates"].append(data["gates"][0])
        variants.append(data)
        data = copy.deepcopy(self.data)
        data["gates"][0]["suites"] = ["absent"]
        variants.append(data)
        data = copy.deepcopy(self.data)
        data["requirements"][0]["gates"] = ["absent"]
        variants.append(data)
        data = copy.deepcopy(self.data)
        data["requirements"][0]["state"] = "full-rust"
        variants.append(data)
        data = copy.deepcopy(self.data)
        data["contract"]["semantic_inventory"] = "complete"
        variants.append(data)
        data = copy.deepcopy(self.data)
        data["contract"]["oracle"]["commit"] = "unpinned"
        variants.append(data)
        for data in variants:
            with self.assertRaises(c.InvalidEvidence):
                c.validate(data, self.root)

    def test_plans_have_no_filters_no_shell_and_full_discovers_new_suites(self):
        args = c.plan(self.data, "full", [], "target/test")[0]["command"]
        self.assertIn("--all-targets", args)
        self.assertIn("--no-fail-fast", args)
        self.assertNotIn("--test", args)
        self.assertEqual(c.discover(self.root)["integration_suites"], ["a", "b"])
        self.assertEqual(len(c.discover(self.root)["triage_markers"]), 1)
        for selected in (["absent"], ["sample", "sample"]):
            with self.assertRaises(c.InvalidEvidence):
                c.plan(self.data, "smoke", selected, "target/test")

    def test_real_subprocess_success_failure_timeout_and_missing_tool(self):
        for script, expected in (("print('test result: ok. 2 passed;')", "passed"),
                                 ("raise SystemExit(101)", "failed"),
                                 ("print('no tests')", "unverified"),
                                 ("import time; time.sleep(10)", "timeout")):
            result = c.execute({"id": "process", "kind": "tests",
                                "command": [sys.executable, "-c", script]}, self.root, 1, self.root)
            self.assertEqual(result["outcome"], expected)
        result = c.execute({"id": "missing", "kind": "tests",
                            "command": [str(self.root / "absent")]}, self.root, 1, self.root)
        self.assertEqual(result["outcome"], "infrastructure")

    def test_source_fingerprint_binds_untracked_changes_deletions_and_revision(self):
        subprocess.run(["git", "init", "-q", str(self.root)], check=True)
        # A fixed synthetic HEAD is enough to exercise the input hasher independently.
        real = subprocess.check_output
        def git_output(args, **kwargs):
            if args[1:3] == ["rev-parse", "HEAD"]:
                return b"abc\n"
            return real(args, **kwargs)
        with patch.object(c.subprocess, "check_output", side_effect=git_output):
            first = c.source_identity(self.root)
            (self.root / "tests/a.rs").write_text("changed source")
            changed = c.source_identity(self.root)
            self.assertNotEqual(first, changed)
            (self.root / "tests/a.rs").unlink()
            self.assertNotEqual(changed, c.source_identity(self.root))


if __name__ == "__main__":
    unittest.main()
