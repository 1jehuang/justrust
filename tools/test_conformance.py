#!/usr/bin/env python3
"""Adversarial tests of the compatibility evidence system, not compiler proofs."""
import contextlib
import copy
import io
import json
import os
from pathlib import Path
import subprocess
import sys
import tempfile
import unittest
from unittest.mock import patch

import conformance as c


CLIENT_RETRY_LINES = (
    "justrust: assertions-d44df8670daae726: stock_assertion_exact_sources_and_appended_controls "
    "failed while other test processes ran in parallel and passed when rerun alone by name; "
    "not counted as failures",
    "justrust: equality_assertions-ddc5c8fac31707b3: confirming 3 failures by replaying the "
    "original test group alone instead of retrying each test serially",
)


HOSTED_REFUSAL = (
    "justrust: remote (JUSTRUST_REMOTE=force)\n"
    "justrust: remote required, refusing local fallback: Jcode API error 404 "
    "(not_found): no route for POST /internal/builds/charge\n"
)


class EvidenceTests(unittest.TestCase):
    def test_require_measured_invalid_combinations_are_rejected_before_manifest_io(self):
        actions = ("validate", "plan", "discover", "run", "assess", "track",
                   "catalog", "assess-catalog", "catalog-coverage")
        invalid = [[action, "--require-measured"] for action in actions]
        invalid += [[action, "--require-measured", "--evidence", "missing-report"]
                    for action in actions]
        invalid.append(["coverage", "--require-measured"])
        invalid += [["coverage", *flags, "--evidence", "missing-report"] for flags in (
            ["--require-measured", "--require-complete"],
            ["--require-complete", "--require-measured"])]
        for argv in invalid:
            with self.subTest(argv=argv), patch.object(c, "load_json") as load, \
                    patch.object(c, "validate") as validate, \
                    patch.object(c, "execute") as execute, \
                    contextlib.redirect_stderr(io.StringIO()):
                self.assertEqual(c.main(argv), 2)
                load.assert_not_called()
                validate.assert_not_called()
                execute.assert_not_called()

    def test_require_measured_with_evidence_reaches_coverage_dispatch(self):
        import compatibility_scope
        for evidence in (["first"], ["first", "second"]):
            argv = ["coverage", "--require-measured", "--json"]
            for directory in evidence:
                argv.extend(["--evidence", directory])
            for code in (0, 1):
                with self.subTest(evidence=evidence, code=code), \
                        patch.object(c, "load_json", return_value=self.data) as load, \
                        patch.object(c, "validate", return_value=self.data), \
                        patch.object(compatibility_scope, "main", return_value=code) as scope_main:
                    self.assertEqual(c.main(argv), code)
                    load.assert_called_once()
                    args, data, runner = scope_main.call_args.args
                    self.assertTrue(args.require_measured)
                    self.assertFalse(args.require_complete)
                    self.assertTrue(args.json)
                    self.assertEqual(args.evidence, list(map(Path, evidence)))
                    self.assertIs(data, self.data)
                    self.assertIs(runner, c)

    def test_require_complete_invalid_combinations_are_rejected_before_io(self):
        invalid = [[action, "--require-complete"]
                   for action in ("validate", "plan", "discover", "run", "assess", "track")]
        invalid += [[action, "--require-complete", "--evidence", "missing-report"]
                    for action in ("validate", "plan", "discover", "run", "assess", "track")]
        invalid.append(["coverage", "--require-complete"])
        for argv in invalid:
            with self.subTest(argv=argv), patch.object(c, "load_json") as load, \
                    contextlib.redirect_stderr(io.StringIO()) as errors:
                self.assertEqual(c.main(argv), 2)
                load.assert_not_called()
                self.assertIn("--require-complete", errors.getvalue())

    def test_require_complete_with_evidence_reaches_coverage_dispatch(self):
        import compatibility_scope
        for evidence in (["first"], ["first", "second"]):
            argv = ["coverage", "--require-complete", "--json"]
            for directory in evidence:
                argv.extend(["--evidence", directory])
            for code in (0, 1):
                with self.subTest(evidence=evidence, code=code), \
                        patch.object(c, "load_json", return_value=self.data) as load, \
                        patch.object(c, "validate", return_value=self.data), \
                        patch.object(compatibility_scope, "main", return_value=code) as scope_main:
                    self.assertEqual(c.main(argv), code)
                    load.assert_called_once()
                    args, data, runner = scope_main.call_args.args
                    self.assertTrue(args.require_complete)
                    self.assertTrue(args.json)
                    self.assertEqual(args.evidence, list(map(Path, evidence)))
                    self.assertIs(data, self.data)
                    self.assertIs(runner, c)

    def test_failure_annotations_require_whole_whitelisted_lines(self):
        prefix = "[justrust-conformance-failure:v1] "
        for annotation in c.FAILURE_ANNOTATIONS:
            marker = prefix + annotation
            for text in (marker, marker + "\n", "noise\n" + marker + "\r\n", marker + "\n" + marker):
                with self.subTest(text=text):
                    self.assertEqual(c.failure_annotation(text), annotation)
            for text in (" " + marker, marker + " ", "panic: " + marker,
                         marker + " details", marker.upper(), prefix + "unknown",
                         marker.replace(":v1", ":v2"), "noise\u2028" + marker,
                         '"' + marker + '"', marker + "\x1b[0m"):
                with self.subTest(text=text):
                    self.assertEqual(c.failure_annotation(text), "unannotated")
            for code, expected in ((0, "passed"), (101, "failed")):
                self.assertEqual(c.outcome(code, marker + "\njustrust: ok: 1 test passed\n", "case"), expected)
            self.assertEqual(c.outcome(0, marker, "case"), "unverified")
        self.assertEqual(c.failure_annotation("legacy failure"), "unannotated")
        self.assertEqual(c.failure_annotation("\n".join(prefix + value for value in c.FAILURE_ANNOTATIONS)),
                         "unannotated")

    def test_shard_is_rejected_for_every_other_action_before_io(self):
        for action in ("validate", "plan", "discover", "run", "assess", "coverage"):
            with self.subTest(action=action), patch.object(c, "load_json") as load, \
                    contextlib.redirect_stderr(io.StringIO()):
                self.assertEqual(c.main([action, "--shard", "0/2"]), 2)
                load.assert_not_called()

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

    def test_client_retried_or_ignored_failures_never_pass(self):
        for marker in CLIENT_RETRY_LINES:
            for newline in ("\n", "\r\n"):
                text = marker + newline + "test result: ok. 9 passed;" + newline
                self.assertEqual(c.test_count(text), 9)
                for kind in ("tests", "lib", "all-tests", "fmt", "clippy"):
                    self.assertEqual(c.outcome(0, text, kind), "unverified")
                    self.assertEqual(c.outcome(1, text, kind), "failed")

    def test_hosted_admission_refusal_is_unmeasured_infrastructure(self):
        for text in (HOSTED_REFUSAL, HOSTED_REFUSAL.rstrip("\n"),
                     HOSTED_REFUSAL.replace("\n", "\r\n"),
                     HOSTED_REFUSAL.replace("404 (not_found)", "503 (unavailable)")):
            for kind in ("tests", "lib", "all-tests", "case", "clippy"):
                with self.subTest(text=text, kind=kind):
                    self.assertTrue(c.hosted_admission_refusal(text))
                    self.assertEqual(c.outcome(1, text, kind), "infrastructure")
                    self.assertEqual(c.outcome(1, text, kind, True), "timeout")
                    self.assertEqual(c.outcome(0, text, kind), "unverified")
                    self.assertEqual(c.outcome(101, text, kind), "failed")
        self.assertEqual(c.outcome(1, HOSTED_REFUSAL, "fmt"), "failed")

    def test_ambiguous_or_executed_hosted_failure_stays_failed(self):
        variants = (HOSTED_REFUSAL.split("\n", 1)[1],
                    "quoted user source: " + HOSTED_REFUSAL,
                    HOSTED_REFUSAL.replace("404 (not_found)", "200 (ok)"),
                    HOSTED_REFUSAL.replace("refusing local fallback", "using local fallback"),
                    HOSTED_REFUSAL + "\n", HOSTED_REFUSAL.replace("\n", "\u2028"))
        variants += tuple(HOSTED_REFUSAL + extra for extra in (
            "running 1 test\n", "test original_case ... FAILED\n",
            "test result: FAILED. 0 passed; 1 failed;\n",
            "justrust: ok: 1 test passed in 1s\n", "error: compiler failure\n"))
        for text in variants:
            with self.subTest(text=text):
                self.assertFalse(c.hosted_admission_refusal(text))
                self.assertEqual(c.outcome(1, text, "case"), "failed")

    def test_hosted_refusal_receipt_reassessment_never_establishes_success(self):
        report = self.report()
        payload = HOSTED_REFUSAL.encode()
        (self.root / "sample.log").write_bytes(payload)
        row = report["results"][0]
        row.update(returncode=1, outcome="infrastructure", tests_passed=0,
                   log_sha256=c.digest(payload))
        result = self.assess(report)
        self.assertFalse(result["selected_gates_passed"])
        self.assertFalse(result["full_rust_support_established"])
        self.assertIn("sample: infrastructure", result["problems"])
        row["outcome"] = "passed"
        self.assertFalse(self.assess(report)["selected_gates_passed"])

    def test_real_hosted_refusal_subprocess_preserves_command_and_full_log(self):
        payload = HOSTED_REFUSAL.encode()
        command = [sys.executable, "-c", "import sys; sys.stderr.buffer.write(" + repr(payload)
                   + "); sys.stderr.flush(); raise SystemExit(1)"]
        entry = {"id": "refused", "kind": "case", "command": command}
        visible = io.StringIO()
        with contextlib.redirect_stdout(visible):
            result = c.execute(entry, self.root, 5, self.root)
        self.assertEqual(result["command"], command)
        self.assertEqual(result["returncode"], 1)
        self.assertEqual(result["outcome"], "infrastructure")
        self.assertEqual(result["tests_passed"], 0)
        self.assertEqual((self.root / result["log"]).read_bytes(), payload)
        self.assertEqual(result["log_sha256"], c.digest(payload))
        self.assertIn(HOSTED_REFUSAL, visible.getvalue())

    def test_retry_words_user_identifiers_and_quoted_markers_do_not_false_positive(self):
        text = "\n".join([
            "test user::retry::not_counted_as_failures ... ok",
            "test user::failed_while_other_test_processes_ran_in_parallel ... ok",
            "test user::passed_when_rerun_alone_by_name ... ok",
            "test user::confirming_failures_by_replaying ... ok",
            "ordinary message: not counted as failures",
            "quoted user source: " + CLIENT_RETRY_LINES[0],
            repr(CLIENT_RETRY_LINES[1]),
            "justrust: ok: 7 tests passed in 1.0s",
        ])
        self.assertFalse(c.client_retried_failures(text))
        self.assertEqual(c.outcome(0, text, "all-tests"), "passed")

    def test_assessment_recomputes_client_retry_status_from_original_log(self):
        for marker in CLIENT_RETRY_LINES:
            report = self.report()
            payload = (marker + "\njustrust: ok: 2 tests passed in 1.0s\n").encode()
            (self.root / "sample.log").write_bytes(payload)
            report["results"][0]["log_sha256"] = c.digest(payload)
            # A client success cannot be laundered by retaining a forged passed row.
            self.assertFalse(self.assess(report)["selected_gates_passed"])
            report["results"][0]["outcome"] = "unverified"
            result = self.assess(report)
            self.assertFalse(result["selected_gates_passed"])
            self.assertIn("sample: unverified", result["problems"])

    def test_client_retry_command_and_full_failure_log_are_preserved(self):
        for index, marker in enumerate(CLIENT_RETRY_LINES):
            for code in (0, 1):
                stdout = b"original stdout: \xff\n" + b"x" * 70000 + b"\n"
                stderr = marker.encode() + b"\njustrust: ok: 2 tests passed in 1.0s\n"
                payload = stdout + stderr
                command = [sys.executable, "-c",
                           "import sys; sys.stdout.buffer.write(" + repr(stdout)
                           + "); sys.stdout.flush(); sys.stderr.buffer.write(" + repr(stderr)
                           + "); sys.stderr.flush(); raise SystemExit(" + str(code) + ")"]
                entry = {"id": f"retry-{index}-{code}", "kind": "all-tests", "command": command}
                visible = io.StringIO()
                with contextlib.redirect_stdout(visible):
                    result = c.execute(entry, self.root, 5, self.root)
                self.assertEqual(result["command"], command)
                self.assertEqual(result["returncode"], code)
                self.assertEqual(result["outcome"], "unverified" if code == 0 else "failed")
                self.assertEqual(result["tests_passed"], 2)
                self.assertEqual((self.root / result["log"]).read_bytes(), payload)
                self.assertEqual(result["log_sha256"], c.digest(payload))
                self.assertIn(payload.decode(errors="replace"), visible.getvalue())

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
            (self.root / "data").mkdir(exist_ok=True)
            for filename in ("compiler-compatibility-contract.json",
                             "compiler-language-inventory.json",
                             "compiler-language-reference-rules.json",
                             "compiler-library-surface-provenance.json",
                             "compiler-library-project-inventory.json"):
                before = c.source_identity(self.root)
                path = self.root / "data" / filename
                path.write_text('{"revision": 1}')
                added = c.source_identity(self.root)
                self.assertNotEqual(before, added, filename)
                path.write_text('{"revision": 2}')
                self.assertNotEqual(added, c.source_identity(self.root), filename)


