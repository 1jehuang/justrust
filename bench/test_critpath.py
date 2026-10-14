import copy
import json
from pathlib import Path
import subprocess
import sys
import tempfile
import unittest
from unittest.mock import patch

sys.path.insert(0, str(Path(__file__).resolve().parent))
import critpath


def event(name, start, duration, thread=1, category="", process=0):
    return {"name": name, "ts": start * 1e6, "dur": duration * 1e6,
            "tid": thread, "pid": process, "cat": category}


class OccupancyTests(unittest.TestCase):
    def check_integrals(self, result):
        histogram = result["occupancy_histogram_s"]
        self.assertAlmostEqual(sum(histogram.values()), result["observed_event_span_s"])
        self.assertAlmostEqual(sum(count * seconds for count, seconds in histogram.items()),
                               result["reported_nonblocked_thread_s"])
        self.assertAlmostEqual(sum(seconds for _, seconds in result["single_thread_by_phase"]),
                               result["single_reported_thread_s"])

    def test_nesting_counts_once(self):
        events = [event("expand_crate", 0, 10), event("expand_proc_macro", 2, 3)]
        original = copy.deepcopy(events)
        result = critpath.analyze(events)
        self.assertEqual(events, original)
        self.assertEqual(dict(result["exclusive_elapsed_by_label"]),
                         {"expand_crate": 7, "expand_proc_macro": 3})
        self.assertEqual(result["average_reported_nonblocked_threads"], 1)
        self.check_integrals(result)

    def test_wait_suppresses_enclosing_parent(self):
        result = critpath.analyze([event("mono", 0, 10),
                                   event("upstream_drop_glue_for", 2, 4, category="QueryBlocked")])
        self.assertEqual(result["reported_nonblocked_thread_s"], 6)
        self.assertEqual(result["reported_blocked_thread_s"], 4)
        self.assertEqual(result["no_reported_nonblocked_thread_s"], 4)
        self.check_integrals(result)

    def test_parallel_worker_during_query_wait(self):
        result = critpath.analyze([event("mono", 0, 10),
                                   event("waiting", 2, 4, category="QueryBlocked"),
                                   event("metadata", 1, 6, thread=2)])
        self.assertEqual(result["reported_nonblocked_thread_s"], 12)
        self.assertEqual(result["occupancy_histogram_s"], {1: 8, 2: 2})
        self.assertEqual(dict(result["single_thread_by_label"]), {"mono": 4, "metadata": 4})
        self.check_integrals(result)

    def test_nested_wait_is_not_double_counted(self):
        result = critpath.analyze([event("mono", 0, 10),
                                   event("wait", 2, 4, category="QueryBlocked"),
                                   event("wait_child", 3, 2, category="QueryBlocked")])
        self.assertEqual(result["reported_blocked_thread_s"], 4)
        self.check_integrals(result)

    def test_equal_boundaries_and_unsorted_events(self):
        result = critpath.analyze([event("second", 5, 5), event("child", 0, 5), event("parent", 0, 10)])
        self.assertEqual(result["reported_nonblocked_thread_s"], 10)
        self.assertEqual(dict(result["exclusive_elapsed_by_label"]), {"child": 5, "second": 5})
        self.check_integrals(result)

    def test_same_tid_different_processes(self):
        result = critpath.analyze([event("a", 0, 5), event("b", 0, 5, process=1)])
        self.assertEqual(result["threads"], 2)
        self.assertEqual(result["occupancy_histogram_s"], {2: 5})
        self.check_integrals(result)

    def test_gap_is_not_called_cpu_idle(self):
        result = critpath.analyze([event("a", 0, 2), event("b", 5, 2)])
        self.assertEqual(result["no_reported_nonblocked_thread_s"], 3)
        self.assertNotIn("idle_s", result)
        self.check_integrals(result)

    def test_no_inferred_work_span_or_speedup(self):
        result = critpath.analyze([event("a", 0, 3), event("b", 0, 3, thread=2)])
        for name in ("cpu_work_s", "causal_dependency_span_s", "theoretical_speedup_ceiling"):
            self.assertIsNone(result[name])
        for name in ("W", "S", "parallelism", "serial_s"):
            self.assertNotIn(name, result)

    def test_crossing_intervals_are_rejected(self):
        with self.assertRaisesRegex(ValueError, "crossing intervals"):
            critpath.analyze([event("a", 0, 3), event("b", 2, 3)])

    def test_crox_microsecond_quantization_is_reported(self):
        result = critpath.analyze([event("a", 0, 3), event("b", 2, 1.000001)])
        self.assertEqual(result["rounded_endpoints_clipped"], 1)
        self.assertAlmostEqual(result["rounded_endpoint_clipping_us"], 1)
        self.assertEqual(result["reported_nonblocked_thread_s"], 3)
        self.check_integrals(result)

    def test_empty_and_zero_only_traces_are_rejected(self):
        for events in ([], [event("zero", 0, 0)]):
            with self.assertRaisesRegex(ValueError, "no positive-duration"):
                critpath.analyze(events)

    def test_invalid_numbers_are_rejected(self):
        for start, duration in [(0, -1), (float("inf"), 1), (0, float("nan"))]:
            with self.assertRaisesRegex(ValueError, "finite"):
                critpath.analyze([event("invalid", start, duration)])

    def test_real_cli_and_filtered_json(self):
        with tempfile.TemporaryDirectory() as directory:
            source = Path(directory) / "trace.json"
            output = Path(directory) / "out.json"
            events = [dict(event("parent", 0, 1), ph="X"),
                      dict(event("short_wait", 0.2, 0.00001, category="QueryBlocked"), ph="X"),
                      dict(event("self_profile_alloc", 2, 20), ph="X"),
                      {"ph": "M", "name": "thread_name"}]
            source.write_text(json.dumps({"traceEvents": events}))
            self.assertEqual(len(critpath.load_events(str(source))), 2)
            self.assertEqual(len(critpath.load_events(str(source), 50)), 1)
            command = [sys.executable, str(Path(critpath.__file__)), str(source), "--json", str(output)]
            result = subprocess.run(command, capture_output=True, text=True)
            self.assertEqual(result.returncode, 0, result.stderr)
            self.assertIn("NOT a causal critical path", result.stdout)
            actual = json.loads(output.read_text())
            self.assertEqual(actual["schema_version"], 2)
            self.assertAlmostEqual(actual["reported_blocked_thread_s"], 0.00001)
            self.assertEqual(actual["observed_event_span_s"], 1)
            self.assertIsNone(actual["theoretical_speedup_ceiling"])
            source.write_text("[]")
            failed = subprocess.run(command, capture_output=True, text=True)
            self.assertEqual(failed.returncode, 2)
            self.assertIn("no positive-duration", failed.stderr)

    def test_crox_integration_requires_integer_threshold(self):
        def convert(command, **kwargs):
            self.assertEqual(command[1:3], ["--minimum-duration", "50"])
            trace = [dict(event("mono", 0, 1), ph="X")]
            (Path(kwargs["cwd"]) / "chrome_profiler.json").write_text(json.dumps(trace))
            return subprocess.CompletedProcess(command, 0, "", "")
        with patch.object(critpath.subprocess, "run", side_effect=convert):
            self.assertEqual(len(critpath.load_events("test.mm_profdata", 50)), 1)

    def test_invalid_json_duration_is_not_silently_filtered(self):
        with tempfile.TemporaryDirectory() as directory:
            source = Path(directory) / "trace.json"
            source.write_text(json.dumps([dict(event("invalid", 0, -1), ph="X")]))
            with self.assertRaisesRegex(ValueError, "nonnegative"):
                critpath.load_events(str(source), 50)


if __name__ == "__main__":
    unittest.main()
