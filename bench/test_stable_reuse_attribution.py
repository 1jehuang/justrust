"""Offline tests using shipping Summary fields, plus a frozen repository run."""

import contextlib
import copy
import io
import json
from pathlib import Path
import tempfile
import unittest

import stable_reuse_attribution as attribution


# Field names/units follow Summary, UnitStats, Split, RemoteInfo, E2e and
# RunTiming in src/{summary,e2e,remote_proto}. Durations deliberately overlap.
def shipping_summary():
    return {
        "id": "fixture-hosted", "wall": 5.0, "exit": 0, "toolchain": "jr-exact",
        "place": "hosted", "args": ["test"], "subcommand": "test",
        "remote": {"machine": "hosted", "sync_ms": 1000.0, "client_wall": 8.0,
                   "sync": {"lock_ms": 100, "link_ms": 100, "push_ms": 600, "barrier_ms": 200},
                   "run": {"prepare_ms": 100, "execute_ms": 5500, "finalize_ms": 100, "records_ms": 100}},
        "e2e": {"wall": 9.0, "segments": [{"key": "client.route", "secs": 1},
                                              {"key": "host.compile", "secs": 8}]},
        "units": {"compiled": 3, "with_pass_timings": 2, "rustc_wall_sum": 12,
                  "phase_wall_secs": 10, "llvm_thread_secs": 14,
                  "phase_split": {"frontend": 4, "codegen": 3, "link": 2, "incremental": .5, "other": .5}},
        "link": {"linker_processes": 2, "linker_cpu_secs": 1.5, "link_pass_secs": 2},
        "top_units": [{"wall": 8, "split": {"frontend": 4},
                       "top_passes": [{"name": "total", "secs": 8}, {"name": "type_check_crate", "secs": 2}]}],
    }