class SnapshotTests(unittest.TestCase):
    """Tiny real Git objects and subprocesses, not Rust compatibility proofs."""

    def setUp(self):
        self.temp = tempfile.TemporaryDirectory(prefix="conformance-snapshot-")
        self.addCleanup(self.temp.cleanup)
        self.base = Path(self.temp.name)
        self.repo = self.base / "repo"
        self.repo.mkdir()
        self.git("init", "-q")
        self.write("tests/a.rs", "#[test]\nfn accepts() {}\n")
        self.write("tests/support/stock_rustc.rs", "release commit llvm")
        self.write("src/input.txt", "committed\n")
        self.write("run.sh", "#!/bin/sh\n# $Format:%H$\nexit 0\n")
        (self.repo / "run.sh").chmod(0o755)
        self.write(".gitattributes", "src/input.txt export-ignore\nrun.sh export-subst\n")
        self.manifest = {"schema_version": 1, "contract": {
            "edition": "2024", "target": "x86_64-unknown-linux-gnu", "semantic_inventory": "incomplete",
            "oracle": {"release": "release", "commit": "commit", "llvm": "llvm",
                       "verifier": "tests/support/stock_rustc.rs"}},
            "gates": [{"id": "sample", "kind": "tests", "suites": ["a"], "profiles": ["full"]}],
            "requirements": [{"id": "sample", "state": "partial", "gates": ["sample"], "remaining": "open"}],
            "known_gaps": []}
        self.scope = {"schema_version": 1, "scope": self.manifest["contract"], "areas": [
            {"id": "sample", "title": "Sample", "requirements": [
                {"id": "sample", "title": "Sample", "gates": [
                    {"layer": layer, "tests": [{"suite": "a", "test": "accepts"}]}
                    for layer in ("feature", "rejection", "interaction", "end_to_end")]}]}]}
        self.write(str(c.MANIFEST), c.encoded(self.manifest))
        self.write("data/compiler-compatibility-scope.json", c.encoded(self.scope))
        self.write("tests/archive_case.py", """import pathlib, unittest
class ArchivedCase(unittest.TestCase):
    def test_source(self):
        self.assertEqual(pathlib.Path('src/input.txt').read_text(), 'committed\\n')
unittest.main()
""")
        self.commit = self.commit_all()
        self.directory = self.base / "snapshot"
        self.client_dir = self.base / "bin"
        self.client_dir.mkdir()
        self.client = self.client_dir / "justrust"
        self.client.write_text("#!" + sys.executable + "\n" + """import os, pathlib, subprocess, sys
root = pathlib.Path.cwd()
assert root == pathlib.Path(os.environ['EXPECTED_SNAPSHOT_ROOT'])
assert (root / 'src/input.txt').read_text() == 'committed\\n'
result = subprocess.run([sys.executable, '-B', 'tests/archive_case.py'])
if result.returncode:
    sys.exit(result.returncode)
if os.environ.get('MUTATE_SNAPSHOT'):
    path = root / 'src/input.txt'
    path.chmod(0o644)
    path.write_text('late drift\\n')
    path.chmod(0o444)
print('test result: ok. 1 passed; 0 failed;')
""")
        self.client.chmod(0o755)
        self.environment = patch.dict(os.environ, {
            "PATH": str(self.client_dir) + os.pathsep + os.environ.get("PATH", ""),
            "EXPECTED_SNAPSHOT_ROOT": str(self.directory / "source")})
        self.environment.start()
        self.addCleanup(self.environment.stop)

    def git(self, *args, payload=None):
        return subprocess.check_output(["git", "-C", str(self.repo), *args], input=payload,
                                       stderr=subprocess.PIPE)

    def write(self, name, value):
        path = self.repo / name
        path.parent.mkdir(parents=True, exist_ok=True)
        path.write_bytes(value if isinstance(value, bytes) else value.encode())

    def commit_all(self):
        self.git("add", ".")
        self.git("-c", "user.name=Snapshot fixture", "-c", "user.email=fixture@example.invalid",
                 "commit", "-qm", "fixture")
        return self.git("rev-parse", "HEAD").decode().strip()

    def snapshot(self, create=True, commit=None):
        snapshot = c.GitSnapshot(self.directory, self.repo, commit or self.commit)
        if create:
            snapshot.create()
        return snapshot

    def argv(self, action="track", *extra):
        return [action, "--snapshot", str(self.directory), "--snapshot-repository", str(self.repo),
                "--snapshot-commit", self.commit, "--target-dir", str(self.base / "target"), *extra]

    def main(self, argv):
        output, errors = io.StringIO(), io.StringIO()
        with contextlib.redirect_stdout(output), contextlib.redirect_stderr(errors):
            code = c.main(argv)
        return code, output.getvalue(), errors.getvalue()

    def test_exact_objects_attributes_modes_and_live_changes(self):
        snapshot = self.snapshot()
        with snapshot:
            before = snapshot.source_identity()
            self.assertEqual(before["tree"], self.git("rev-parse", self.commit + "^{tree}").decode().strip())
            self.assertEqual(before["provenance"], "git-archive-v1")
            self.assertEqual(before["executed_runner"]["conformance.py"], c.digest(Path(c.__file__).read_bytes()))
            self.assertEqual((snapshot.ROOT / "src/input.txt").read_text(), "committed\n")
            self.assertEqual((snapshot.ROOT / "run.sh").stat().st_mode & 0o777, 0o555)
            self.assertIn("$Format:%H$", (snapshot.ROOT / "run.sh").read_text())
            self.assertEqual((snapshot.ROOT / "src/input.txt").stat().st_mode & 0o777, 0o444)
            self.write("src/input.txt", "other owner's live edit\n")
            self.write("src/extra.txt", "untracked live input\n")
            self.commit_all()
            self.assertEqual(snapshot.source_identity(), before)
            self.assertNotEqual(c.source_identity(self.repo), before)
            self.assertFalse((snapshot.ROOT / ".git").exists())

    def test_snapshot_rejects_changed_missing_extra_modes_and_symlinks(self):
        snapshot = self.snapshot()
        with snapshot:
            path = snapshot.ROOT / "src/input.txt"
            parent = path.parent
            for mutation in ("bytes", "missing", "extra", "mode", "symlink", "directory"):
                with self.subTest(mutation=mutation):
                    parent.chmod(0o700)
                    path.chmod(0o644)
                    if mutation == "bytes":
                        path.write_text("forged\n")
                        path.chmod(0o444)
                    elif mutation == "missing":
                        path.unlink()
                    elif mutation == "extra":
                        (parent / "extra.rs").write_text("ignored or untracked still forbidden")
                        path.chmod(0o444)
                    elif mutation == "mode":
                        path.chmod(0o555)
                    elif mutation == "symlink":
                        path.unlink()
                        path.symlink_to(self.repo / "src/input.txt")
                    else:
                        (parent / "extra-directory").mkdir()
                        (parent / "extra-directory").chmod(0o500)
                        path.chmod(0o444)
                    parent.chmod(0o500)
                    with self.assertRaises((c.InvalidEvidence, OSError)):
                        snapshot.source_identity()
                    parent.chmod(0o700)
                    if path.exists() or path.is_symlink():
                        path.unlink()
                    path.write_text("committed\n")
                    path.chmod(0o444)
                    if (parent / "extra.rs").exists():
                        (parent / "extra.rs").unlink()
                    if (parent / "extra-directory").exists():
                        (parent / "extra-directory").rmdir()
                    parent.chmod(0o500)
                    snapshot.source_identity()

    def test_lock_excludes_other_campaigns_and_rejects_foreign_lock(self):
        snapshot = self.snapshot()
        with snapshot:
            with self.assertRaises(BlockingIOError):
                with self.snapshot(create=False):
                    self.fail("concurrent campaign acquired snapshot")
        lock = self.directory / "owner.lock"
        lock.unlink()
        lock.symlink_to(self.client)
        with self.assertRaises(c.InvalidEvidence):
            with self.snapshot(create=False):
                pass

    def test_lock_inode_replacement_and_foreign_owner_rejected(self):
        snapshot = self.snapshot()
        with snapshot:
            lock = self.directory / "owner.lock"
            value = lock.read_bytes()
            lock.rename(self.directory / "old.lock")
            lock.write_bytes(value)
            lock.chmod(0o600)
            with self.assertRaisesRegex(c.InvalidEvidence, "lock replaced"):
                snapshot.source_identity()
        with patch.object(c.os, "getuid", return_value=os.getuid() + 1):
            with self.assertRaises(c.InvalidEvidence):
                with self.snapshot(create=False):
                    pass

    def test_forged_commit_tree_and_git_environment_do_not_select_other_source(self):
        with self.assertRaises(c.InvalidEvidence):
            self.snapshot(commit="0" * 40)
        self.assertFalse(self.directory.exists())
        for commit in ("HEAD", self.commit + "^{tree}", "../HEAD", "-" * 40):
            with self.assertRaises(c.InvalidEvidence):
                c.GitSnapshot(self.directory, self.repo, commit)
        tree = self.git("rev-parse", self.commit + "^{tree}").decode().strip()
        with self.assertRaises(c.InvalidEvidence):
            self.snapshot(commit=tree)
        self.write("src/input.txt", "replacement\n")
        replacement = self.commit_all()
        self.git("replace", self.commit, replacement)
        with patch.dict(os.environ, {"GIT_DIR": "/absent", "GIT_WORK_TREE": "/absent",
                                     "GIT_OBJECT_DIRECTORY": "/absent", "GIT_INDEX_FILE": "/absent"}):
            with self.snapshot() as snapshot:
                self.assertEqual((snapshot.ROOT / "src/input.txt").read_text(), "committed\n")
                identity = snapshot.source_identity()
                self.assertEqual(identity["revision"], self.commit)
                self.assertEqual(identity["tree"], tree)

    def test_unsupported_symlink_and_gitlink_objects_fail_before_creation(self):
        (self.repo / "unsafe").symlink_to("/etc/passwd")
        commit = self.commit_all()
        with self.assertRaisesRegex(c.InvalidEvidence, "unsupported"):
            self.snapshot(commit=commit)
        self.assertFalse(self.directory.exists())
        (self.repo / "unsafe").unlink()
        self.git("add", "-u")
        self.git("update-index", "--add", "--cacheinfo", "160000," + self.commit + ",submodule")
        self.git("-c", "user.name=Snapshot fixture", "-c", "user.email=fixture@example.invalid",
                 "commit", "-qm", "gitlink")
        commit = self.git("rev-parse", "HEAD").decode().strip()
        with self.assertRaisesRegex(c.InvalidEvidence, "unsupported"):
            self.snapshot(commit=commit)

    def test_real_tracker_subprocess_and_retained_coverage_after_live_head_changes(self):
        out = self.base / "receipts"
        code, _, errors = self.main(self.argv("track", "--create-snapshot", "--out", str(out)))
        self.assertEqual((code, errors), (0, ""))
        report = c.load_json(out / "report.json")
        self.assertEqual(report["source_before"], report["source_after"])
        self.assertEqual(report["source_before"]["provenance"], "git-archive-v1")
        self.assertEqual(report["results"][0]["tests_passed"], 1)
        self.assertIn("Ran 1 test", (out / report["results"][0]["log"]).read_text())
        self.write("src/input.txt", "different live compiler\n")
        self.commit_all()
        code, output, errors = self.main(self.argv("coverage", "--evidence", str(out), "--require-complete", "--json"))
        self.assertEqual((code, errors), (0, ""))
        self.assertTrue(json.loads(output)["campaign"]["all_cases_passed"])
        import compatibility_scope as scope
        manifest = scope.case_manifest(self.scope, self.manifest, c)
        runner = scope.runner_identity(c)
        values = scope.evidence(self.scope, manifest, out, c.source_identity(self.repo), runner, c, self.repo)
        self.assertTrue(all(row["status"] == "stale" for row in values.values()))
        report["source_before"]["tree"] = "f" * 40
        (out / "report.json").write_bytes(c.encoded(report))
        code, output, _ = self.main(self.argv("coverage", "--evidence", str(out), "--require-measured", "--json"))
        self.assertEqual(code, 1)
        self.assertFalse(json.loads(output)["campaign"]["complete_current_measurement"])

    def test_late_source_drift_never_finalizes_successful_receipt(self):
        out = self.base / "receipts"
        with patch.dict(os.environ, {"MUTATE_SNAPSHOT": "1"}):
            code, _, errors = self.main(self.argv("track", "--create-snapshot", "--out", str(out)))
        self.assertEqual(code, 2)
        self.assertIn("source bytes changed", errors)
        report = c.load_json(out / "report.json")
        self.assertNotIn("source_after", report)
        self.assertFalse((out / "coverage.json").exists())

    def test_snapshot_cli_refuses_incomplete_flags_and_unsafe_paths_before_execution(self):
        invalid = [["track", "--snapshot", str(self.directory)],
                   ["coverage", "--create-snapshot"],
                   self.argv("run"),
                   self.argv("track", "--target-dir", str(self.repo / "target")),
                   self.argv("track", "--out", str(self.repo / ".git/receipts")),
                   self.argv("track", "--target-dir", str(self.directory / "source/target"))]
        for argv in invalid:
            with self.subTest(argv=argv), patch.object(c, "execute") as execute:
                self.assertEqual(self.main(argv)[0], 2)
                execute.assert_not_called()
        self.snapshot()
        code, _, errors = self.main(self.argv("track", "--manifest", str(self.repo / c.MANIFEST),
                                              "--out", str(self.base / "unused-receipts")))
        self.assertEqual(code, 2)
        self.assertIn("manifest/scope", errors)

    def tree_commit(self, tree):
        return self.git("-c", "user.name=Snapshot fixture", "-c", "user.email=fixture@example.invalid",
                        "commit-tree", tree, "-p", self.commit, "-m", "adversarial tree").decode().strip()

    def test_empty_committed_subtree_and_parent_traversal_are_explicitly_rejected(self):
        empty = self.git("mktree", payload=b"").strip()
        listing = self.git("ls-tree", self.commit) + b"040000 tree " + empty + b"\tempty\n"
        tree = self.git("mktree", payload=listing).decode().strip()
        with self.assertRaisesRegex(c.InvalidEvidence, "empty Git subtree"):
            self.snapshot(commit=self.tree_commit(tree))
        blob = self.git("rev-parse", self.commit + ":src/input.txt").decode().strip()
        for name in (b"../escape", b"/absolute", b".git/config"):
            tree_bytes = b"100644 " + name + b"\0" + bytes.fromhex(blob)
            tree = self.git("hash-object", "-w", "-t", "tree", "--literally", "--stdin", payload=tree_bytes).decode().strip()
            with self.subTest(name=name), self.assertRaisesRegex(c.InvalidEvidence, "unsafe"):
                self.snapshot(commit=self.tree_commit(tree))
        self.assertFalse(self.directory.exists())

    def test_size_and_disk_limits_fail_before_materialization(self):
        snapshot = self.snapshot(create=False)
        real_git = snapshot.git
        def oversized(*args, **kwargs):
            value = real_git(*args, **kwargs)
            if args[0] == "ls-tree":
                row, rest = value.split(b"\0", 1)
                header, name = row.split(b"\t", 1)
                value = header.rsplit(b" ", 1)[0] + b" 536870913\t" + name + b"\0" + rest
            return value
        with patch.object(snapshot, "git", side_effect=oversized):
            with self.assertRaisesRegex(c.InvalidEvidence, "infrastructure limit"):
                snapshot.create()
        self.assertFalse(self.directory.exists())
        from types import SimpleNamespace
        with patch.object(c.shutil, "disk_usage", return_value=SimpleNamespace(free=0)):
            with self.assertRaisesRegex(c.InvalidEvidence, "free disk"):
                snapshot.create()
        self.assertFalse(self.directory.exists())

    def test_replaced_source_root_and_mutated_lock_token_never_authenticate(self):
        snapshot = self.snapshot()
        with snapshot:
            root = snapshot.ROOT
            root.chmod(0o700)
            root.rename(self.base / "old-source")
            (self.base / "old-source").chmod(0o500)
            root.mkdir(mode=0o500)
            with self.assertRaisesRegex(c.InvalidEvidence, "source directory replaced"):
                snapshot.source_identity()
            root.rmdir()
            (self.base / "old-source").chmod(0o700)
            (self.base / "old-source").rename(root)
            root.chmod(0o500)
            lock = self.directory / "owner.lock"
            value = lock.read_bytes()
            lock.write_bytes(value[:-2] + (b"0" if value[-2:-1] != b"0" else b"1") + b"\n")
            with self.assertRaisesRegex(c.InvalidEvidence, "ownership token changed"):
                snapshot.source_identity()

    def test_wrong_snapshot_cwd_cannot_gain_execution_credit(self):
        out = self.base / "receipts"
        with patch.dict(os.environ, {"EXPECTED_SNAPSHOT_ROOT": str(self.repo)}):
            code, _, errors = self.main(self.argv("track", "--create-snapshot", "--out", str(out)))
        self.assertEqual((code, errors), (1, ""))
        report = c.load_json(out / "report.json")
        self.assertEqual(report["results"][0]["outcome"], "failed")
        self.assertFalse(c.load_json(out / "coverage.json")["campaign"]["all_cases_passed"])

    def test_invalid_snapshot_does_not_fall_back_to_live_git_identity(self):
        self.snapshot()
        (self.directory / "owner.lock").chmod(0o666)
        with patch.object(c, "source_identity", side_effect=AssertionError("no legacy fallback")):
            self.assertEqual(self.main(self.argv("coverage", "--evidence", str(self.base / "absent")))[0], 2)

    def test_legacy_identity_function_and_default_receipt_schema_remain_exact(self):
        # Independent reference for the original legacy wire format.
        legacy = c.source_identity(self.repo)
        self.assertEqual(set(legacy), {"revision", "inputs_sha256", "input_files"})
        self.assertEqual(legacy["revision"], self.commit)
        self.assertNotIn("provenance", legacy)
        self.write("src/untracked.rs", "tracked only by legacy source inventory\n")
        self.assertNotEqual(c.source_identity(self.repo), legacy)

    def test_threaded_tracker_cases_share_one_exclusively_owned_snapshot(self):
        self.write("tests/a.rs", "#[test]\nfn accepts() {}\n#[test]\nfn rejects() {}\n")
        for gate in self.scope["areas"][0]["requirements"][0]["gates"]:
            gate["tests"].append({"suite": "a", "test": "rejects"})
        self.write("data/compiler-compatibility-scope.json", c.encoded(self.scope))
        self.commit = self.commit_all()
        out = self.base / "threaded-receipts"
        code, _, errors = self.main(self.argv("track", "--create-snapshot", "--jobs", "2", "--out", str(out)))
        self.assertEqual((code, errors), (0, ""))
        report = c.load_json(out / "report.json")
        self.assertEqual(len(report["results"]), 2)
        self.assertTrue(all(row["outcome"] == "passed" for row in report["results"]))
        self.assertEqual(report["source_before"], report["source_after"])
        self.assertTrue(c.load_json(out / "coverage.json")["campaign"]["all_cases_passed"])

    def test_retained_snapshot_authenticates_in_fresh_process_with_different_hash_seed(self):
        out = self.base / "cross-process-receipts"
        argv = self.argv("track", "--create-snapshot", "--out", str(out))
        with patch.dict(os.environ, {"PYTHONHASHSEED": "1"}):
            process = subprocess.run([sys.executable, "-B", str(Path(c.__file__).resolve()), *argv],
                                     capture_output=True, text=True)
        self.assertEqual(process.returncode, 0, process.stdout + process.stderr)
        argv = self.argv("coverage", "--evidence", str(out), "--require-complete", "--json")
        with patch.dict(os.environ, {"PYTHONHASHSEED": "2"}):
            process = subprocess.run([sys.executable, "-B", str(Path(c.__file__).resolve()), *argv],
                                     capture_output=True, text=True)
        self.assertEqual(process.returncode, 0, process.stdout + process.stderr)
        self.assertTrue(json.loads(process.stdout)["campaign"]["all_cases_passed"])

    def test_loaded_runner_identity_rejects_foreign_patched_code(self):
        import compatibility_scope
        import compatibility_inventory
        for module, name in ((c, "execute"), (compatibility_scope, "evidence"),
                             (compatibility_inventory, "load_and_summarize")):
            with self.subTest(module=module.__name__), patch.object(module, name, lambda *args: None):
                with self.assertRaisesRegex(c.InvalidEvidence, "foreign loaded runner callable"):
                    c.loaded_runner_identity(module)
        with patch.object(c.GitSnapshot, "check_lock", lambda *args: None):
            with self.assertRaisesRegex(c.InvalidEvidence, "GitSnapshot.check_lock"):
                c.loaded_runner_identity(c)

    def test_loaded_runner_identity_rejects_source_changed_after_import(self):
        from types import ModuleType
        path = self.base / "runner_fixture.py"
        path.write_text("def run():\n    return 1\n")
        module = ModuleType("runner_fixture")
        module.__file__ = str(path)
        exec(compile(path.read_bytes(), str(path), "exec"), vars(module))
        before = c.loaded_runner_identity(module)
        self.assertIn("run", before["loaded_code_sha256"])
        path.write_text("def run():\n    return 2\n")
        with self.assertRaisesRegex(c.InvalidEvidence, "stale or foreign"):
            c.loaded_runner_identity(module)

    def test_pure_invalid_snapshot_flags_never_materialize(self):
        for flags in (("--require-measured",), ("--require-complete",),
                      ("--jobs", "9"), ("--timeout", "0"), ("--suite", "a")):
            with self.subTest(flags=flags), patch.object(c.GitSnapshot, "create") as create:
                code, _, _ = self.main(self.argv("track", "--create-snapshot", "--out", str(self.base / "absent"), *flags))
                self.assertEqual(code, 2)
                create.assert_not_called()
        self.assertFalse(self.directory.exists())


