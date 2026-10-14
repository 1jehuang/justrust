#!/usr/bin/env python3
"""Decode the exact stock 2026-10-08 incremental query graph outside rustc.

Edges are consumer -> dependency. This is an invalidation/read graph, not a
complete execution DAG. Query-kind DOT output is deliberately not accepted.
Wire constants are pinned to rustc 1d81eb4ad9cd207e3e638bd32b17ec4fce8412a6.
"""
import argparse
from collections import Counter, deque
import hashlib
import json
import os
from pathlib import Path
import struct

from scheduler_costs import open_safe

VERSION = b"1.101.0-nightly (1d81eb4ad 2026-10-07)"
COMPILER_COMMIT = "1d81eb4ad9cd207e3e638bd32b17ec4fce8412a6"
KIND_COUNT = 337
MAX_BYTES = 256 * 1024 * 1024
MAX_NODES = 2_000_000
MAX_NODE_SPACE = 4_000_000
MAX_EDGES = 20_000_000
LARGE_MAX_NODES = 4_000_000
LARGE_MAX_EDGES = 32_000_000
TRAILER = b"rust-end-file"


class Reader:
    def __init__(self, data):
        self.data = data
        self.pos = 0

    def take(self, size):
        if size < 0 or self.pos + size > len(self.data):
            raise ValueError("truncated graph")
        value = self.data[self.pos:self.pos + size]
        self.pos += size
        return value

    def leb(self, bits):
        value = 0
        for shift in range(0, bits, 7):
            byte = self.take(1)[0]
            value |= (byte & 127) << shift
            if value >= 1 << bits:
                raise ValueError("LEB integer overflow")
            if byte < 128:
                if shift and byte == 0:
                    raise ValueError("noncanonical LEB integer")
                return value
        raise ValueError("unterminated LEB integer")


def decode(raw, large_capture=False):
    if type(large_capture) is not bool:
        raise ValueError("large_capture requires an explicit boolean")
    max_nodes = LARGE_MAX_NODES if large_capture else MAX_NODES
    max_edges = LARGE_MAX_EDGES if large_capture else MAX_EDGES
    if not isinstance(raw, bytes) or len(raw) > MAX_BYTES:
        raise ValueError("graph exceeds bounded byte input")
    if not raw.endswith(TRAILER) or len(raw) < len(TRAILER) + 24:
        raise ValueError("missing compiler file trailer")
    payload = raw[:-len(TRAILER)]
    node_max, node_count, edge_count = struct.unpack("<QQQ", payload[-24:])
    if not 0 < node_count <= max_nodes or not node_count <= node_max <= MAX_NODE_SPACE:
        raise ValueError("invalid bounded graph node counts")
    if edge_count > max_edges:
        raise ValueError("edge count exceeds bound")
    reader = Reader(payload[:-24])
    if reader.take(6) != b"RSIC\0\0":
        raise ValueError("unsupported incremental graph header")
    version = reader.take(reader.take(1)[0])
    if version != VERSION:
        raise ValueError("graph compiler revision is not the pinned stock version")
    command_hash = reader.take(8).hex()
    if node_count * 38 > len(reader.data) - reader.pos:
        raise ValueError("node count exceeds available bytes")
    nodes = {}
    counts = [0] * KIND_COUNT
    actual_edges = 0
    for _ in range(node_count):
        header = reader.take(38)
        head, index = struct.unpack("<HI", header[:6])
        kind = head & 511
        width = ((head >> 9) & 3) + 1
        length = head >> 11
        length = length - 1 if length else reader.leb(32)
        if not 0 < kind < KIND_COUNT or index >= node_max or index in nodes:
            raise ValueError("invalid kind, duplicate or outside node index")
        if actual_edges + length > edge_count or length * width > len(reader.data) - reader.pos:
            raise ValueError("edge length exceeds footer or remaining bytes")
        edge_data = reader.take(length * width)
        dependencies = [int.from_bytes(edge_data[i:i + width], "little")
                        for i in range(0, len(edge_data), width)]
        if index in dependencies or len(set(dependencies)) != len(dependencies):
            raise ValueError("self or duplicate dependency edge")
        nodes[index] = {"kind": kind, "key_fingerprint": header[6:22].hex(),
                        "value_fingerprint": header[22:].hex(), "dependencies": dependencies}
        counts[kind] += 1
        actual_edges += length
    declared_counts = [reader.leb(32) for _ in range(KIND_COUNT)]
    session_count = reader.leb(64)
    if reader.pos != len(reader.data) or actual_edges != edge_count or declared_counts != counts:
        raise ValueError("graph counts or trailing bytes mismatch")
    if any(dep not in nodes for node in nodes.values() for dep in node["dependencies"]):
        raise ValueError("dependency references unencoded index hole")
    return {"compiler_commit": COMPILER_COMMIT, "version": version.decode(),
            "sha256": hashlib.sha256(raw).hexdigest(), "command_hash_le": command_hash,
            "session_count": session_count, "index_space": node_max, "node_count": node_count,
            "edge_count": edge_count, "holes": node_max - node_count,
            "kind_counts": counts, "nodes": nodes}


