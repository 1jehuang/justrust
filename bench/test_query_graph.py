"""Pinned saved-graph decoding tests, not synthetic evidence of compiler speed."""
import struct
import unittest
from unittest import mock

import query_graph as graph


def leb(value):
    out = bytearray()
    while value >= 128:
        out.append((value & 127) | 128)
        value >>= 7
    out.append(value)
    return bytes(out)


def capture(rows, node_max=None):
    data = b"RSIC\0\0" + bytes([len(graph.VERSION)]) + graph.VERSION + bytes(8)
    counts = [0] * 337
    edges = 0
    for index, kind, dependencies in rows:
        counts[kind] += 1
        width = max(1, (max(dependencies, default=0).bit_length() + 7) // 8)
        head = kind | ((width - 1) << 9)
        head |= (len(dependencies) + 1) << 11 if len(dependencies) <= 30 else 0
        data += struct.pack("<HI", head, index) + bytes(32)
        if len(dependencies) > 30:
            data += leb(len(dependencies))
        data += b"".join(x.to_bytes(width, "little") for x in dependencies)
        edges += len(dependencies)
    data += b"".join(leb(x) for x in counts) + leb(1)
    maximum = max((r[0] for r in rows), default=-1) + 1
    data += struct.pack("<QQQ", maximum if node_max is None else node_max, len(rows), edges)
    return data + b"rust-end-file"


class DecodeTests(unittest.TestCase):
    def test_out_of_order_actual_ids_and_holes(self):
        result = graph.decode(capture([(256, 2, [0]), (0, 3, [])], 512))
        self.assertEqual(result["nodes"][256]["dependencies"], [0])
        self.assertEqual(result["holes"], 510)
        self.assertEqual(result["edge_count"], 1)

    def test_all_edge_widths_and_escaped_length(self):
        for target in [1, 256, 65536, 16777216]:
            if target >= graph.MAX_NODE_SPACE:
                # Test wire widths directly with a bounded custom node space.
                with unittest.mock.patch.object(graph, "MAX_NODE_SPACE", target + 2):
                    result = graph.decode(capture([(0, 1, [target]), (target, 2, [])]))
            else:
                result = graph.decode(capture([(0, 1, [target]), (target, 2, [])]))
            self.assertEqual(result["nodes"][0]["dependencies"], [target])
        rows = [(0, 1, list(range(1, 32)))] + [(n, 2, []) for n in range(1, 32)]
        self.assertEqual(graph.decode(capture(rows))["edge_count"], 31)

    def test_source_kind_counts(self):
        result = graph.decode(capture([(0, 336, [])]))
        self.assertEqual(result["kind_counts"][336], 1)

    def test_magic_version_and_trailer_rejected(self):
        good = capture([(0, 1, [])])
        for bad in [b"BAD!" + good[4:], good[:-1], good[:7] + b"x" + good[8:]]:
            with self.assertRaises(ValueError):
                graph.decode(bad)

    def test_duplicate_dangling_and_self_edges(self):
        for rows in [[(0, 1, []), (0, 2, [])], [(0, 1, [1])], [(0, 1, [0])],
                     [(0, 1, [1, 1]), (1, 2, [])]]:
            with self.assertRaises(ValueError):
                graph.decode(capture(rows))

    def test_null_and_outside_index_rejected(self):
        for raw in [capture([(0, 0, [])]), capture([(3, 1, [])], 2)]:
            with self.assertRaises(ValueError):
                graph.decode(raw)

    def test_footer_count_mismatch_and_truncation(self):
        raw = capture([(0, 1, [])])
        altered = raw[:-37] + struct.pack("<QQQ", 1, 2, 0) + raw[-13:]
        for bad in [altered, raw[:55] + raw[56:]]:
            with self.assertRaises(ValueError):
                graph.decode(bad)

    def test_footer_only_mutations_rejected(self):
        raw = capture([(0, 1, [])])
        for field in range(3):
            for value in [0, 2**32, 2**64 - 1]:
                fields = [1, 1, 0]
                if fields[field] == value:
                    continue
                fields[field] = value
                altered = raw[:-37] + struct.pack("<QQQ", *fields) + raw[-13:]
                self.assertEqual(len(altered), len(raw))
                with self.assertRaises(ValueError):
                    graph.decode(altered)

    def test_leb_corruption_rejected(self):
        for value in [b"\x80\x00", b"\x80" * 5, b"\xff" * 5, b"\x80"]:
            with self.assertRaises(ValueError):
                graph.Reader(value).leb(32)

    def test_fixed_seed_sparse_dag_record_permutations(self):
        import random
        rng = random.Random(20261014)
        for _ in range(100):
            ids = rng.sample(range(0, 5000), 30)
            rows = [(node, 1 + rng.randrange(336), rng.sample(ids[:i], rng.randrange(min(i, 4) + 1)))
                    for i, node in enumerate(ids)]
            first = graph.decode(capture(rows))
            rng.shuffle(rows)
            second = graph.decode(capture(rows))
            self.assertEqual(first["nodes"], second["nodes"])
            self.assertEqual(graph.structure(first), graph.structure(second))

    def test_kind_count_mismatch(self):
        raw = bytearray(capture([(0, 1, [])]))
        start = 7 + len(graph.VERSION) + 8 + 38
        raw[start + 1] = 0
        with self.assertRaises(ValueError):
            graph.decode(bytes(raw))

    def test_empty_graph_rejected(self):
        with self.assertRaises(ValueError):
            graph.decode(capture([]))

    def test_cycle_rejected_by_structural_mapping(self):
        parsed = graph.decode(capture([(0, 1, [1]), (1, 2, [0])]))
        with self.assertRaisesRegex(ValueError, "cycle"):
            graph.structure(parsed)

    def test_diamond_has_depth_not_claimed_duration_span(self):
        parsed = graph.decode(capture([(3, 1, [1, 2]), (2, 2, [0]), (1, 2, [0]), (0, 3, [])]))
        stats = graph.structure(parsed)
        self.assertEqual(stats["longest_dependency_chain_nodes"], 3)
        self.assertEqual(stats["frontier_widths"], [1, 2, 1])
        self.assertIsNone(stats["execution_span_ns"])
        self.assertIn("invalidation", stats["scope"])


if __name__ == "__main__":
    import unittest.mock
    unittest.main()