class CatalogTests(unittest.TestCase):
    """Libtest enumeration authenticates names, never execution credit."""

    def setUp(self):
        import compatibility_catalog
        self.catalog = compatibility_catalog
        temp = tempfile.TemporaryDirectory(prefix="catalog-tests-")
        self.addCleanup(temp.cleanup)
        self.root = Path(temp.name)
        (self.root / "tests").mkdir()
        (self.root / "tests/a.rs").write_text(
            "#[test]\nfn lexical() {}\n#[test]\nfn absent() {}\n")
        (self.root / "tests/b.rs").write_text("")
        self.identity = {"revision": "synthetic", "inputs_sha256": "source"}
        self.runner = {"executable": "/synthetic/justrust", "sha256": "client"}
        self.scope = {"areas": [{"requirements": [{"gates": [{"tests": [
            {"suite": "a", "test": "lexical"},
            {"suite": "a", "test": "absent"}]}]}]}]}
        self.text = "lexical: test\nmacro_generated::nested::case: test\n\n2 tests, 0 benchmarks\n"

    def receipt(self, payload=None, code=0, timeout=False):
        payload = self.text.encode() if payload is None else payload
        invocation = self.catalog.entry("a", "target/catalog")
        name = invocation["id"] + ".log"
        (self.root / name).write_bytes(payload)
        text = payload.decode(errors="replace")
        row = {**invocation, "returncode": code, "timed_out": timeout,
               "seconds": 0.125, "tests_passed": c.test_count(text),
               "outcome": c.outcome(code, text, "tests", timeout),
               "log": name, "log_sha256": c.digest(payload)}
        return {"schema_version": 1, "purpose": self.catalog.PURPOSE,
                "selected_suites": ["a"], "target_dir": "target/catalog",
                "integration_suites": ["a", "b"],
                "source_before": self.identity, "source_after": self.identity,
                "runner_before": self.runner, "runner_after": self.runner,
                "results": [row]}

    def assess(self, report):
        return self.catalog.assess(report, self.root, c, self.root,
                                   self.scope, self.identity, self.runner)

    def assert_inventory_only(self, result):
        self.assertEqual(result["executed_compatibility_case_count"], 0)
        self.assertIs(result["full_rust_support_established"], False)
        self.assertIsNone(result["full_rust_completion_percent"])
        self.assertNotIn("selected_gates_passed", result)
        self.assertNotIn("full_gate_set_passed", result)
        self.assertNotIn("passed", [row["status"] for row in result["suites"]])

    def test_default_crlf_macro_nested_and_empty_listings(self):
        for text in (self.text, self.text.replace("\n", "\r\n")):
            with self.subTest(text=text):
                self.assertEqual(self.catalog.listing(text, c), {
                    "tests": ["lexical", "macro_generated::nested::case"], "benchmarks": []})
        self.assertEqual(self.catalog.listing("0 tests, 0 benchmarks\n", c),
                         {"tests": [], "benchmarks": []})
        self.assertEqual(self.catalog.listing("bench: benchmark\n0 tests, 1 benchmark\n", c),
                         {"tests": [], "benchmarks": ["bench"]})

    def test_missing_duplicate_mismatched_reordered_and_execution_listings_rejected(self):
        invalid = ["lexical: test\n", self.text + "0 tests, 0 benchmarks\n",
                   self.text.replace("2 tests", "3 tests"),
                   "lexical: test\nlexical: test\n2 tests, 0 benchmarks\n",
                   "2 tests, 0 benchmarks\nlexical: test\nmacro_generated::nested::case: test\n",
                   "running 0 tests\n0 tests, 0 benchmarks\n",
                   "justrust: ok: 0 tests passed\n0 tests, 0 benchmarks\n",
                   "0 tests matched\n" + self.text,
                   self.text + "test result: ok. 2 passed; 0 failed;\n"]
        invalid += [marker + "\n" + self.text for marker in CLIENT_RETRY_LINES]
        for text in invalid:
            with self.subTest(text=text), self.assertRaises(c.InvalidEvidence):
                self.catalog.listing(text, c)

    def test_authentic_unverified_log_lists_without_execution_credit(self):
        report = self.receipt()
        row = report["results"][0]
        self.assertEqual(row["outcome"], "unverified")
        self.assertEqual(row["tests_passed"], 0)
        result = self.assess(report)
        self.assert_inventory_only(result)
        self.assertEqual(result["purpose"], "libtest-selector-catalog-v1")
        self.assertTrue(result["all_selected_suites_listed"])
        self.assertFalse(result["all_selected_mapped_tests_listed"])
        self.assertEqual(result["listed_test_count"], 2)
        observed = result["suites"][0]
        self.assertEqual(observed["lexical_only"], ["absent"])
        self.assertEqual(observed["listed_only"], ["macro_generated::nested::case"])
        self.assertEqual(observed["mapped_not_listed"], ["absent"])

    def test_zero_suite_is_valid_inventory_not_executed(self):
        result = self.assess(self.receipt(b"0 tests, 0 benchmarks\n"))
        self.assertTrue(result["all_selected_suites_listed"])
        self.assertEqual(result["listed_test_count"], 0)
        self.assert_inventory_only(result)

    def test_exact_receipt_schema_and_finalization(self):
        for key in self.receipt():
            report = self.receipt()
            del report[key]
            with self.subTest(missing=key), self.assertRaises(c.InvalidEvidence):
                self.assess(report)
        for key, value in (("extra", True), ("schema_version", True),
                           ("schema_version", 2), ("purpose", "execution")):
            report = self.receipt()
            report[key] = value
            with self.subTest(key=key, value=value), self.assertRaises(c.InvalidEvidence):
                self.assess(report)
        for key in ("source_before", "source_after", "runner_before", "runner_after"):
            report = self.receipt()
            report[key] = None
            with self.subTest(key=key), self.assertRaises(c.InvalidEvidence):
                self.assess(report)

    def test_changed_source_client_and_suite_inventory_are_stale(self):
        for key in ("source_before", "source_after", "runner_before", "runner_after",
                    "integration_suites"):
            report = self.receipt()
            report[key] = ["different"] if key == "integration_suites" else {"changed": True}
            result = self.assess(report)
            self.assertEqual(result["suites"][0]["status"], "stale")
            self.assertEqual(result["listed_test_count"], 0)
            self.assertFalse(result["all_selected_suites_listed"])
            self.assert_inventory_only(result)

    def test_failures_timeout_non_utf8_and_execution_are_not_listed(self):
        variants = [(self.text.encode(), 101, False, "failed"),
                    (self.text.encode(), None, True, "timeout"),
                    (self.text.encode(), None, False, "infrastructure"),
                    (b"\xff\n" + self.text.encode(), 0, False, "unverified"),
                    (b"test result: ok. 2 passed;\n", 0, False, "unverified"),
                    (b"truncated", 0, False, "unverified")]
        for payload, code, timeout, status in variants:
            with self.subTest(status=status, payload=payload):
                result = self.assess(self.receipt(payload, code, timeout))
                self.assertEqual(result["suites"][0]["status"], status)
                self.assertEqual(result["listed_test_count"], 0)
                self.assert_inventory_only(result)

    def test_malformed_boolean_nonfinite_and_negative_process_values(self):
        for key, value in (("returncode", True), ("timed_out", 0),
                           ("tests_passed", False), ("seconds", True),
                           ("seconds", float("nan")), ("seconds", float("inf")),
                           ("seconds", -1)):
            report = self.receipt()
            report["results"][0][key] = value
            with self.subTest(key=key, value=value), self.assertRaises(c.InvalidEvidence):
                self.assess(report)

    def test_tamper_unsafe_missing_symlink_and_changed_command_rejected(self):
        for key, value in (("log", "../catalog-a.log"), ("log", "/catalog-a.log"),
                           ("log_sha256", "forged"), ("outcome", "passed"),
                           ("tests_passed", 2), ("kind", "case"),
                           ("command", ["justrust", "test", "--list"])):
            report = self.receipt()
            report["results"][0][key] = value
            with self.subTest(key=key), self.assertRaises(c.InvalidEvidence):
                self.assess(report)
        report = self.receipt()
        path = self.root / "catalog-a.log"
        path.write_text("tampered")
        with self.assertRaises(c.InvalidEvidence):
            self.assess(report)
        report = self.receipt()
        path.unlink()
        with self.assertRaises(c.InvalidEvidence):
            self.assess(report)
        target = self.root / "original.log"
        target.write_text(self.text)
        path.symlink_to(target)
        with self.assertRaises(c.InvalidEvidence):
            self.assess(report)

    def test_selection_and_invocation_are_explicit_unique_and_inventory_only(self):
        for suites in ([], ["a", "a"], ["missing"], "a", [True]):
            with self.subTest(suites=suites), self.assertRaises(c.InvalidEvidence):
                self.catalog.selection(suites, c, self.root)
        self.assertEqual(self.catalog.selection(["b", "a"], c, self.root), ["a", "b"])
        self.assertEqual(self.catalog.entry("a", "target/catalog")["command"], [
            "justrust", "test", "--no-fail-fast", "--target-dir", "target/catalog",
            "--target", "x86_64-unknown-linux-gnu", "--test", "a", "--", "--list", "--include-ignored"])

    def test_catalog_invalid_cli_combinations_before_io(self):
        invalid = [["catalog"], ["catalog", "--out", "absent"],
                   ["assess-catalog", "--out", "absent", "--dry-run"]]
        base = ["catalog", "--out", "absent", "--suite", "a"]
        invalid += [base + flags for flags in (["--suite", "a"], ["--timeout", "0"],
            ["--area", "x"], ["--case", "a::x"], ["--gate", "x"],
            ["--evidence", "x"], ["--jobs", "2"], ["--profile", "full"],
            ["--shard", "0/2"], ["--require-complete"])]
        invalid += [[action, "--suite", "a"] for action in (
            "validate", "plan", "discover", "run", "assess", "track", "coverage", "assess-catalog")]
        for argv in invalid:
            with self.subTest(argv=argv), patch.object(c, "load_json") as load, \
                    contextlib.redirect_stderr(io.StringIO()):
                self.assertEqual(c.main(argv), 2)
                load.assert_not_called()

    def test_valid_catalog_dispatch_preserves_return_codes(self):
        for action in ("catalog", "assess-catalog"):
            for code in (0, 1):
                argv = [action, "--out", "receipt"]
                if action == "catalog":
                    argv += ["--suite", "a"]
                with self.subTest(action=action, code=code), \
                        patch.object(c, "load_json", return_value={}), \
                        patch.object(c, "validate", return_value={}), \
                        patch.object(self.catalog, "main", return_value=code) as dispatch:
                    self.assertEqual(c.main(argv), code)
                    self.assertIs(dispatch.call_args.args[2], c)

    def test_process_row_schema_and_duplicate_results_rejected(self):
        for key in self.receipt()["results"][0]:
            report = self.receipt()
            del report["results"][0][key]
            with self.subTest(missing=key), self.assertRaises(c.InvalidEvidence):
                self.assess(report)
        report = self.receipt()
        report["results"][0]["support_state"] = "passed"
        with self.assertRaises(c.InvalidEvidence):
            self.assess(report)
        for rows in ([], [None], [self.receipt()["results"][0]] * 2):
            report = self.receipt()
            report["results"] = rows
            with self.subTest(rows=rows), self.assertRaises(c.InvalidEvidence):
                self.assess(report)

    def test_all_mapped_names_listed_still_yield_zero_execution_credit(self):
        payload = b"absent: test\nlexical: test\n2 tests, 0 benchmarks\n"
        result = self.assess(self.receipt(payload))
        self.assertTrue(result["all_selected_mapped_tests_listed"])
        self.assert_inventory_only(result)

    def test_main_collects_finalized_before_after_source_and_runner(self):
        import argparse
        import compatibility_scope
        args = argparse.Namespace(action="catalog", out=self.root / "receipt",
            scope=self.root / "scope.json", suite=["a"], timeout=5,
            target_dir="target/catalog", dry_run=False)
        row = self.receipt()["results"][0]
        def execute(invocation, directory, timeout, root):
            self.assertEqual(invocation, self.catalog.entry("a", "target/catalog"))
            self.assertEqual(root, self.root)
            (directory / row["log"]).write_text(self.text)
            return row
        with patch.object(c, "ROOT", self.root), patch.object(c, "load_json", return_value=self.scope), \
                patch.object(compatibility_scope, "validate", return_value=self.scope), \
                patch.object(self.catalog, "runner_identity", return_value=self.runner) as runner, \
                patch.object(c, "source_identity", return_value=self.identity) as identity, \
                patch.object(c, "execute", side_effect=execute) as process, \
                contextlib.redirect_stdout(io.StringIO()):
            self.assertEqual(self.catalog.main(args, {}, c), 1)
        self.assertEqual(runner.call_count, 3)
        self.assertEqual(identity.call_count, 3)
        process.assert_called_once()
        report = c.load_json(args.out / "catalog.json")
        self.assertEqual(report["source_before"], self.identity)
        self.assertEqual(report["source_after"], self.identity)
        self.assertEqual(report["runner_before"], self.runner)
        self.assertEqual(report["runner_after"], self.runner)
        self.assertEqual(report["results"][0]["outcome"], "unverified")

    def coverage_receipt(self, name, suite="a", payload=None, code=0, stale=False):
        """Persist original receipts and logs through the existing fixture API."""
        report = self.receipt(payload, code=code)
        directory = self.root / name
        directory.mkdir()
        row = report["results"][0]
        original_log = self.root / row["log"]
        row.update(self.catalog.entry(suite, report["target_dir"]))
        row["log"] = row["id"] + ".log"
        (directory / row["log"]).write_bytes(original_log.read_bytes())
        report["selected_suites"] = [suite]
        if stale:
            report["source_before"] = {"revision": "old"}
        (directory / "catalog.json").write_bytes(c.encoded(report))
        return directory

    def catalog_coverage(self, directories):
        return self.catalog.coverage(directories, c, self.root, self.scope,
                                     self.identity, self.runner)

    def test_catalog_coverage_partial_and_canonical_duplicate_directories(self):
        directory = self.coverage_receipt("partial")
        alias = self.root / "alias"
        alias.symlink_to(directory, target_is_directory=True)
        result = self.catalog_coverage([directory, directory / ".", alias])
        self.assert_inventory_only(result)
        self.assertFalse(result["complete_current_inventory"])
        self.assertFalse(result["all_mapped_tests_listed"])
        self.assertEqual(result["listed_test_count"], 2)
        rows = {row["suite"]: row for row in result["suites"]}
        self.assertEqual(set(rows), {"a", "b"})
        self.assertEqual(rows["a"]["status"], "listed")
        self.assertEqual(len(rows["a"]["attempts"]), 1)
        self.assertEqual(rows["b"]["status"], "missing")
        self.assertEqual(rows["b"]["tests"], [])

    def test_catalog_coverage_identical_listings_coalesce_without_execution_credit(self):
        payload = b"absent: test\nlexical: test\n2 tests, 0 benchmarks\n"
        first = self.coverage_receipt("first", payload=payload)
        second = self.coverage_receipt("second", payload=payload)
        # The same name in a different suite is a distinct inventory entry.
        other = self.coverage_receipt("other", suite="b",
                                      payload=b"lexical: test\n1 test, 0 benchmarks\n")
        result = self.catalog_coverage([first, second, other])
        self.assert_inventory_only(result)
        self.assertTrue(result["complete_current_inventory"])
        self.assertTrue(result["all_mapped_tests_listed"])
        self.assertEqual(result["listed_test_count"], 3)
        rows = {row["suite"]: row for row in result["suites"]}
        self.assertEqual(len(rows["a"]["attempts"]), 2)
        self.assertEqual(rows["a"]["tests"], ["absent", "lexical"])

    def test_catalog_coverage_complete_inventory_can_lack_mapped_names(self):
        first = self.coverage_receipt("first")
        other = self.coverage_receipt("other", suite="b", payload=b"0 tests, 0 benchmarks\n")
        result = self.catalog_coverage([first, other])
        self.assertTrue(result["complete_current_inventory"])
        self.assertFalse(result["all_mapped_tests_listed"])
        rows = {row["suite"]: row for row in result["suites"]}
        self.assertEqual(rows["a"]["mapped_not_listed"], ["absent"])
        self.assert_inventory_only(result)

    def test_catalog_coverage_stale_names_never_establish_inventory(self):
        stale = self.coverage_receipt("stale", stale=True)
        result = self.catalog_coverage([stale])
        row = next(row for row in result["suites"] if row["suite"] == "a")
        self.assertEqual(row["status"], "stale")
        self.assertEqual(row["tests"], [])
        self.assertEqual(row["benchmarks"], [])
        self.assertEqual(result["listed_test_count"], 0)
        self.assertFalse(result["complete_current_inventory"])

    def test_catalog_coverage_successful_retry_retains_unsuccessful_attempts(self):
        failed = self.coverage_receipt("failed", code=101)
        stale = self.coverage_receipt("stale", stale=True)
        success = self.coverage_receipt("success")
        for directories in ([failed, stale, success], [success, stale, failed]):
            with self.subTest(directories=directories):
                result = self.catalog_coverage(directories)
                row = next(row for row in result["suites"] if row["suite"] == "a")
                self.assertEqual(row["status"], "listed")
                self.assertEqual(row["tests"], ["lexical", "macro_generated::nested::case"])
                self.assertEqual(len(row["attempts"]), 3)
                self.assertEqual(result["listed_test_count"], 2)

    def test_catalog_coverage_conflicting_tests_or_benchmarks_credit_no_names(self):
        first = self.coverage_receipt("first")
        for name, payload in (
                ("tests-conflict", b"different: test\n1 test, 0 benchmarks\n"),
                ("benches-conflict", self.text.replace(
                    "2 tests, 0 benchmarks", "bench: benchmark\n2 tests, 1 benchmark").encode())):
            other = self.coverage_receipt(name, payload=payload)
            with self.subTest(name=name):
                result = self.catalog_coverage([first, other])
                row = next(row for row in result["suites"] if row["suite"] == "a")
                self.assertEqual(row["status"], "conflicting")
                self.assertEqual(row["tests"], [])
                self.assertEqual(row["benchmarks"], [])
                self.assertEqual(result["listed_test_count"], 0)
                self.assertFalse(result["complete_current_inventory"])
                self.assertFalse(result["all_mapped_tests_listed"])

    def test_catalog_coverage_authenticates_every_original_receipt_and_log(self):
        good = self.coverage_receipt("good")
        tampered = self.coverage_receipt("tampered")
        (tampered / "catalog-a.log").write_text("forged: test\n1 test, 0 benchmarks\n")
        with self.assertRaises(c.InvalidEvidence):
            self.catalog_coverage([good, tampered])
        bad_receipt = self.coverage_receipt("bad-receipt")
        report = c.load_json(bad_receipt / "catalog.json")
        report["results"][0]["command"].append("--ignored")
        (bad_receipt / "catalog.json").write_bytes(c.encoded(report))
        with self.assertRaises(c.InvalidEvidence):
            self.catalog_coverage([good, bad_receipt])

    def test_catalog_coverage_rejects_missing_and_redirected_receipts(self):
        for value in ([], None, "not-a-list", [True], [self.root / "missing"]):
            with self.subTest(value=value), self.assertRaises(c.InvalidEvidence):
                self.catalog_coverage(value)
        directory = self.coverage_receipt("redirected")
        receipt = directory / "catalog.json"
        original = self.root / "outside.json"
        receipt.rename(original)
        receipt.symlink_to(original)
        with self.assertRaises(c.InvalidEvidence):
            self.catalog_coverage([directory])

    def test_catalog_coverage_preserves_multiple_unresolved_attempt_categories(self):
        failed = self.coverage_receipt("failed", code=101)
        stale = self.coverage_receipt("stale", stale=True)
        row = self.catalog_coverage([failed, stale])["suites"][0]
        self.assertEqual(row["status"], "unresolved")
        self.assertEqual({attempt["status"] for attempt in row["attempts"]},
                         {"failed", "stale"})
        self.assertEqual(row["tests"], [])

    def test_catalog_coverage_invalid_cli_combinations_before_io(self):
        base = ["catalog-coverage", "--evidence", "absent"]
        invalid = [["catalog-coverage"]]
        invalid += [base + flags for flags in (
            ["--out", "output"], ["--suite", "a"], ["--dry-run"],
            ["--area", "x"], ["--case", "a::x"], ["--gate", "x"],
            ["--shard", "0/2"], ["--require-complete"],
            ["--jobs", "2"], ["--profile", "full"])]
        for argv in invalid:
            with self.subTest(argv=argv), patch.object(c, "load_json") as load, \
                    contextlib.redirect_stderr(io.StringIO()):
                self.assertEqual(c.main(argv), 2)
                load.assert_not_called()

    def test_catalog_coverage_cli_returns_zero_only_for_complete_mapped_inventory(self):
        import json
        import compatibility_scope
        manifest = self.root / "manifest.json"
        manifest.write_bytes(c.encoded({}))
        scope_path = self.root / "scope.json"
        scope_path.write_bytes(c.encoded(self.scope))
        partial = self.coverage_receipt("partial")
        complete = self.coverage_receipt("complete",
            payload=b"absent: test\nlexical: test\n2 tests, 0 benchmarks\n")
        other = self.coverage_receipt("other", suite="b", payload=b"0 tests, 0 benchmarks\n")
        for directories, expected in (([partial], 1), ([partial, other], 1),
                                      ([complete, other], 0)):
            argv = ["catalog-coverage", "--manifest", str(manifest),
                    "--scope", str(scope_path), "--json"]
            for directory in directories:
                argv += ["--evidence", str(directory)]
            output = io.StringIO()
            with self.subTest(directories=directories), \
                    patch.object(c, "ROOT", self.root), \
                    patch.object(c, "validate", return_value={}), \
                    patch.object(compatibility_scope, "validate", return_value=self.scope), \
                    patch.object(c, "source_identity", return_value=self.identity), \
                    patch.object(self.catalog, "runner_identity", return_value=self.runner), \
                    patch.object(c, "execute") as execute, \
                    contextlib.redirect_stdout(output):
                self.assertEqual(c.main(argv), expected)
                execute.assert_not_called()
                self.assert_inventory_only(json.loads(output.getvalue()))

    def test_dry_run_needs_no_client_source_identity_execution_or_output_directory(self):
        import argparse
        import json
        import compatibility_scope
        args = argparse.Namespace(action="catalog", out=self.root / "not-created",
            scope=self.root / "scope.json", suite=["a"], timeout=5,
            target_dir="target/catalog", dry_run=True)
        output = io.StringIO()
        with patch.object(c, "ROOT", self.root), patch.object(c, "load_json", return_value=self.scope), \
                patch.object(compatibility_scope, "validate", return_value=self.scope), \
                patch.object(self.catalog, "runner_identity") as runner, \
                patch.object(c, "source_identity") as identity, patch.object(c, "execute") as execute, \
                contextlib.redirect_stdout(output):
            self.assertEqual(self.catalog.main(args, {}, c), 0)
            runner.assert_not_called()
            identity.assert_not_called()
            execute.assert_not_called()
        self.assertFalse(args.out.exists())
        result = json.loads(output.getvalue())
        self.assertEqual(result["executed_compatibility_case_count"], 0)
        self.assertEqual(result["invocations"], [self.catalog.entry("a", "target/catalog")])


if __name__ == "__main__":
    unittest.main()
