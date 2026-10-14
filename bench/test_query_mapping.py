"""Measured query-to-dependency mapping tests, never execution-span assertions."""
import contextlib
import copy
import hashlib
import io
import json
import os
from pathlib import Path
import struct
import tempfile
import unittest
import query_mapping as mapping
from test_query_graph import capture
from test_query_profile import BASE, META, fixture, raw


def event(node, start, end, kind="Query", thread=0, label=None):
    return dict(event_id=node, start_ns=start, end_ns=end, kind=kind,
                thread=thread, record_type="interval", label=label,
                label_resolved=label is not None)


def graph():
    return {"nodes": {0: {"kind": 8, "dependencies": [1]},
                      1: {"kind": 9, "dependencies": []},
                      2: {"kind": 10, "dependencies": []}}, "node_count": 3}


class MappingTests(unittest.TestCase):
    def test_streamed_records_match_materialized_mapping(self):
        records = [event(0, 0, 10), event(1, 2, 6), event(1, 6, 8, 'QueryBlocked')]
        self.assertEqual(mapping.map_events(graph(), iter(records)), mapping.map_events(graph(), records))

    def test_nested_query_and_known_wait_not_double_counted(self):
        result = mapping.map_events(graph(), [event(0, 0, 10), event(1, 2, 6),
                                              event(1, 6, 8, "QueryBlocked")])
        self.assertEqual(result["observed_queries"][0]["exclusive_elapsed_ns"], 4)
        self.assertEqual(result["observed_queries"][1]["exclusive_elapsed_ns"], 4)
        self.assertEqual(result["known_query_blocked_elapsed_ns"], 2)
        self.assertEqual(result["unobserved_node_count"], 1)
        self.assertIsNone(result["execution_span_ns"])
        self.assertIsNone(result["cpu_work_ns"])
        self.assertEqual(result["blocked_relations"][0]["producer_id"], 1)
        self.assertEqual(result["blocked_relations"][0]["caller_id"], 0)

    def test_cross_thread_producer_is_not_caller(self):
        result = mapping.map_events(graph(), [event(0, 0, 10), event(1, 0, 5, thread=1),
                                              event(1, 2, 5, "QueryBlocked")])
        self.assertEqual(result["observed_queries"][0]["exclusive_elapsed_ns"], 7)
        self.assertEqual(result["blocked_relations"][0]["caller_id"], 0)
        self.assertEqual(result["blocked_relations"][0]["producer_id"], 1)

    def test_equal_labels_do_not_merge_numeric_ids(self):
        result = mapping.map_events(graph(), [event(0, 0, 2, label="same"),
                                              event(1, 2, 5, label="same")])
        self.assertEqual(set(result["observed_queries"]), {0, 1})

    def test_unresolved_labels_and_unknown_nodes_retained(self):
        result = mapping.map_events(graph(), [event(99, 0, 3)])
        self.assertEqual(result["provider_records_without_graph_node"], 1)
        self.assertEqual(result["provider_records_with_unresolved_label"], 1)
        self.assertEqual(result["unobserved_node_count"], 3)
        self.assertEqual(result["unknown_provider_ids"], [99])

    def test_noninterval_counters_never_become_elapsed(self):
        counter = dict(event_id=0, kind="QueryCacheHitCount", record_type="integer",
                       thread=0, label=None, label_resolved=False, value=100)
        result = mapping.map_events(graph(), [counter])
        self.assertEqual(result["exclusive_query_elapsed_ns"], 0)
        self.assertEqual(result["observed_queries"], {})

    def test_crossing_same_thread_query_intervals_rejected(self):
        with self.assertRaisesRegex(ValueError, "crossing"):
            mapping.map_events(graph(), [event(0, 0, 6), event(1, 3, 9)])

    def test_zero_duration_and_duplicate_numeric_identity(self):
        result = mapping.map_events(graph(), [event(0, 0, 0), event(0, 1, 3), event(0, 4, 7)])
        self.assertEqual(result["observed_queries"][0]["exclusive_elapsed_ns"], 5)
        self.assertEqual(result["observed_queries"][0]["provider_records"], 3)

    def test_invalid_interval_rejected(self):
        for bad in [event(0, 3, 2), event(True, 1, 2), event(0, -1, 2)]:
            with self.assertRaises(ValueError):
                mapping.map_events(graph(), [bad])

    def test_nested_waits_partition_without_double_counting(self):
        result = mapping.map_events(graph(), [event(0, 0, 10),
            event(1, 2, 8, "QueryBlocked"), event(2, 3, 5, "QueryBlocked")])
        self.assertEqual(result["exclusive_query_elapsed_ns"], 4)
        self.assertEqual(result["known_query_blocked_elapsed_ns"], 6)
        self.assertEqual(result["inclusive_query_blocked_elapsed_ns"], 8)

    def test_wait_without_caller_and_missing_producer_remains_explicit(self):
        result = mapping.map_events(graph(), [event(99, 1, 4, "QueryBlocked")])
        relation = result["blocked_relations"][0]
        self.assertIsNone(relation["caller_id"])
        self.assertFalse(relation["producer_in_graph"])
        self.assertFalse(relation["producer_provider_observed"])
        self.assertEqual(result["known_query_blocked_elapsed_ns"], 3)


