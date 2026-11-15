"""Tests for the stdout-only lock inventory, without Rust builds."""
import contextlib
import io
import json
from pathlib import Path
import tempfile
import unittest

try:
    from . import lock_report
except ImportError:
    import lock_report


class LockReportTests(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory()
        self.addCleanup(self.temp.cleanup)
        self.root = Path(self.temp.name)

    def put(self, rid, **fields):
        path = self.root / rid / "summary.json"
        path.parent.mkdir()
        record = dict(id=rid, start=100, cwd="/home/me/project")
        record.update(fields)
        path.write_text(json.dumps(record))
        return path

    def test_overlap_and_measured_sync_are_not_added(self):
        self.put("overlap", phases={"lock_wait": 12},
                 remote={"sync": {"lock_ms": 2500}},
                 locks_waited=["build directory"],
                 e2e={"wall": 30, "segments": [
                     {"key": "host.lock_wait", "secs": 9},
                     {"key": "sync.lock_wait", "secs": 2},
                     {"key": "remote.admission", "secs": 3}]})
        result = lock_report.report(self.root)
        metrics = result["metrics"]
        self.assertEqual(metrics["cargo_lock_estimated"]["total_secs"], 12)
        self.assertEqual(metrics["sync_queue_measured"]["total_secs"], 2.5)
        self.assertEqual(metrics["e2e_sync_lock_wait"]["total_secs"], 2)
        self.assertEqual(metrics["e2e_admission"]["total_secs"], 3)
        self.assertNotIn("total_lock_secs", result)
        self.assertEqual(result["sources"]["cargo_e2e_fallback"], 0)

    def test_mixed_labels_remain_unallocated(self):
        self.put("mixed", phases={"lock_wait": 10},
                 locks_waited=["build directory", "package cache"])
        self.put("package", phases={"lock_wait": 4}, locks_waited=["package cache"])
        self.put("unknown", phases={"lock_wait": 3}, locks_waited=["other"])
        self.put("no-label", phases={"lock_wait": 2})
        result = lock_report.report(self.root)
        metrics = result["metrics"]
        self.assertEqual(metrics["cargo_unallocated_estimated"]["total_secs"], 15)
        self.assertEqual(metrics["cargo_package_cache_estimated"]["total_secs"], 4)
        self.assertIsNone(metrics["cargo_build_directory_estimated"]["total_secs"])
        self.assertEqual(result["lock_label_record_counts"]["package cache"], 2)

    def test_missing_is_not_zero_and_empty_segments_are_unknown(self):
        self.put("missing")
        self.put("empty", e2e={"segments": []})
        self.put("zero", phases={"lock_wait": 0}, remote={"sync": {"lock_ms": 0}})
        metrics = lock_report.report(self.root)["metrics"]
        self.assertEqual(metrics["cargo_lock_estimated"]["covered_records"], 1)
        self.assertEqual(metrics["cargo_lock_estimated"]["missing_records"], 2)
        self.assertEqual(metrics["cargo_lock_estimated"]["p95_secs"], 0)
        self.assertIsNone(metrics["e2e_admission"]["total_secs"])
        self.assertIsNone(metrics["e2e_sync_lock_wait"]["p50_secs"])

    def test_malformed_files_and_nested_values(self):
        for rid, raw in (("bad", "{"), ("array", "[]"), ("encoding", "\xff")):
            path = self.root / rid / "summary.json"
            path.parent.mkdir()
            path.write_bytes(raw.encode("latin1"))
        self.put("nested", phases=[], remote=[], e2e={"segments": [None, [],
                 {"key": [], "secs": 2}, {"key": "remote.admission", "secs": -1}]},
                 locks_waited=[{}, None])
        self.put("invalid", phases={"lock_wait": float("nan")},
                 remote={"sync": {"lock_ms": True}})
        result = lock_report.report(self.root)
        self.assertEqual(result["counts"]["malformed_files"], 3)
        self.assertEqual(result["counts"]["selected_records"], 2)
        self.assertIsNone(result["metrics"]["cargo_lock_estimated"]["total_secs"])
        json.dumps(result, allow_nan=False)

    def test_fallback_and_derived_do_not_claim_measured_admission(self):
        self.put("fallback", e2e={"segments": [{"key": "build.lock_wait", "secs": 4}]})
        self.put("ambiguous", e2e={"segments": [
            {"key": "build.lock_wait", "secs": 8}, {"key": "host.lock_wait", "secs": 8}]})
        self.put("derived", e2e={"derived": True, "segments": [
            {"key": "remote.admission", "secs": 5}, {"key": "sync.lock_wait", "secs": 6}]})
        result = lock_report.report(self.root)
        self.assertEqual(result["sources"]["cargo_e2e_fallback"], 1)
        self.assertEqual(result["metrics"]["cargo_lock_estimated"]["total_secs"], 4)
        self.assertIsNone(result["metrics"]["e2e_admission"]["total_secs"])

    def test_filters_scratch_since_and_limit(self):
        self.put("old", start=1)
        self.put("new", start=200)
        self.put("missing", start=None)
        for rid, cwd in (("local", "/home/me/.justrust/bench/work/project"),
                         ("hosted", "/home/me/.cache/justrust-bench/work/project"),
                         ("synth", "/home/me/.justrust/bench/synth/1"),
                         ("scratch", "/home/me/.jcode/scratch/lock-bench/project")):
            self.put(rid, cwd=cwd, start=300)
        result = lock_report.report(self.root, since=100, limit=1)
        self.assertEqual(result["counts"]["selected_records"], 1)
        self.assertEqual(result["counts"]["excluded_benchmark"], 4)
        self.assertEqual(result["counts"]["excluded_since"], 2)
        self.assertEqual(lock_report.report(self.root, include_benchmarks=True, limit=0)["counts"]["selected_records"], 7)
        self.assertEqual(lock_report.report(self.root, limit=1)["counts"]["excluded_limit"], 2)
        self.assertTrue(lock_report.scratch({"cwd": "/custom/snapshot/repo"}, [Path("/custom/snapshot")]))
        self.assertFalse(lock_report.scratch({"cwd": "/home/me/justrust/bench"}))

    def test_percentiles_samples_and_unknown(self):
        observed = [(str(i), i) for i in range(1, 21)]
        result = lock_report.stats(observed, 22, 2)
        self.assertEqual(result["p50_secs"], 10)
        self.assertEqual(result["p95_secs"], 19)
        self.assertEqual(result["sample_record_ids"], ["20", "19"])
        self.assertEqual(result["missing_records"], 2)
        self.assertIsNone(lock_report.stats([], 1, 2)["total_secs"])

    def test_since_formats(self):
        self.assertEqual(lock_report.since_time("2h", now=10000), 2800)
        self.assertEqual(lock_report.since_time("100"), 100)
        self.assertEqual(lock_report.since_time("1970-01-02T00:00:00Z"), 86400)
        self.assertEqual(lock_report.since_time("1970-01-02"), 86400)
        for value in ("NaN", "inf", "-1", "nonsense"):
            with self.assertRaises(ValueError):
                lock_report.since_time(value)

    def test_cli_stdout_only_json_and_text(self):
        self.put("example", phases={"lock_wait": 3})
        before = {str(p): p.read_bytes() for p in self.root.rglob("*") if p.is_file()}
        for fmt in ("json", "text"):
            output = io.StringIO()
            with contextlib.redirect_stdout(output):
                lock_report.main(["--runs", str(self.root), "--format", fmt, "--limit", "0"])
            if fmt == "json":
                self.assertEqual(json.loads(output.getvalue())["counts"]["selected_records"], 1)
            else:
                self.assertIn("may overestimate", output.getvalue())
                self.assertIn("unknown", output.getvalue())
        after = {str(p): p.read_bytes() for p in self.root.rglob("*") if p.is_file()}
        self.assertEqual(before, after)


if __name__ == "__main__":
    unittest.main()