def structure(graph):
    nodes = graph["nodes"]
    remaining = {index: len(node["dependencies"]) for index, node in nodes.items()}
    consumers = {index: [] for index in nodes}
    for index, node in nodes.items():
        for dep in node["dependencies"]:
            consumers[dep].append(index)
    ready = deque(sorted(index for index, size in remaining.items() if size == 0))
    levels = {}
    while ready:
        index = ready.popleft()
        levels[index] = 1 + max((levels[dep] for dep in nodes[index]["dependencies"]), default=0)
        for consumer in consumers[index]:
            remaining[consumer] -= 1
            if remaining[consumer] == 0:
                ready.append(consumer)
    if len(levels) != len(nodes):
        raise ValueError("cycle in invocation dependency graph")
    widths = Counter(levels.values())
    return {"scope": "same-session incremental invalidation/read dependency graph",
            "edge_orientation": "consumer -> dependency",
            "longest_dependency_chain_nodes": max(levels.values()),
            "frontier_widths": [widths[level] for level in range(1, max(levels.values()) + 1)],
            "execution_span_ns": None, "cpu_work_ns": None,
            "limitations": ["No durations are attached by structural decoding.",
                            "Eval-always and ignored reads, query continuations, locks, scheduling and subprocesses are not a complete execution DAG.",
                            "Frontiers show graph structure, not measured runnable parallelism or an optimization ceiling."]}


def read(path):
    with os.fdopen(open_safe(path, os.O_RDONLY), "rb") as handle:
        if os.fstat(handle.fileno()).st_size > MAX_BYTES:
            raise ValueError("graph exceeds byte bound")
        raw = handle.read(MAX_BYTES + 1)
    if len(raw) > MAX_BYTES:
        raise ValueError("graph exceeds byte bound")
    return raw


def main(argv=None):
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("graph", type=Path)
    parser.add_argument("--output", type=Path, help="exclusive new full graph JSON")
    parser.add_argument("--large-capture", action="store_true", help="opt in to 4M nodes/32M edges; byte and index bounds unchanged")
    args = parser.parse_args(argv)
    try:
        parsed = decode(read(args.graph), large_capture=args.large_capture)
        stats = structure(parsed)
        if args.output:
            with os.fdopen(open_safe(args.output, os.O_WRONLY | os.O_CREAT | os.O_EXCL), "w") as output:
                json.dump({"graph": parsed, "structure": stats}, output, indent=2, allow_nan=False)
                output.write("\n")
        print(json.dumps({**{key: value for key, value in parsed.items() if key not in ("nodes", "kind_counts")},
                          "structure": stats}, allow_nan=False))
    except (ValueError, OSError, UnicodeError) as error:
        parser.exit(2, "query graph: " + (str(error) if isinstance(error, ValueError)
                                         else "invalid or unavailable bounded input/output") + "\n")


if __name__ == "__main__":
    main()