def paired_fixture():
    metadata = dict(process_id=42, start_time=100, cmd="rustc --crate-name example -Cincremental=/captured/inc")
    strings = b"Query\xfflabel\xff" + json.dumps(metadata).encode() + b"\xff"
    profile = fixture(strings=strings, index=struct.pack("<QQQQ", 0, 14, META, 20),
                      events=raw(BASE + 8, 0, 1, 2, 4))
    wire_graph = capture([(0, 8, [])])
    receipt = dict(compiler_sha256=mapping.PINNED_COMPILER_SHA256,
        graph_sha256=hashlib.sha256(wire_graph).hexdigest(),
        profile_sha256=hashlib.sha256(profile).hexdigest(),
        run_id="20261014-000000001-1", process_id=42,
        remote_profile="/captured/example-0000042.mm_profdata",
        incremental_root="/captured/inc", remote_graph="/captured/inc/session/dep-graph.bin",
        crate_name="example", unit_start_ns=90, unit_end_ns=110)
    return wire_graph, profile, metadata, receipt


class ProvenanceTests(unittest.TestCase):
    def test_matching_receipt_and_last_incremental_override(self):
        wire, profile, metadata, receipt = paired_fixture()
        metadata["cmd"] = "rustc -Cincremental=/unused " + metadata["cmd"]
        result = mapping.verify_pair(mapping.query_graph.decode(wire), hashlib.sha256(profile).hexdigest(), metadata, receipt)
        self.assertEqual(result["process_id"], 42)
        self.assertNotIn("profile_command", result)
        self.assertIn("not an independent cryptographic attestation", result["trust_scope"])

    def test_each_receipt_identity_mismatch_is_rejected(self):
        wire, profile, metadata, receipt = paired_fixture()
        for key, value in [("compiler_sha256", "0" * 64), ("graph_sha256", "0" * 64),
                ("profile_sha256", "0" * 64), ("run_id", "bad"), ("process_id", 43),
                ("remote_profile", "/captured/example-43.mm_profdata"),
                ("remote_profile", "example-42.mm_profdata"),
                ("remote_graph", "/outside/dep-graph.bin"),
                ("remote_graph", "/captured/inc/../dep-graph.bin"),
                ("incremental_root", "/wrong"), ("crate_name", "wrong"),
                ("unit_start_ns", 101), ("unit_end_ns", 99), ("unit_start_ns", True)]:
            altered = copy.deepcopy(receipt)
            altered[key] = value
            with self.subTest(key=key, value=value), self.assertRaises(ValueError):
                mapping.verify_pair(mapping.query_graph.decode(wire), hashlib.sha256(profile).hexdigest(), metadata, altered)

    def test_metadata_mismatch_is_rejected(self):
        wire, profile, metadata, receipt = paired_fixture()
        for key, value in [("process_id", True), ("start_time", 111),
                ("cmd", "rustc --crate-name example -Cincremental=/wrong"),
                ("cmd", "rustc --crate-name other -Cincremental=/captured/inc")]:
            altered = dict(metadata, **{key: value})
            with self.subTest(key=key), self.assertRaises(ValueError):
                mapping.verify_pair(mapping.query_graph.decode(wire), hashlib.sha256(profile).hexdigest(), altered, receipt)

    def test_public_cli_writes_bounded_readonly_mapping_and_refuses_overwrite(self):
        wire, profile, _, receipt = paired_fixture()
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            (root / "graph").write_bytes(wire)
            (root / "profile").write_bytes(profile)
            (root / "receipt").write_text(json.dumps(receipt))
            args = ["--graph", str(root / "graph"), "--profile", str(root / "profile"),
                    "--receipt", str(root / "receipt"), "--output", str(root / "output")]
            with contextlib.redirect_stdout(io.StringIO()):
                mapping.main(args)
            result = json.loads((root / "output").read_text())
            self.assertEqual(result["provider_records"], 1)
            self.assertEqual(result["exclusive_query_elapsed_ns"], 2)
            self.assertIsNone(result["execution_span_ns"])
            self.assertEqual((root / "output").stat().st_mode & 0o777, 0o400)
            before = (root / "output").read_bytes()
            with contextlib.redirect_stderr(io.StringIO()), self.assertRaises(SystemExit) as error:
                mapping.main(args)
            self.assertEqual(error.exception.code, 2)
            self.assertEqual((root / "output").read_bytes(), before)

    def test_large_capture_public_cli_and_postwake_scope(self):
        wire, profile, _, receipt = paired_fixture()
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            (root / 'graph').write_bytes(wire)
            (root / 'profile').write_bytes(profile)
            (root / 'receipt').write_text(json.dumps(receipt))
            args = ['--graph', str(root / 'graph'), '--profile', str(root / 'profile'),
                    '--receipt', str(root / 'receipt'), '--output', str(root / 'output'), '--large-capture']
            with contextlib.redirect_stdout(io.StringIO()):
                mapping.main(args)
            result = json.loads((root / 'output').read_text())
            self.assertEqual(result['provider_records'], 1)
            self.assertTrue(any('post-wake' in item for item in result['limitations']))
            self.assertIsNone(result['cpu_work_ns'])

    def test_receipt_read_cap_and_symlink_rejection(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            (root / "oversize").write_bytes(b"x" * 65537)
            with self.assertRaisesRegex(ValueError, "64KiB"):
                mapping.read_receipt(root / "oversize")
            (root / "normal").write_bytes(b"{}")
            (root / "linked").symlink_to(root / "normal")
            with self.assertRaises((ValueError, OSError)):
                mapping.read_receipt(root / "linked")
            (root / "dirlink").symlink_to(root, target_is_directory=True)
            with self.assertRaises((ValueError, OSError)):
                mapping.read_receipt(root / "dirlink" / "normal")


if __name__ == "__main__":
    unittest.main()