class AttributionTests(unittest.TestCase):
    def compare(self, before=None, after=None, **kwargs):
        options = dict(baseline_toolchain="jr-exact", candidate_toolchain="jr-exact",
                       baseline_backend="llvm", candidate_backend="llvm")
        options.update(kwargs)
        return attribution.compare(shipping_summary() if before is None else before,
                                   shipping_summary() if after is None else after, **options)

    def test_shipping_schema_units_and_scopes(self):
        metrics = self.compare()["baseline"]["metrics"]
        for name, expected in [("source_sync_critical_path", 1), ("source_scan_diff_push", .6),
                               ("host_execute", 5.5), ("cargo_wall", 5), ("client_remote_wall", 8),
                               ("client_e2e_wall", 9), ("frontend_main_thread_sum", 4),
                               ("llvm_worker_thread_sum", 14), ("link_pass_sum", 2),
                               ("incremental_cache_io_sum", .5)]:
            with self.subTest(name=name):
                self.assertEqual(metrics[name]["seconds"], expected)
                self.assertEqual(metrics[name]["status"], "recorded")
        self.assertIsNone(metrics["host_wall"]["seconds"])
        self.assertIsNone(metrics["query_cache"]["seconds"])

    def test_real_archived_shipping_summary(self):
        path = Path(__file__).resolve().parents[1] / "data/runs/xps14/readme-cli-acceptance-20261014.jsonl"
        with path.open() as handle:
            record = json.loads(next(handle))
        report = self.compare(record, record, baseline_toolchain=record["toolchain"],
                              candidate_toolchain=record["toolchain"])
        metrics = report["baseline"]["metrics"]
        self.assertEqual(record["id"], "20261014-040524708-2378822")
        self.assertEqual(metrics["cargo_wall"]["seconds"], 16.099)
        self.assertIsNone(metrics["frontend_main_thread_sum"]["seconds"])
        self.assertIsNone(metrics["client_remote_wall"]["seconds"])
        self.assertEqual(report["differences"]["cargo_wall"]["candidate_minus_baseline_seconds"], 0)

    def test_nested_and_parallel_fields_never_added(self):
        record = shipping_summary()
        record["top_units"] *= 20
        record["link"]["link_pass_secs"] = 999  # redundant aggregate, never added
        metrics = self.compare(record, record)["baseline"]["metrics"]
        self.assertEqual(metrics["frontend_main_thread_sum"]["seconds"], 4)
        self.assertEqual(metrics["link_pass_sum"]["seconds"], 2)
        self.assertEqual(metrics["client_e2e_wall"]["seconds"], 9)
        self.assertNotIn("total", metrics)
        self.assertNotIn("unattributed", metrics)

    def test_missing_and_null_are_unknown(self):
        record = {"id": "old", "wall": 0, "remote": None, "units": {"phase_split": None}}
        metrics = self.compare(record, record)["baseline"]["metrics"]
        self.assertEqual(metrics["cargo_wall"]["seconds"], 0)
        for name, metric in metrics.items():
            if name != "cargo_wall":
                self.assertIsNone(metric["seconds"], name)
                self.assertEqual(metric["status"], "unknown", name)

    def test_explicit_zero_not_missing_except_ambiguous_llvm_default(self):
        record = shipping_summary()
        record["remote"]["sync_ms"] = 0
        record["units"]["llvm_thread_secs"] = 0
        metrics = self.compare(record, record)["baseline"]["metrics"]
        self.assertEqual(metrics["source_sync_critical_path"]["seconds"], 0)
        self.assertIsNone(metrics["llvm_worker_thread_sum"]["seconds"])

    def test_missing_candidate_does_not_create_saving(self):
        report = self.compare(after={"id": "old", "wall": 3})
        self.assertEqual(report["differences"]["cargo_wall"]["candidate_minus_baseline_seconds"], -2)
        self.assertIsNone(report["differences"]["frontend_main_thread_sum"]["candidate_minus_baseline_seconds"])

    def test_identity_required_and_equal(self):
        for options in [{"baseline_toolchain": ""}, {"candidate_toolchain": "other"},
                        {"candidate_backend": "cranelift"}, {"baseline_backend": " "}]:
            with self.subTest(options=options), self.assertRaises(ValueError):
                self.compare(**options)

    def test_recorded_identity_conflicts(self):
        for field, value in [("toolchain", "different"), ("backend", "cranelift"),
                             ("codegen_backend", "cranelift"), ("rustc", "different"),
                             ("compiler", "different")]:
            before, after = shipping_summary(), shipping_summary()
            before[field] = "llvm" if "backend" in field else "original"
            after[field] = value
            with self.subTest(field=field), self.assertRaises(ValueError):
                self.compare(before, after)

    def test_identity_one_sided_backend_checked(self):
        record = shipping_summary()
        record["backend"] = "cranelift"
        with self.assertRaises(ValueError):
            self.compare(after=record)

    def test_invalid_numeric_values(self):
        for value in [-1, float("inf"), float("nan"), True, "1", []]:
            record = shipping_summary()
            record["units"]["phase_split"]["frontend"] = value
            with self.subTest(value=value), self.assertRaises(ValueError):
                self.compare(after=record)

    def test_invalid_roots(self):
        for record in [[], None, {}, {"wall": 2}]:
            with self.subTest(record=record), self.assertRaises(ValueError):
                attribution.compare(record, shipping_summary(), "jr-exact", "jr-exact", "llvm", "llvm")

    def test_derived_and_inconsistent_e2e_disclosed(self):
        record = shipping_summary()
        record["e2e"]["derived"] = True
        record["e2e"]["segments"][0]["secs"] = 100
        result = self.compare(record, record)["baseline"]
        self.assertEqual(result["metrics"]["client_e2e_wall"]["status"], "derived")
        self.assertEqual(len(result["warnings"]), 2)
        self.assertEqual(result["metrics"]["client_e2e_wall"]["seconds"], 9)

    def test_derived_difference_not_labeled_measured(self):
        record = shipping_summary()
        record["e2e"]["derived"] = True
        report = self.compare(after=record)
        self.assertEqual(report["differences"]["client_e2e_wall"]["status"], "derived_difference")
        record["e2e"]["wall"] = None
        self.assertEqual(self.compare(after=record)["candidate"]["metrics"]["client_e2e_wall"]["status"], "unknown")

    def test_failed_run_disclosed(self):
        record = shipping_summary()
        record["exit"] = 101
        self.assertIn("run failed", self.compare(after=record)["candidate"]["warnings"][0])

    def test_cli_reports_input_errors(self):
        with tempfile.TemporaryDirectory() as directory:
            path = Path(directory) / "invalid.json"
            path.write_text("[]")
            with contextlib.redirect_stderr(io.StringIO()), self.assertRaises(SystemExit) as raised:
                attribution.main([str(path), str(path), "--baseline-toolchain", "jr-exact",
                                  "--candidate-toolchain", "jr-exact", "--baseline-backend", "llvm",
                                  "--candidate-backend", "llvm"])
            self.assertEqual(raised.exception.code, 2)

    def test_does_not_mutate_input(self):
        record = shipping_summary()
        original = copy.deepcopy(record)
        self.compare(record, record)
        self.assertEqual(record, original)

    def test_cli_offline_json(self):
        with tempfile.TemporaryDirectory() as directory:
            path = Path(directory) / "summary.json"
            path.write_text(json.dumps(shipping_summary()))
            output = io.StringIO()
            with contextlib.redirect_stdout(output):
                attribution.main([str(path), str(path), "--baseline-toolchain", "jr-exact",
                                  "--candidate-toolchain", "jr-exact", "--baseline-backend", "llvm",
                                  "--candidate-backend", "llvm"])
            self.assertEqual(json.loads(output.getvalue())["schema_version"], 1)

    def test_cli_requires_identity(self):
        with contextlib.redirect_stderr(io.StringIO()), self.assertRaises(SystemExit) as raised:
            attribution.main(["before.json", "after.json"])
        self.assertEqual(raised.exception.code, 2)


if __name__ == "__main__":
    unittest.main()
