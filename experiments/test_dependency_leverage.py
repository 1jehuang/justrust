"""Synthetic profile comparison and command-line contract tests."""
import contextlib
import copy
import importlib.util
import io
from pathlib import Path
import sys
import tempfile
import unittest
from unittest import mock

SPEC = importlib.util.spec_from_file_location("dependency_leverage", Path(__file__).with_name("dependency_leverage.py"))
probe = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(probe)


def profile(identifier=1, **overrides):
    row = {"id": identifier, "kind": 102, "version": 1, "phase": "body", "params": [0],
           "direct_dependencies": 2, "direct_consumers": 3,
           "transitive_dependencies": 5, "transitive_consumers": 7,
           "closure_complete": True, "summed_wall_ns": 100, "summed_wait_ns": 50}
    row.update(overrides)
    return {"graph": {"dependency_leverage": {
        "edge_attribution_complete": True, "intervals_complete": True, "unattributed_edges": 0,
        "missing_producers": 0, "closure_edge_visit_limit": 1000,
        "top_nodes_limit": 128, "omitted_nodes": 0, "ranking": "consumers",
        "top_nodes": [row]}}}


def first_comparison(a, b):
    return probe.compare_profiles(a, b)["target_nodes"][0]


class ComparisonTests(unittest.TestCase):
    def test_stable_keys_ignore_ephemeral_ids_and_phase(self):
        row = first_comparison(profile(10), profile(900, phase="renamed", direct_dependencies=8,
                                                   summed_wall_ns=90))
        self.assertTrue(row["available"])
        self.assertEqual(row["deltas"]["direct_dependencies"], 6)
        self.assertEqual(row["deltas"]["summed_wall_ns"], -10)
        self.assertEqual(row["deltas"]["transitive_consumers"], 0)

    def test_same_id_different_params_not_matched(self):
        row = first_comparison(profile(), profile(params=[1]))
        self.assertFalse(row["available"])
        self.assertIsNone(row["after"])
        self.assertTrue(all(v is None for v in row["deltas"].values()))

    def test_different_kind_not_matched(self):
        self.assertFalse(first_comparison(profile(), profile(kind=107))["available"])

    def test_missing_profile_section_is_unavailable(self):
        result = probe.compare_profiles({}, profile())
        self.assertIsNone(result["base_coverage"])
        self.assertIsNone(result["target_nodes"][0]["before"])

    def test_omitted_rows_are_not_zero(self):
        changed = profile()
        section = changed["graph"]["dependency_leverage"]
        section["top_nodes"] = []
        section["omitted_nodes"] = 400
        result = probe.compare_profiles(profile(), changed)
        self.assertEqual(result["variant_coverage"]["omitted_nodes"], 400)
        self.assertIsNone(result["target_nodes"][0]["deltas"]["direct_consumers"])

    def test_incomplete_closure_has_no_transitive_delta(self):
        row = first_comparison(profile(), profile(closure_complete=False))
        self.assertIsNone(row["deltas"]["transitive_dependencies"])
        self.assertEqual(row["deltas"]["direct_dependencies"], 0)
        self.assertFalse(row["after"]["closure_complete"])

    def test_incomplete_attribution_has_no_transitive_delta(self):
        changed = profile()
        changed["graph"]["dependency_leverage"]["edge_attribution_complete"] = False
        self.assertIsNone(first_comparison(profile(), changed)["deltas"]["transitive_consumers"])

    def test_missing_metric_not_zero(self):
        changed = profile()
        del changed["graph"]["dependency_leverage"]["top_nodes"][0]["summed_wait_ns"]
        self.assertIsNone(first_comparison(profile(), changed)["deltas"]["summed_wait_ns"])

    def test_duplicate_stable_keys_rejected(self):
        data = profile()
        section = data["graph"]["dependency_leverage"]
        section["top_nodes"].append(copy.deepcopy(section["top_nodes"][0]))
        with self.assertRaisesRegex(ValueError, "duplicate stable key"):
            probe.compare_profiles(data, profile())

    def test_versions_do_not_cross_match(self):
        result = probe.compare_profiles(profile(), profile(version=2))
        rows = [row for row in result["target_nodes"] if row["kind"] == 102]
        self.assertEqual([row["version"] for row in rows], [1, 2])
        self.assertTrue(all(not row["available"] for row in rows))
        self.assertTrue(all(all(value is None for value in row["deltas"].values()) for row in rows))

    def test_globals_compared_by_full_key(self):
        for kind in (101, 106):
            result = probe.compare_profiles(profile(kind=kind, params=[]),
                                            profile(80, kind=kind, params=[], direct_consumers=9))
            row = next(row for row in result["global_nodes"] if row["kind"] == kind)
            self.assertTrue(row["available"])
            self.assertEqual(row["deltas"]["direct_consumers"], 6)
            self.assertEqual(row["params"], [])

    def test_all_breadth_gated_on_attribution(self):
        changed = profile()
        changed["graph"]["dependency_leverage"]["edge_attribution_complete"] = False
        row = first_comparison(profile(), changed)
        self.assertTrue(all(row["deltas"][name] is None for name in probe.BREADTH))
        self.assertEqual(row["deltas"]["summed_wall_ns"], 0)

    def test_intervals_gate_timing_only(self):
        changed = profile()
        changed["graph"]["dependency_leverage"]["intervals_complete"] = False
        row = first_comparison(profile(), changed)
        self.assertTrue(all(row["deltas"][name] is None for name in probe.TIMING))
        self.assertEqual(row["deltas"]["direct_dependencies"], 0)

    def test_missing_flags_are_not_complete(self):
        changed = profile()
        del changed["graph"]["dependency_leverage"]["intervals_complete"]
        del changed["graph"]["dependency_leverage"]["edge_attribution_complete"]
        row = first_comparison(profile(), changed)
        self.assertTrue(all(value is None for value in row["deltas"].values()))

    def test_telemetry_parity(self):
        data = {"nodes": 20, "edges": 40, "capture_complete": True,
                "failures": 0, "events_dropped": 0, "total_ns": 123}
        other = dict(data, total_ns=999)
        self.assertEqual(probe.telemetry_parity(data, other)["nodes"], 20)
        for changed in (dict(other, nodes=21), dict(other, capture_complete=False),
                        dict(other, failures=1), dict(other, events_dropped=1)):
            with self.subTest(changed=changed), self.assertRaises(ValueError):
                probe.telemetry_parity(data, changed)
        del other["edges"]
        with self.assertRaisesRegex(ValueError, "missing baseline counters"):
            probe.telemetry_parity(data, other)

    def test_first_target_and_main_are_unchanged(self):
        variants = probe.sources(16)
        for text in variants.values():
            self.assertEqual(text.splitlines()[:2], [probe.TARGET, probe.MAIN])
        self.assertEqual(variants["base"].count("fn unrelated_"), 1)
        self.assertEqual(variants["unrelated-declarations-added"].count("fn unrelated_"), 17)
        self.assertNotEqual(variants["base"], variants["unrelated-body-edit"])


