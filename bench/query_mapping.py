#!/usr/bin/env python3
"""Join observed same-session query/wait IDs to actual dependency nodes.

Elapsed wall-clock attribution is not CPU work. This deliberately does not
claim execution span or a core-utilization ceiling from an invalidation graph.
The caller must establish paired-file provenance before invoking map_events.
"""
import hashlib
import json
import os
from pathlib import Path
import re
from collections import defaultdict

import query_graph
import query_profile
from scheduler_costs import open_safe


def map_events(graph, events):
    nodes = graph["nodes"]
    threads = defaultdict(list)
    kinds = defaultdict(int)
    unknown = set()
    provider_count = unresolved = unknown_records = 0
    for ordinal, event in enumerate(events):
        kinds[event["kind"]] += 1
        if event.get("record_type") != "interval" or event["kind"] not in ("Query", "QueryBlocked"):
            continue
        if any(type(event.get(key)) is not int or event[key] < 0
               for key in ("event_id", "thread", "start_ns", "end_ns")):
            raise ValueError("invalid query interval identity or timestamp")
        if event["end_ns"] < event["start_ns"]:
            raise ValueError("invalid query interval start/end")
        threads[event["thread"]].append((ordinal, event))
        if event["kind"] == "Query":
            provider_count += 1
            unresolved += not event.get("label_resolved", event.get("label") is not None)
            if event["event_id"] not in nodes:
                unknown.add(event["event_id"])
                unknown_records += 1
    observed = {}
    relations = []
    known_wait = inclusive_wait = unattributed_elapsed = 0

    def finish(entry):
        nonlocal known_wait, unattributed_elapsed
        event, exclusive = entry
        if exclusive < 0:
            raise ValueError("negative exclusive query elapsed")
        if event["kind"] == "QueryBlocked":
            known_wait += exclusive
            return
        index = event["event_id"]
        if index not in nodes:
            unattributed_elapsed += exclusive
            return
        value = observed.setdefault(index, {"kind": nodes[index]["kind"],
                                             "provider_records": 0,
                                             "inclusive_elapsed_ns": 0,
                                             "exclusive_elapsed_ns": 0})
        value["provider_records"] += 1
        value["inclusive_elapsed_ns"] += event["end_ns"] - event["start_ns"]
        value["exclusive_elapsed_ns"] += exclusive

    for records in threads.values():
        # Equal intervals use stable raw ordering, never labels or numeric-ID order.
        records.sort(key=lambda item: (item[1]["start_ns"], -item[1]["end_ns"], item[0]))
        stack = []
        for _, event in records:
            while stack and stack[-1][0]["end_ns"] <= event["start_ns"]:
                finish(stack.pop())
            duration = event["end_ns"] - event["start_ns"]
            if stack:
                if event["end_ns"] > stack[-1][0]["end_ns"]:
                    raise ValueError("crossing same-thread query intervals")
                stack[-1][1] -= duration
            if event["kind"] == "QueryBlocked":
                inclusive_wait += duration
                caller = next((parent[0]["event_id"] for parent in reversed(stack)
                               if parent[0]["kind"] == "Query"), None)
                producer = event["event_id"]
                relations.append({"producer_id": producer, "caller_id": caller,
                                  "thread": event["thread"], "start_ns": event["start_ns"],
                                  "end_ns": event["end_ns"], "elapsed_ns": duration,
                                  "producer_in_graph": producer in nodes,
                                  "caller_in_graph": caller in nodes,
                                  "recorded_read_edge_present": caller in nodes and producer in nodes[caller]["dependencies"]})
            stack.append([event, duration])
        while stack:
            finish(stack.pop())
    for relation in relations:
        relation["producer_provider_observed"] = relation["producer_id"] in observed
    return {"event_kind_counts": dict(kinds), "provider_records": provider_count,
            "provider_records_with_unresolved_label": unresolved,
            "provider_records_without_graph_node": unknown_records,
            "unknown_provider_ids": sorted(unknown), "observed_queries": observed,
            "observed_node_count": len(observed), "unobserved_node_count": len(nodes) - len(observed),
            "exclusive_query_elapsed_ns": sum(value["exclusive_elapsed_ns"] for value in observed.values()),
            "unattributed_query_elapsed_ns": unattributed_elapsed,
            "known_query_blocked_elapsed_ns": known_wait,
            "inclusive_query_blocked_elapsed_ns": inclusive_wait,
            "blocked_relations": relations, "execution_span_ns": None, "cpu_work_ns": None,
            "limitations": ["Numeric invocation IDs are valid only inside a verified same-session capture pair.",
                            "Missing provider timings remain unknown, not zero CPU-cost work.",
                            "Exclusive elapsed subtracts nested query and known blocked intervals, but still contains unreported locks and OS descheduling.",
                            "QueryBlocked identifies the waited producer. Caller is observed enclosing query context, not the blocked event ID.",
                            "QueryBlocked includes latch waiting plus post-wake cache lookup, not pure idle time. Profiler thread IDs are Rust thread IDs, not proven OS TIDs.",
                            "Dependency edges omit eval-always/ignored reads and ordered continuations. No complete execution DAG or W/S ceiling is claimed."]}


