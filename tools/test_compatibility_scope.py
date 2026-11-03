#!/usr/bin/env python3
"""Tests of scope coverage accounting, not claims of Rust conformance."""
import contextlib
import copy
import io
import json
from pathlib import Path
import tempfile
import unittest
from types import SimpleNamespace
from unittest.mock import patch

import compatibility_scope as s
import conformance as c


class ScopeTests(unittest.TestCase):
    def assert_campaign(self, result, statuses):
        ids = [row["id"] for row in self.manifest()["gates"]]
        self.assertEqual(set(ids), set(statuses))
        campaign = result["campaign"]
        self.assertEqual(set(campaign), {"expected_case_count", "measured_case_count",
            "complete_current_measurement", "all_cases_passed", "unresolved_cases",
            "failed_case_ids", "scope_note"})
        measured = sum(statuses[key] in {"passed", "failed"} for key in ids)
        self.assertEqual(campaign["expected_case_count"], len(ids))
        self.assertEqual(campaign["measured_case_count"], measured)
        self.assertIs(campaign["complete_current_measurement"], bool(ids) and measured == len(ids))
        self.assertIs(campaign["all_cases_passed"], bool(ids) and all(statuses[key] == "passed" for key in ids))
        self.assertEqual(campaign["unresolved_cases"], [{"id": key, "status": statuses[key]}
            for key in ids if statuses[key] not in {"passed", "failed"}])
        self.assertEqual(campaign["failed_case_ids"], [key for key in ids if statuses[key] == "failed"])
        self.assertIsInstance(campaign["scope_note"], str)
        self.assertIn("mapped", campaign["scope_note"].lower())
        self.assertIn("rust", campaign["scope_note"].lower())
        self.assertFalse(result["full_rust_support_established"])
        self.assertIsNone(result["full_rust_completion_percent"])

    def coverage(self, reports=(), strict=False, json_output=True):
        path = self.root / "scope.json"
        path.write_bytes(c.encoded(self.scope))
        args = SimpleNamespace(scope=path, action="coverage", evidence=list(reports),
                               json=json_output, require_complete=strict)
        output = io.StringIO()
        with patch.object(c, "ROOT", self.root), \
                patch.object(c, "source_identity", return_value=self.identity), \
                patch.object(s, "runner_identity", return_value=self.runner), \
                contextlib.redirect_stdout(output):
            code = s.main(args, self.base, c)
        return code, json.loads(output.getvalue()) if json_output else output.getvalue()

    def test_campaign_successful_union_missing_layers_is_not_full_rust(self):
        first, _ = self.report("first")
        second, _ = self.report("second", {"rejects": (0, "justrust: ok: 1 test passed\n")})
        for reports in ([first, second], [second, first], [first, second, first]):
            result = self.summary(reports)
            self.assert_campaign(result, {s.case_id(self.accept): "passed", s.case_id(self.reject): "passed"})
            self.assertEqual(result["gate_counts"]["missing"], 2)
            code, printed = self.coverage(reports, strict=True)
            self.assertEqual(code, 0)
            self.assertEqual(printed, result)

    def test_campaign_complete_failed_has_measurement_but_strict_exit_one(self):
        directory, _ = self.report(states={"accepts": (101, "failure"), "rejects": (101, "failure")})
        result = self.summary([directory])
        self.assert_campaign(result, {s.case_id(self.accept): "failed", s.case_id(self.reject): "failed"})
        for strict, expected in ((False, 0), (True, 1)):
            code, printed = self.coverage([directory], strict=strict)
            self.assertEqual(code, expected)
            self.assertEqual(printed, result)
        code, text = self.coverage([directory], strict=True, json_output=False)
        self.assertEqual(code, 1)
        self.assertEqual(text, s.render(result) + "\n")

    def test_campaign_partial_no_evidence_and_empty_manifest(self):
        directory, _ = self.report()
        for reports, status in (([], "untested"), ([directory], "passed")):
            result = self.summary(reports)
            self.assert_campaign(result, {s.case_id(self.accept): status, s.case_id(self.reject): "untested"})
            self.assertEqual(self.coverage(reports)[0], 0)
        code, printed = self.coverage([directory], strict=True)
        self.assertEqual(code, 1)
        self.assertEqual(printed, self.summary([directory]))
        for gate in self.scope["areas"][0]["requirements"][0]["gates"]:
            gate["tests"] = []
        self.assert_campaign(self.summary(), {})

    def test_campaign_noncurrent_or_unverified_cases_remain_unresolved(self):
        for status in ("stale", "conflicting", "timeout", "infrastructure", "unverified"):
            with self.subTest(status=status):
                directory, report = self.report(status, {test: (0, "justrust: ok: 1 test passed\n")
                                                        for test in ("accepts", "rejects")})
                reports = [directory]
                if status == "stale":
                    report["source_before"] = {"revision": "old"}
                elif status == "conflicting":
                    failed, _ = self.report("conflict-failure", {"accepts": (101, "failure")})
                    reports.append(failed)
                else:
                    row = next(row for row in report["results"] if row["id"] == s.case_id(self.accept))
                    payload = b"unverified log\n"
                    (directory / row["log"]).write_bytes(payload)
                    row.update(returncode=101 if status == "timeout" else None if status == "infrastructure" else 0,
                               timed_out=status == "timeout", outcome=status, tests_passed=0,
                               log_sha256=c.digest(payload))
                self.save(directory, report)
                self.assert_campaign(self.summary(reports), {s.case_id(self.accept): status,
                    s.case_id(self.reject): "stale" if status == "stale" else "passed"})
                code, printed = self.coverage(reports, strict=True)
                self.assertEqual(code, 1)
                self.assertEqual(printed, self.summary(reports))

    def test_campaign_interaction_only_and_duplicate_mappings_are_unique(self):
        (self.root / "tests/sample.rs").write_text(
            "#[test]\nfn accepts() {}\n#[test]\nfn rejects() {}\n#[test]\nfn combined() {}\n")
        self.matrix()
        combined = {"suite": "sample", "test": "combined"}
        self.scope["interaction_matrix"]["scenarios"][0]["tests"] = [combined]
        original = self.scope["areas"][0]["requirements"][0]
        original["gates"][2]["tests"] = [self.accept]
        another = copy.deepcopy(original)
        another["id"] = "duplicate-campaign-mapping"
        self.scope["areas"][0]["requirements"].append(another)
        directory, _ = self.report(states={test: (0, "justrust: ok: 1 test passed\n")
                                           for test in ("accepts", "rejects", "combined")})
        self.assert_campaign(self.summary([directory, directory]), {
            s.case_id(case): "passed" for case in (self.accept, self.reject, combined)})
        self.assertEqual(self.coverage([directory], strict=True)[0], 0)

    def test_campaign_malformed_or_interrupted_reports_never_complete(self):
        changes = {
            "interrupted": lambda report: report["results"].pop(),
            "nonobject": lambda report: None,
            **{f"missing-{field}": lambda report, field=field: report.pop(field)
               for field in ("source_before", "source_after", "runner_before", "runner_after")},
            **{f"null-{field}": lambda report, field=field: report.update({field: None})
               for field in ("source_before", "source_after", "runner_before", "runner_after")},
            "duplicate": lambda report: report["results"].append(report["results"][0]),
            "forged": lambda report: report["results"][0].update(tests_passed=999),
        }
        for name, change in changes.items():
            with self.subTest(name=name):
                directory, report = self.report(name, {test: (0, "justrust: ok: 1 test passed\n")
                                                       for test in ("accepts", "rejects")})
                change(report)
                self.save(directory, [] if name == "nonobject" else report)
                with self.assertRaises(c.InvalidEvidence):
                    self.coverage([directory], strict=True)
        directory, _ = self.report("broken-json")
        (directory / "report.json").write_text('{"results": [')
        with self.assertRaises(c.InvalidEvidence):
            self.coverage([directory], strict=True)

    def test_timeout_infrastructure_and_unverified_logs_never_annotate_failures(self):
        marker = "[justrust-conformance-failure:v1] invalid-source-accepted\n"
        for status, code, timed_out in (("timeout", 101, True),
                                         ("infrastructure", None, False),
                                         ("unverified", 0, False)):
            directory, report = self.report(status, {"accepts": (code, marker)})
            report["results"][0].update(timed_out=timed_out, outcome=status)
            self.save(directory, report)
            result = self.summary([directory])
            self.assertEqual(result["exact_case_counts"][status], 1)
            self.assertEqual(sum(result["failed_case_annotations"].values()), 0)
            case = result["areas"][0]["requirements"][0]["gates"][0]["tests"][0]
            self.assertNotIn("failure_annotation", case)
            rendered = s.render(result)
            self.assertIn(status + ": 1", rendered)
            self.assertLess(rendered.index("Failure annotations"), rendered.index("AREA /"))

    def test_annotations_are_rederived_not_trusted_report_fields(self):
        marker = "[justrust-conformance-failure:v1] valid-source-rejected\n"
        for name, text, expected in (("marked", marker, "valid-source-rejected"),
                                     ("legacy", "legacy failure", "unannotated")):
            directory, report = self.report(name, {"accepts": (101, text)})
            report["failed_case_annotations"] = {"invalid-source-accepted": 999}
            report["results"][0]["failure_annotation"] = "invalid-source-accepted"
            self.save(directory, report)
            result = self.summary([directory])
            self.assertEqual(result["failed_case_annotations"][expected], 1)
            self.assertEqual(sum(result["failed_case_annotations"].values()), 1)
            observed = result["areas"][0]["requirements"][0]["gates"][0]["tests"][0]
            self.assertEqual(observed["failure_annotation"], expected)
            self.assertEqual(observed["evidence"][0]["failure_annotation"], expected)
            self.assertEqual(observed["status"], "failed")
        log = directory / report["results"][0]["log"]
        log.write_text(marker)
        with self.assertRaises(c.InvalidEvidence):
            self.summary([directory])

    def test_success_stale_and_conflicting_markers_are_not_current_annotations(self):
        marker = "[justrust-conformance-failure:v1] semantic-stage-unproven\n"
        passed, _ = self.report("passed-marked", {"accepts": (0, marker + "justrust: ok: 1 test passed\n")})
        stale, report = self.report("stale-marked", {"accepts": (101, marker)})
        report["source_before"] = {"revision": "old"}
        self.save(stale, report)
        failed, _ = self.report("failed-marked", {"accepts": (101, marker)})
        for reports, status in (([passed], "passed"), ([stale], "stale"),
                                ([passed, failed, stale], "conflicting"),
                                ([failed, passed], "conflicting"), ([passed, stale], "passed")):
            result = self.summary(reports)
            self.assertEqual(result["exact_case_counts"][status], 1)
            self.assertEqual(sum(result["failed_case_annotations"].values()), 0)
            case = result["areas"][0]["requirements"][0]["gates"][0]["tests"][0]
            self.assertNotIn("failure_annotation", case)
            for attempt in case["evidence"]:
                if attempt["status"] != "failed":
                    self.assertNotIn("failure_annotation", attempt)
        legacy, _ = self.report("current-legacy", {"accepts": (101, "legacy")})
        result = self.summary([legacy, stale])
        self.assertEqual(result["failed_case_annotations"]["unannotated"], 1)
        self.assertEqual(result["failed_case_annotations"]["semantic-stage-unproven"], 0)

    def test_annotation_disagreement_and_legacy_attempts_are_unannotated(self):
        prefix = "[justrust-conformance-failure:v1] "
        marked, _ = self.report("marked", {"accepts": (101, prefix + "valid-source-rejected\n")})
        other, _ = self.report("other", {"accepts": (101, prefix + "invalid-source-accepted\n")})
        legacy, _ = self.report("legacy", {"accepts": (101, "legacy")})
        for reports in ([marked, other], [other, marked], [marked, legacy]):
            result = self.summary(reports)
            self.assertEqual(result["exact_case_counts"]["failed"], 1)
            self.assertEqual(result["failed_case_annotations"]["unannotated"], 1)
            self.assertEqual(sum(result["failed_case_annotations"].values()), 1)
        self.assertEqual(self.summary([marked, marked])["failed_case_annotations"]["valid-source-rejected"], 1)

    def test_unique_counts_include_interaction_only_and_deduplicate_gate_mappings(self):
        self.matrix()
        combined = {"suite": "sample", "test": "combined"}
        self.scope["interaction_matrix"]["scenarios"][0]["tests"] = [combined]
        original = self.scope["areas"][0]["requirements"][0]
        original["gates"][2]["tests"] = [self.accept]
        another = copy.deepcopy(original)
        another["id"] = "duplicate-mapping"
        self.scope["areas"][0]["requirements"].append(another)
        directory, _ = self.report(states={
            "accepts": (101, "[justrust-conformance-failure:v1] valid-source-rejected\n"),
            "combined": (101, "[justrust-conformance-failure:v1] rejection-proof-unproven\n")})
        result = self.summary([directory])
        self.assertEqual(sum(result["exact_case_counts"].values()), 3)
        self.assertEqual(result["exact_case_counts"]["failed"], 2)
        self.assertEqual(result["exact_case_counts"]["untested"], 1)
        self.assertEqual(result["gate_counts"]["failed"], 4)
        self.assertEqual(result["failed_case_annotations"]["valid-source-rejected"], 1)
        self.assertEqual(result["failed_case_annotations"]["rejection-proof-unproven"], 1)
        rendered = s.render(result)
        self.assertIn("Unique exact tests: 3; passed: 0; failed: 2", rendered)
        self.assertIn("diagnostic only, not proof or outcome changes", rendered)

    def test_missing_source_inventory_is_explicit_not_complete(self):
        summary = self.summary()
        self.assertEqual(summary["semantic_inventory"]["readiness"], "missing")
        self.assertIsNone(summary["full_rust_completion_percent"])
        self.assertFalse(summary["full_rust_support_established"])

    def test_malformed_source_inventory_fails_closed(self):
        with patch.object(s.compatibility_inventory, "load_and_summarize",
                          side_effect=s.compatibility_inventory.InvalidInventory("changed oracle")):
            with self.assertRaisesRegex(c.InvalidEvidence, "changed oracle"):
                self.summary()

    def setUp(self):
        self.temp = tempfile.TemporaryDirectory(prefix="compatibility-scope-")
        self.addCleanup(self.temp.cleanup)
        self.root = Path(self.temp.name)
        (self.root / "tests").mkdir()
        (self.root / "tests/sample.rs").write_text(
            "#[test]\nfn accepts() {}\n#[test]\nfn rejects() {}\n")
        self.accept = {"suite": "sample", "test": "accepts"}
        self.reject = {"suite": "sample", "test": "rejects"}
        self.scope = {"schema_version": 1, "scope": {
            "edition": "2024", "target": "x86_64-unknown-linux-gnu", "semantic_inventory": "incomplete"},
            "areas": [{"id": "memory", "title": "Memory", "requirements": [
                {"id": "moves", "title": "Move scenarios", "gates": [
                    {"layer": layer, "tests": [self.accept] if layer == "feature" else
                     [self.reject] if layer == "rejection" else []} for layer in s.LAYERS]}]}]}
        self.base = {"contract": {}}
        self.identity = {"revision": "abc", "inputs_sha256": "def", "input_files": 3}
        self.runner = {"executable": "/stable/justrust", "sha256": "client", "environment": {}}

    def manifest(self):
        return s.case_manifest(self.scope, self.base, c)

    def track(self, shard=(), area=(), cases=(), dry_run=True, out=None):
        path = self.root / "scope.json"
        path.write_bytes(c.encoded(self.scope))
        args = SimpleNamespace(scope=path, action="track", area=list(area),
                               case=list(cases), shard=list(shard), jobs=1,
                               target_dir="target/test", dry_run=dry_run,
                               timeout=10, out=out)
        output = io.StringIO()
        with patch.object(c, "ROOT", self.root), contextlib.redirect_stdout(output):
            result = s.main(args, self.base, c)
        return result, json.loads(output.getvalue()) if dry_run else output.getvalue()

    def test_shards_are_sorted_disjoint_exhaustive_and_uneven(self):
        selected = {"case:" + str(index) for index in range(7)}
        shards = [s.shard_selection(selected, [f"{index}/3"], c) for index in range(3)]
        self.assertEqual(shards, [sorted(selected)[index::3] for index in range(3)])
        self.assertEqual([len(part) for part in shards], [3, 2, 2])
        self.assertEqual(set().union(*map(set, shards)), selected)
        self.assertEqual(sum(map(len, shards)), len(selected))
        self.assertEqual(s.shard_selection(selected, [], c), sorted(selected))
        self.assertEqual(s.shard_selection(selected, ["0/1"], c), sorted(selected))
        self.assertEqual(s.shard_selection(selected, ["0/100000000000000000000"], c),
                         [sorted(selected)[0]])

    def test_invalid_shards_rejected_before_output_creation_or_execution(self):
        invalid = ["", "0", "0/", "/1", "0/0", "-1/2", "0/-2", "2/2", "3/2",
                   "1.0/2", " 0/2", "0/2 ", "+0/2", "0/2/3", "\u0660/2", "998/999"]
        for shard in ([value] for value in invalid):
            out = self.root / "must-not-exist"
            with self.subTest(shard=shard), patch.object(c, "execute") as execute:
                with self.assertRaises(c.InvalidEvidence):
                    self.track(shard=shard, dry_run=False, out=out)
                self.assertFalse(out.exists())
                execute.assert_not_called()
        for shard in (["0/2", "0/2"], ["0/2", "1/2"]):
            with self.subTest(shard=shard), self.assertRaises(c.InvalidEvidence):
                self.track(shard=shard)
        with self.assertRaises(c.InvalidEvidence):
            s.shard_selection([], [], c)

    def test_sharding_follows_area_case_and_interaction_only_selection(self):
        (self.root / "tests/sample.rs").write_text(
            "#[test]\nfn accepts() {}\n#[test]\nfn rejects() {}\n#[test]\nfn combined() {}\n")
        self.matrix()
        combined = {"suite": "sample", "test": "combined"}
        self.scope["interaction_matrix"]["scenarios"][0]["tests"] = [combined]
        _, original = self.track()
        _, identity = self.track(shard=["0/1"])
        self.assertEqual(original, identity)
        self.assertEqual(len(original["gates"]), 3)
        _, interaction = self.track(shard=["0/1"], area=["interactions"],
                                    cases=["sample::combined"])
        self.assertEqual([row["id"] for row in interaction["gates"]], [s.case_id(combined)])
        _, filtered = self.track(shard=["1/2"], area=["memory"],
                                 cases=["sample::rejects", "sample::accepts"])
        self.assertEqual([row["id"] for row in filtered["gates"]], [s.case_id(self.reject)])
        with self.assertRaises(c.InvalidEvidence):
            self.track(shard=["0/1"], area=["memory"], cases=["sample::combined"])

    def test_shard_receipts_union_and_execute_each_case_once_without_retries(self):
        calls = []
        def execute(entry, directory, timeout, root):
            calls.append(entry["id"])
            payload = b"justrust: ok: 1 test passed in 1s\n"
            name = entry["id"] + ".log"
            (directory / name).write_bytes(payload)
            return {**entry, "returncode": 0, "timed_out": False, "outcome": "passed",
                    "tests_passed": 1, "log": name, "log_sha256": c.digest(payload)}
        directories = [self.root / f"shard-{index}" for index in range(2)]
        with patch.object(c, "execute", side_effect=execute), \
                patch.object(c, "source_identity", return_value=self.identity), \
                patch.object(s, "runner_identity", return_value=self.runner):
            for index, directory in enumerate(directories):
                result, _ = self.track(shard=[f"{index}/2"], dry_run=False, out=directory)
                self.assertEqual(result, 0)
                report = c.load_json(directory / "report.json")
                self.assertEqual(len(report["selected"]), 1)
                self.assertEqual(report["source_before"], self.identity)
                self.assertEqual(report["runner_before"], self.runner)
        self.assertEqual(sorted(calls), sorted([s.case_id(self.accept), s.case_id(self.reject)]))
        for directory in directories:
            self.assertEqual(len(s.evidence(self.scope, self.manifest(), directory,
                                           self.identity, self.runner, c, self.root)), 1)
        union = self.summary(directories)
        self.assertEqual(union, self.summary(directories[::-1]))
        self.assertEqual(union["gate_counts"]["passed"], 2)
        self.assertEqual(union["gate_counts"]["untested"], 0)

    def test_failed_and_timed_out_shard_cases_are_never_retried(self):
        entries = s.shard_selection({"case:a", "case:b", "case:c", "case:d"}, ["1/2"], c)
        for jobs in (1, 2):
            calls = []
            def execute(entry, directory, timeout, root):
                calls.append(entry)
                return {"id": entry, "outcome": "timeout" if entry == "case:d" else "failed"}
            runner = SimpleNamespace(execute=execute, InvalidEvidence=c.InvalidEvidence)
            results = list(s.execute_cases(entries, self.root, 10, jobs, runner, self.root))
            self.assertEqual(sorted(calls), entries)
            self.assertEqual([row["outcome"] for row in results], ["failed", "timeout"])

    def report(self, directory="run", states=None):
        states = states or {"accepts": (0, "justrust: ok: 1 test passed in 1s\n")}
        directory = self.root / directory
        directory.mkdir()
        manifest = self.manifest()
        selected = sorted(s.case_id({"suite": "sample", "test": test}) for test in states)
        entries = c.plan(manifest, "full", selected, "target/test")
        results = []
        for entry in entries:
            test = entry["command"][entry["command"].index("--") + 1]
            code, text = states[test]
            payload = text.encode()
            name = entry["id"] + ".log"
            (directory / name).write_bytes(payload)
            results.append({**entry, "returncode": code, "timed_out": False,
                            "outcome": c.outcome(code, text, "case"), "tests_passed": c.test_count(text),
                            "log": name, "log_sha256": c.digest(payload)})
        report = {"schema_version": 1, "profile": "full", "selected": selected,
                  "target_dir": "target/test", "manifest_sha256": c.digest(c.encoded(manifest)),
                  "scope_sha256": c.digest(c.encoded(self.scope)), "source_before": self.identity,
                  "source_after": self.identity, "runner_before": self.runner, "runner_after": self.runner,
                  "integration_suites": ["sample"], "results": results}
        (directory / "report.json").write_bytes(c.encoded(report))
        return directory, report

    def summary(self, reports=()):
        return s.summarize(self.scope, self.manifest(), list(reports), self.identity, self.runner, c, self.root)

    def save(self, directory, report):
        (directory / "report.json").write_bytes(c.encoded(report))

    def test_parallel_execution_preserves_order_failures_and_concurrency_bound(self):
        import threading
        import time
        lock = threading.Lock()
        active = 0
        peak = 0
        def execute(entry, directory, timeout, root):
            nonlocal active, peak
            with lock:
                active += 1
                peak = max(peak, active)
            time.sleep(0.02 if entry != 0 else 0.04)
            with lock:
                active -= 1
            return {"id": entry, "exit_code": 1 if entry == 1 else 0}
        runner = SimpleNamespace(execute=execute, InvalidEvidence=c.InvalidEvidence)
        results = list(s.execute_cases(list(range(6)), self.root, 10, 2, runner, self.root))
        self.assertEqual([row["id"] for row in results], list(range(6)))
        self.assertEqual([row["exit_code"] for row in results], [0, 1, 0, 0, 0, 0])
        self.assertEqual(peak, 2)
        self.assertEqual(active, 0)

    def test_invalid_parallel_limits_are_not_silently_clamped(self):
        for jobs in (0, -1, 9, True, 1.5, "2"):
            with self.subTest(jobs=jobs), self.assertRaises(c.InvalidEvidence):
                list(s.execute_cases([], self.root, 10, jobs, c, self.root))

    def test_evidence_notes_are_scoped_to_exact_gate_tests_and_reported(self):
        gate = self.scope["areas"][0]["requirements"][0]["gates"][0]
        gate["evidence_notes"] = [{"tests": [self.accept], "note": "One narrow scenario, not all moves."}]
        s.validate(self.scope, c, self.root)
        self.assertEqual(self.summary()["areas"][0]["requirements"][0]["gates"][0]["evidence_notes"], gate["evidence_notes"])
        for note in ({"tests": [self.reject], "note": "foreign gate"},
                     {"tests": [], "note": "unbound claim"},
                     {"tests": [self.accept], "note": " "}):
            gate["evidence_notes"] = [note]
            with self.assertRaises(c.InvalidEvidence):
                s.validate(self.scope, c, self.root)

    def test_missing_and_unexecuted_obligations_stay_in_denominator(self):
        s.validate(self.scope, c, self.root)
        result = self.summary()
        self.assertEqual(result["declared_gate_count"], 4)
        self.assertEqual(result["gate_counts"]["missing"], 2)
        self.assertEqual(result["gate_counts"]["untested"], 2)
        self.assertEqual(result["passing_declared_gates_percent"], 0)
        self.assertFalse(result["full_rust_support_established"])

    def test_readiness_counts_mapping_without_claiming_implementation(self):
        result = self.summary()
        readiness = result["measurement_readiness"]
        self.assertEqual(readiness["mapped_gate_count"], 2)
        self.assertEqual(readiness["mapped_declared_gates_percent"], 50)
        self.assertEqual(readiness["mapped_exact_test_count"], 2)
        self.assertFalse(readiness["can_estimate_full_rust_completion"])
        self.assertIsNone(result["full_rust_completion_percent"])
        self.assertEqual([row["layer"] for row in result["missing_test_obligations"]],
                         ["interaction", "end_to_end"])
        directory, _ = self.report()
        measured = self.summary([directory])
        self.assertEqual(measured["measurement_readiness"], readiness)
        self.assertEqual(measured["passing_declared_gates_percent"], 25)

    def test_all_declared_cells_green_does_not_certify_a_complete_inventory(self):
        for gate in self.scope["areas"][0]["requirements"][0]["gates"]:
            gate["tests"] = [self.accept]
        directory, _ = self.report()
        result = self.summary([directory])
        self.assertEqual(result["passing_declared_gates_percent"], 100)
        self.assertEqual(result["measurement_readiness"]["mapped_declared_gates_percent"], 100)
        self.assertIsNone(result["full_rust_completion_percent"])
        self.assertFalse(result["measurement_readiness"]["can_estimate_full_rust_completion"])
        self.assertIn("unavailable", s.render(result))

    def test_exact_case_success_is_not_entire_requirement_support(self):
        directory, _ = self.report()
        result = self.summary([directory])
        self.assertEqual(result["gate_counts"]["passed"], 1)
        self.assertEqual(result["passing_declared_gates_percent"], 25)
        self.assertEqual(result["requirements_with_all_declared_gates_passed"], 0)
        self.assertIn("NOT percentage of all Rust", s.render(result))
        command = c.command({"kind": "case", **self.accept}, "target/test")
        self.assertEqual(command[-5:], ["accepts", "--exact", "--include-ignored", "--nocapture", "--test-threads=1"])

    def test_unverified_empty_or_multiple_cases_never_pass(self):
        for text in ("", "justrust: ok: 0 tests passed", "justrust: ok: 2 tests passed",
                     "justrust: ok, but 0 tests matched the filter"):
            self.assertEqual(c.outcome(0, text, "case"), "unverified")
        self.assertEqual(c.outcome(0, "test result: ok. 1 passed;", "case"), "passed")
        self.assertEqual(c.outcome(101, "justrust: ok: 1 test passed", "case"), "failed")

    def test_failed_case_preserves_other_independent_success(self):
        directory, _ = self.report(states={"accepts": (0, "justrust: ok: 1 test passed\n"),
                                           "rejects": (101, "unsupported valid source\n")})
        result = self.summary([directory])
        self.assertEqual(result["gate_counts"]["passed"], 1)
        self.assertEqual(result["gate_counts"]["failed"], 1)

    def test_gate_with_several_cases_requires_every_case(self):
        self.scope["areas"][0]["requirements"][0]["gates"][0]["tests"].append(self.reject)
        directory, _ = self.report()
        self.assertEqual(self.summary([directory])["gate_counts"]["passed"], 0)
        self.assertEqual(self.summary([directory])["gate_counts"]["untested"], 2)

    def test_source_runner_environment_and_during_run_changes_are_stale(self):
        for field in ("source_before", "source_after", "runner_before", "runner_after", "integration_suites"):
            with self.subTest(field=field):
                directory, report = self.report(directory=field)
                report[field] = {"changed": True}
                self.save(directory, report)
                result = self.summary([directory])
                self.assertEqual(result["gate_counts"]["passed"], 0)
                self.assertEqual(result["gate_counts"]["stale"], 1)

    def test_current_failure_cannot_be_laundered_by_passing_rerun(self):
        passed, _ = self.report("passed")
        failed, _ = self.report("failed", {"accepts": (101, "failed")})
        for reports in ([passed, failed], [failed, passed]):
            result = self.summary(reports)
            self.assertEqual(result["gate_counts"]["conflicting"], 1)
            self.assertEqual(result["gate_counts"]["passed"], 0)

    def test_current_success_can_replace_stale_evidence_not_current_failure(self):
        stale, report = self.report("stale")
        report["source_before"] = {"revision": "old"}
        self.save(stale, report)
        fresh, _ = self.report("fresh")
        self.assertEqual(self.summary([stale, fresh])["gate_counts"]["passed"], 1)

    def test_missing_tampered_filtered_and_reordered_evidence_rejected(self):
        changes = {
            "missing": lambda r: r["results"].clear(),
            "duplicate": lambda r: r["results"].append(r["results"][0]),
            "command": lambda r: r["results"][0]["command"].remove("--exact"),
            "outcome": lambda r: r["results"][0].update(outcome="failed"),
            "count": lambda r: r["results"][0].update(tests_passed=999),
            "manifest": lambda r: r.update(manifest_sha256="changed"),
            "scope": lambda r: r.update(scope_sha256="changed"),
            "schema": lambda r: r.update(schema_version=2),
            "profile": lambda r: r.update(profile="smoke"),
            "implicit-selection": lambda r: r.update(selected=[]),
        }
        for name, change in changes.items():
            directory, report = self.report(name)
            change(report)
            self.save(directory, report)
            with self.subTest(change=name), self.assertRaises(c.InvalidEvidence):
                self.summary([directory])
        directory, report = self.report("log")
        (directory / report["results"][0]["log"]).write_text("tampered")
        with self.assertRaises(c.InvalidEvidence):
            self.summary([directory])

    def test_external_symlink_log_is_rejected(self):
        directory, report = self.report()
        log = directory / report["results"][0]["log"]
        outside = self.root / "outside.log"
        outside.write_bytes(log.read_bytes())
        log.unlink()
        log.symlink_to(outside)
        with self.assertRaises(c.InvalidEvidence):
            self.summary([directory])

    def test_scope_validation_rejects_missing_layers_cases_and_duplicate_ids(self):
        variants = []
        changed = copy.deepcopy(self.scope)
        changed["areas"][0]["requirements"][0]["gates"].pop()
        variants.append(changed)
        changed = copy.deepcopy(self.scope)
        changed["areas"][0]["requirements"][0]["gates"][0]["tests"][0]["test"] = "absent"
        variants.append(changed)
        changed = copy.deepcopy(self.scope)
        changed["areas"][0]["requirements"].append(copy.deepcopy(changed["areas"][0]["requirements"][0]))
        variants.append(changed)
        changed = copy.deepcopy(self.scope)
        changed["scope"]["semantic_inventory"] = "complete"
        variants.append(changed)
        changed = copy.deepcopy(self.scope)
        changed["areas"][0]["requirements"][0]["gates"][0]["tests"] = [self.accept, self.accept]
        variants.append(changed)
        for changed in variants:
            with self.assertRaises(c.InvalidEvidence):
                s.validate(changed, c, self.root)

    def test_duplicate_case_maps_once_but_reports_each_explicit_obligation(self):
        original = self.scope["areas"][0]["requirements"][0]
        original["gates"][2]["tests"] = [self.accept]
        another = copy.deepcopy(original)
        another["id"] = "lifetimes"
        self.scope["areas"][0]["requirements"].append(another)
        s.validate(self.scope, c, self.root)
        self.assertEqual(len(self.manifest()["gates"]), 2)
        result = self.summary()
        self.assertEqual(result["declared_gate_count"], 8)
        self.assertEqual(result["declared_interaction_pairs"], [["lifetimes", "moves"]])
        self.assertIn("No pairwise completeness claim", result["interaction_note"])

    def test_scope_changes_do_not_reuse_old_denominator_or_passing_results(self):
        directory, _ = self.report()
        self.scope["areas"][0]["requirements"][0]["title"] = "Different scope"
        with self.assertRaises(c.InvalidEvidence):
            self.summary([directory])

    def test_report_json_is_roundtrippable_and_lists_unmapped_suites(self):
        (self.root / "tests/unmapped.rs").write_text("#[test]\nfn unknown() {}")
        result = self.summary()
        self.assertEqual(json.loads(c.encoded(result)), result)
        self.assertEqual(result["unmapped_integration_suites"], ["unmapped"])

    def test_inventory_without_evidence_does_not_require_or_invent_git_identity(self):
        path = self.root / "scope.json"
        path.write_bytes(c.encoded(self.scope))
        args = SimpleNamespace(scope=path, action="coverage", evidence=[], json=True)
        output = io.StringIO()
        with patch.object(c, "ROOT", self.root), \
                patch.object(c, "source_identity", side_effect=AssertionError("no Git in snapshot")), \
                contextlib.redirect_stdout(output):
            self.assertEqual(s.main(args, self.base, c), 0)
        result = json.loads(output.getvalue())
        self.assertIsNone(result["source"])
        self.assertEqual(result["gate_counts"]["passed"], 0)

    def test_ignored_case_is_not_verified_without_actual_execution(self):
        text = "test result: ok. 0 passed; 0 failed; 1 ignored;"
        self.assertEqual(c.outcome(0, text, "case"), "unverified")
        self.assertIn("--include-ignored", c.command({"kind": "case", **self.accept}, "target/test"))

    def matrix(self):
        self.scope["interaction_matrix"] = {
            "axes": [{"id": name, "title": name} for name in ["borrow", "generic", "drop"]],
            "scenarios": [{"id": "borrow-generic", "features": ["borrow", "generic"],
                           "tests": [self.accept]}]}

    def test_pairwise_denominator_enumerates_untested_and_missing_combinations(self):
        self.matrix()
        s.validate(self.scope, c, self.root)
        result = self.summary()["interaction_matrix"]
        self.assertEqual(result["pair_count"], 3)
        self.assertEqual(result["missing_pairs"], 2)
        self.assertEqual(result["passing_pairs"], 0)
        directory, _ = self.report()
        self.assertEqual(self.summary([directory])["interaction_matrix"]["passing_pairs"], 1)
        self.scope["interaction_matrix"]["axes"].append({"id": "closure", "title": "Closure"})
        self.assertEqual(self.summary()["interaction_matrix"]["pair_count"], 6)
        self.assertEqual(self.summary()["interaction_matrix"]["missing_pairs"], 5)

    def test_pair_requires_all_matching_scenarios_and_actual_cases(self):
        self.matrix()
        self.scope["interaction_matrix"]["scenarios"].append(
            {"id": "different-order", "features": ["borrow", "generic"], "tests": [self.reject]})
        directory, _ = self.report()
        result = self.summary([directory])["interaction_matrix"]
        self.assertEqual(result["passing_pairs"], 0)
        pair = next(row for row in result["pairs"] if row["features"] == ["borrow", "generic"])
        self.assertEqual(pair["status"], "untested")

    def test_higher_order_scenario_retains_all_its_pairs_without_exhaustive_claim(self):
        self.matrix()
        self.scope["interaction_matrix"]["scenarios"][0]["features"].append("drop")
        directory, _ = self.report()
        result = self.summary([directory])["interaction_matrix"]
        self.assertEqual(result["passing_pairs"], 3)
        self.assertEqual(result["scenarios"][0]["features"], ["borrow", "generic", "drop"])

    def test_unknown_duplicate_axes_and_foreign_scenario_features_rejected(self):
        self.matrix()
        original = copy.deepcopy(self.scope)
        for mutation in range(3):
            changed = copy.deepcopy(original)
            matrix = changed["interaction_matrix"]
            if mutation == 0:
                matrix["axes"].append(matrix["axes"][0])
            elif mutation == 1:
                matrix["scenarios"][0]["features"].append("foreign")
            else:
                matrix["scenarios"][0]["features"].append("borrow")
            with self.assertRaises(c.InvalidEvidence):
                s.validate(changed, c, self.root)

    def test_generated_probe_names_are_discovered_but_do_not_become_evidence(self):
        (self.root / "tests/conformance_generated.rs").write_text(
            "probes!(false; valid_00 = 0, valid_01 = 1,);\n")
        gate = self.scope["areas"][0]["requirements"][0]["gates"][2]
        gate["tests"] = [{"suite": "conformance_generated", "test": "valid_00"}]
        s.validate(self.scope, c, self.root)
        self.assertEqual(self.summary()["gate_counts"]["passed"], 0)
        gate["tests"][0]["test"] = "valid_02"
        with self.assertRaises(c.InvalidEvidence):
            s.validate(self.scope, c, self.root)


if __name__ == "__main__":
    unittest.main()