class CliTests(unittest.TestCase):
    def assert_cli_error(self, arguments, expected):
        stderr = io.StringIO()
        with contextlib.redirect_stderr(stderr), self.assertRaises(SystemExit) as caught:
            probe.main(arguments)
        self.assertEqual(caught.exception.code, 2)
        self.assertIn(expected, stderr.getvalue())

    def test_defaults(self):
        args = probe.parser().parse_args(["--binary", "jr", "--output", "out"])
        self.assertEqual((args.workers, args.extra_functions, args.repeats), ([1, 8], 16, 1))

    def test_missing_required(self):
        self.assert_cli_error([], "required")

    def test_nonpositive_or_noninteger(self):
        for option in ("--workers", "--extra-functions", "--repeats"):
            for value in ("0", "-1", "oops"):
                with self.subTest(option=option, value=value):
                    self.assert_cli_error(["--binary", "jr", "--output", "out", option, value],
                                          "positive integer")

    def test_missing_binary(self):
        self.assert_cli_error(["--binary", "/nonexistent-jr-binary", "--output", "/unused"],
                              "executable file")

    def test_existing_output(self):
        with tempfile.TemporaryDirectory() as directory:
            self.assert_cli_error(["--binary", sys.executable, "--output", directory], "new directory")

    def test_repository_output_rejected(self):
        output = Path(__file__).resolve().parents[1] / "dependency-leverage-unwanted-output"
        self.assert_cli_error(["--binary", sys.executable, "--output", str(output)], "outside the repository")
        self.assertFalse(output.exists())

    def test_missing_rustc(self):
        with tempfile.TemporaryDirectory() as directory, mock.patch.object(probe.shutil, "which", return_value=None):
            output = Path(directory) / "new"
            self.assert_cli_error(["--binary", sys.executable, "--output", str(output)], "rustc is required")
            self.assertFalse(output.exists())


if __name__ == "__main__":
    unittest.main()