PINNED_COMPILER_SHA256 = "66877d44efc77e5d5478c756abf48574c5ce4d41a1102518b20e2ff3f3a0ba56"


def verify_pair(parsed_graph, profile_sha256, metadata, receipt):
    """Check a trusted capture manifest, not attest an untrusted supplied manifest."""
    if (receipt.get("compiler_sha256") != PINNED_COMPILER_SHA256
            or receipt.get("graph_sha256") != parsed_graph.get("sha256")
            or receipt.get("profile_sha256") != profile_sha256):
        raise ValueError("capture compiler or file hash mismatch")
    if not re.fullmatch(r"\d{8}-\d{9}-\d+", receipt.get("run_id", "")):
        raise ValueError("invalid capture run identity")
    pid = metadata.get("process_id")
    if type(pid) is not int or pid <= 0 or pid != receipt.get("process_id"):
        raise ValueError("profile process identity mismatch")
    profile = Path(receipt.get("remote_profile", ""))
    if (not profile.is_absolute() or ".." in profile.parts
            or not re.search(r"-0*" + str(pid) + r"\.mm_profdata$", profile.name)):
        raise ValueError("profile filename process identity mismatch")
    root = Path(receipt.get("incremental_root", ""))
    graph_path = Path(receipt.get("remote_graph", ""))
    if (not root.is_absolute() or not graph_path.is_absolute()
            or ".." in root.parts or ".." in graph_path.parts
            or root not in graph_path.parents or graph_path.name != "dep-graph.bin"):
        raise ValueError("graph is not inside the captured incremental directory")
    cmd = metadata.get("cmd", "")
    if not isinstance(cmd, str) or len(cmd) > 1024 * 1024:
        raise ValueError("invalid bounded profile command")
    incremental = re.findall(r"(?:^|\s)-C\s*incremental=(\S+)", cmd)
    if not incremental or incremental[-1] != str(root):
        raise ValueError("profile command incremental directory mismatch")
    crate = receipt.get("crate_name", "")
    if not re.fullmatch(r"[A-Za-z_][A-Za-z_0-9]*", crate):
        raise ValueError("invalid captured crate identity")
    if not re.search(r"(?:^|\s)--crate-name\s+" + re.escape(crate) + r"(?:\s|$)", cmd):
        raise ValueError("profile command crate identity mismatch")
    start = metadata.get("start_time")
    lower, upper = receipt.get("unit_start_ns"), receipt.get("unit_end_ns")
    if (any(type(value) is not int or value < 0 for value in (start, lower, upper))
            or not lower <= start <= upper or lower >= upper):
        raise ValueError("profile start is outside the observed compiler unit")
    return {"run_id": receipt["run_id"], "process_id": pid,
            "profile_start_ns": start, "unit_start_ns": lower, "unit_end_ns": upper,
            "compiler_sha256": PINNED_COMPILER_SHA256,
            "graph_sha256": parsed_graph["sha256"], "profile_sha256": profile_sha256,
            "profile_command_sha256": hashlib.sha256(cmd.encode()).hexdigest(),
            "same_session_checks": ["recorded file hashes", "profile filename PID", "metadata process ID",
                                    "last incremental override and graph containment", "crate name", "unit start interval"],
            "trust_scope": "trusted owner-generated capture receipt checked for mismatches, not an independent cryptographic attestation"}


def read_receipt(path):
    with os.fdopen(open_safe(path, os.O_RDONLY), "rb") as handle:
        if os.fstat(handle.fileno()).st_size > 65536:
            raise ValueError("capture receipt exceeds 64KiB")
        raw = handle.read(65537)
    if len(raw) > 65536:
        raise ValueError("capture receipt exceeds 64KiB")
    return raw


def main(argv=None):
    import argparse
    import heapq
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--graph", type=Path, required=True)
    parser.add_argument("--profile", type=Path, required=True)
    parser.add_argument("--receipt", type=Path, required=True)
    parser.add_argument("--output", type=Path, required=True)
    parser.add_argument("--allow-unmapped-event-labels", action="store_true")
    parser.add_argument("--large-capture", action="store_true", help="opt in to 4M nodes/32M edges/8M events; file bounds unchanged")
    args = parser.parse_args(argv)
    try:
        graph = query_graph.decode(query_graph.read(args.graph), large_capture=args.large_capture)
        raw = query_profile.read_input(args.profile)
        receipt_raw = read_receipt(args.receipt)
        receipt = json.loads(receipt_raw)
        if not isinstance(receipt, dict):
            raise ValueError("capture receipt must be an object")
        proof = verify_pair(graph, hashlib.sha256(raw).hexdigest(),
                            query_profile.profile_metadata(raw, large_capture=args.large_capture), receipt)
        event_stream, strings = query_profile.prepare(raw, args.allow_unmapped_event_labels, args.large_capture)
        events = query_profile.iter_prepared_records(event_stream, strings, args.allow_unmapped_event_labels)
        result = map_events(graph, events)
        observed = result.pop("observed_queries")
        blocked = result.pop("blocked_relations")
        result["blocked_records"] = len(blocked)
        result["blocked_caller_missing"] = sum(item["caller_id"] is None for item in blocked)
        result["blocked_read_edge_missing"] = sum(not item["recorded_read_edge_present"] for item in blocked)
        result["top_queries_by_exclusive_elapsed"] = [dict(node_id=index, **value) for index, value in
            heapq.nlargest(50, observed.items(), key=lambda item: item[1]["exclusive_elapsed_ns"])]
        result["top_blocked_records"] = heapq.nlargest(50, blocked, key=lambda item: item["elapsed_ns"])
        result["unknown_provider_ids"] = result["unknown_provider_ids"][:50]
        result["schema"] = 1
        result["capture"] = proof
        result["structure"] = query_graph.structure(graph)
        result["graph_counts"] = {key: graph[key] for key in ("node_count", "edge_count", "index_space", "holes")}
        result["graph_kind_counts"] = graph["kind_counts"]
        encoded = (json.dumps(result, sort_keys=True, indent=2, allow_nan=False) + "\n").encode()
        if len(encoded) > 1024 * 1024:
            raise ValueError("compact mapping output exceeds 1MiB")
        with os.fdopen(open_safe(args.output, os.O_WRONLY | os.O_CREAT | os.O_EXCL), "wb") as output:
            output.write(encoded)
        print(json.dumps({key: result[key] for key in ("graph_counts", "provider_records", "observed_node_count",
                         "unobserved_node_count", "provider_records_without_graph_node", "exclusive_query_elapsed_ns",
                         "known_query_blocked_elapsed_ns", "blocked_records", "execution_span_ns", "cpu_work_ns")}))
    except (ValueError, OSError, TypeError, KeyError) as error:
        parser.exit(2, "query mapping: " + (str(error) if isinstance(error, ValueError)
                                            and not isinstance(error, json.JSONDecodeError)
                                            else "invalid bounded capture input or exclusive output") + "\n")


if __name__ == "__main__":
    main()
