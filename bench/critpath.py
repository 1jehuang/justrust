#!/usr/bin/env python3
"""Map observed instrumented thread occupancy from a rustc self-profile.

    bench/critpath.py <profile.mm_profdata | chrome_profiler.json> [--json out.json]

This is NOT a causal critical-path analyzer. Complete trace events describe
elapsed intervals, not CPU time or scheduling dependency edges. A thread with
an open event may be descheduled, in an external process, or waiting on an
unreported lock. Therefore neither the event envelope nor single-thread
occupancy proves a serial lower bound, and their ratio is not a speedup ceiling.

QueryBlocked intervals suppress their enclosing thread's reported occupancy.
Nested intervals are counted once per thread. Filtered-out events, including
short waits, cannot be reconstructed. Use unfiltered captures for accounting
checks and low-overhead, non-profiled benchmarks for performance decisions.
"""
import argparse
import collections
import json
import math
import os
import subprocess
import tempfile


# These are investigation hypotheses, not proofs of independence or purity.
# Proc macros can read files/env and have process-global state. Parallelizing
# them requires preserving ordering and side effects, not just token inputs.
KNOWN = {
    "expand_crate": "investigate expansion dependencies and ordered side effects",
    "expand_proc_macro": "investigate memoization or batching with explicit effect tracking",
    "resolver_for_lowering_raw": "investigate dependencies between expanded modules",
    "late_resolve_crate": "investigate independent module work and shared resolver state",
    "incr_comp_persist_dep_graph": "existing fork can overlap with linking when enabled",
    "incr_comp_persist_result_cache": "must preserve incremental-session publication ordering",
    "incr_comp_serialize_result_cache": "must preserve query ownership and publication ordering",
    "drop_ast": "investigate destruction lifetimes before moving work to another thread",
    "free_global_ctxt": "investigate required cleanup and outstanding worker lifetimes",
    "link": "needs codegen outputs, but can overlap unrelated persistence",
    "monomorphization_collector_graph_walk": "investigate metadata prefetch and query dependencies",
    "partition_and_assert_distinct_symbols": "investigate deterministic partition boundaries",
    "type_check_crate": "per-body parallel queries already exist, measure dependencies",
    "MIR_borrow_checking": "per-body parallel queries already exist, measure dependencies",
}

# First matching prefix wins. Classification is just phase attribution.
BUCKETS = [
    ("lints + dead code", ("live_symbols", "lint", "early_lint", "misc_checking", "check_mod_", "privacy", "unused", "dead_code")),
    ("macro expansion", ("expand_", "macro_expand", "configure_and_expand", "resolve_macro", "compute_macro", "proc_macro")),
    ("incr persist", ("encode_query_results", "incr_comp_query_cache_promotion", "incr_comp_persist", "incr_comp_serialize", "serialize_dep_graph", "incr_comp_encode", "incr_comp_finalize", "incr_comp_save", "drop_dep_graph")),
    ("incr load", ("incr_comp_load", "incr_comp_prepare", "incr_comp_garbage")),
    ("mono collect + partition", ("monomorphization", "cgu_", "partition", "assert_symbols", "upstream_mono", "exported_generic", "exported_symbols", "items_of_instance", "collect_and_partition", "reachable", "symbol_name", "metadata_decode_entry_exported")),
    ("codegen", ("codegen", "find_cgu_reuse", "LLVM", "llvm")),
    ("link", ("link", "run_linker")),
    ("resolve + HIR lowering", ("late_resolve", "resolver", "resolve_", "hir_", "lower_to_hir", "effective_visibilities", "maybe_building_test_harness", "index_ast", "AST_validation", "get_lang_items", "complete_gated_feature", "visible_parent_map", "debugger_visualizers", "module_children", "opt_hir", "local_def")),
    ("typeck + borrowck", ("typeck", "type_check", "MIR_borrow", "mir_borrowck", "check_", "thir", "mir_", "coherence", "trait_impls", "implementations_of", "adt_", "fn_sig", "predicates", "type_of", "generics", "param_env", "layout")),
    ("metadata", ("metadata_", "crate_metadata", "write_crate_metadata", "generate_crate_metadata")),
    ("parse", ("parse",)),
]


def bucket_of(label):
    return next((name for name, prefixes in BUCKETS if label.startswith(prefixes)), "other")


def load_events(path, min_us=0):
    if path.endswith(".json"):
        with open(path) as stream:
            data = json.load(stream)
    else:
        prefix = path.removesuffix(".mm_profdata")
        with tempfile.TemporaryDirectory(dir=os.environ.get("JCODE_SCRATCH_DIR")) as directory:
            cmd = ["crox", os.path.abspath(prefix)]
            if min_us:
                cmd[1:1] = ["--minimum-duration", str(min_us)]
            result = subprocess.run(cmd, cwd=directory, capture_output=True, text=True)
            if result.returncode:
                raise ValueError(f"crox failed: {result.stderr.strip()[-500:]}")
            with open(os.path.join(directory, "chrome_profiler.json")) as stream:
                data = json.load(stream)
    events = data["traceEvents"] if isinstance(data, dict) else data
    result = []
    for event in events:
        if event.get("ph") != "X" or "dur" not in event:
            continue
        if event.get("name", "").startswith("self_profile"):
            continue
        timestamp, duration = float(event["ts"]), float(event["dur"])
        if not math.isfinite(timestamp) or not math.isfinite(duration) or duration < 0:
            raise ValueError("event timestamps/durations must be finite and durations nonnegative")
        if duration >= min_us:
            result.append({"pid": event.get("pid", 0), "tid": event.get("tid", 0),
                           "ts": timestamp, "dur": duration,
                           "name": event.get("name", "?"), "cat": event.get("cat", "")})
    return result


def is_blocked(event):
    return event.get("cat") == "QueryBlocked" or "query_blocked" in event["name"].lower()


def analyze(events):
    by_thread = collections.defaultdict(list)
    for index, original in enumerate(events):
        event = dict(original, index=index)
        if not math.isfinite(event["ts"]) or not math.isfinite(event["dur"]) or event["dur"] < 0:
            raise ValueError("event timestamps/durations must be finite and durations nonnegative")
        if event["dur"] == 0:
            continue
        event["end"] = event["ts"] + event["dur"]
        if not math.isfinite(event["end"]):
            raise ValueError("event end must be finite")
        thread = (event.get("pid", 0), event["tid"])
        by_thread[thread].append(event)
    if not by_thread:
        raise ValueError("no positive-duration complete compilation events")

    points = []
    rounded_endpoints = 0
    clipped_us = 0.0
    for thread, thread_events in by_thread.items():
        stack = []
        for event in sorted(thread_events, key=lambda item: (item["ts"], -item["dur"], item["index"])):
            while stack and stack[-1]["end"] <= event["ts"]:
                stack.pop()
            if stack and event["end"] > stack[-1]["end"]:
                overflow = event["end"] - stack[-1]["end"]
                # crox rounds start/duration separately to microseconds. A
                # truly nested interval can extend one microsecond past its
                # parent after conversion. Do not tolerate larger crossings.
                if overflow > 1.000001:
                    raise ValueError(f"crossing intervals on thread {thread}: {stack[-1]['name']} / {event['name']} "
                                     f"(child exceeds parent by {overflow:.9f} us)")
                event["end"] = stack[-1]["end"]
                rounded_endpoints += 1
                clipped_us += overflow
            event["depth"] = len(stack)
            event["top"] = stack[0]["name"] if stack else event["name"]
            stack.append(event)
            points.append((event["ts"], 1, event["index"], thread, event))
            points.append((event["end"], -1, event["index"], thread, event))
    points.sort(key=lambda point: point[:3])
    active = collections.defaultdict(dict)
    occupancy_hist = collections.Counter()
    exclusive = collections.Counter()
    single_top = collections.Counter()
    single_inner = collections.Counter()
    blocked_us = 0.0
    last = points[0][0]
    for timestamp, kind, index, thread, event in points:
        elapsed = timestamp - last
        if elapsed > 0:
            occupied = []
            for open_events in active.values():
                if not open_events:
                    continue
                if any(is_blocked(item) for item in open_events.values()):
                    blocked_us += elapsed
                    continue
                inner = max(open_events.values(), key=lambda item: (item["depth"], item["index"]))
                occupied.append(inner)
                exclusive[inner["name"]] += elapsed
            occupancy_hist[len(occupied)] += elapsed
            if len(occupied) == 1:
                single_top[occupied[0]["top"]] += elapsed
                single_inner[occupied[0]["name"]] += elapsed
        last = timestamp
        if kind == 1:
            active[thread][index] = event
        else:
            del active[thread][index]
    span_s = (points[-1][0] - points[0][0]) / 1e6
    occupancy_s = sum(exclusive.values()) / 1e6
    phases = collections.Counter()
    for label, duration in single_inner.items():
        phases[bucket_of(label)] += duration
    rows = lambda counter, limit=None: [(key, value / 1e6) for key, value in counter.most_common(limit)]
    return {
        "schema_version": 2,
        "measurement": "instrumented elapsed-event occupancy, not CPU work or a causal DAG",
        "observed_event_span_s": span_s,
        "reported_nonblocked_thread_s": occupancy_s,
        "average_reported_nonblocked_threads": occupancy_s / span_s,
        "single_reported_thread_s": occupancy_hist[1] / 1e6,
        "no_reported_nonblocked_thread_s": occupancy_hist[0] / 1e6,
        "reported_blocked_thread_s": blocked_us / 1e6,
        "threads": len(by_thread),
        "rounded_endpoints_clipped": rounded_endpoints,
        "rounded_endpoint_clipping_us": clipped_us,
        "occupancy_histogram_s": {key: value / 1e6 for key, value in sorted(occupancy_hist.items())},
        "single_thread_by_phase": rows(phases),
        "exclusive_elapsed_by_label": rows(exclusive, 40),
        "single_thread_by_pass": rows(single_top, 40),
        "single_thread_by_label": rows(single_inner, 60),
        "causal_dependency_span_s": None,
        "cpu_work_s": None,
        "theoretical_speedup_ceiling": None,
        "limitations": [
            "No scheduling dependency edges or CPU samples are present.",
            "Open events may include descheduling, external work and unreported waits.",
            "Single-thread occupancy is not proof that work is inherently serial.",
            "Profiling and event filtering can materially distort timing and omit waits.",
            "Phase labels and optimization notes are investigation hypotheses only.",
            "Nested end overshoots of at most one microsecond are clipped for crox quantization.",
        ],
    }


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("profile")
    parser.add_argument("--top", type=int, default=25)
    parser.add_argument("--json")
    parser.add_argument("--min-us", type=int, default=0,
                        help="minimum event duration in microseconds; filtering may omit waits (default: unfiltered)")
    args = parser.parse_args()
    if not math.isfinite(args.min_us) or args.min_us < 0 or args.top < 0:
        parser.error("--min-us and --top must be nonnegative and finite")
    try:
        result = analyze(load_events(args.profile, args.min_us))
    except (ValueError, KeyError, OSError) as error:
        parser.error(str(error))
    result["minimum_event_duration_us"] = args.min_us
    print("Observed instrumented occupancy, NOT a causal critical path or speedup bound")
    print(f"Event envelope:                 {result['observed_event_span_s']:.3f} s")
    print(f"Nonblocked elapsed thread time: {result['reported_nonblocked_thread_s']:.3f} thread-s (not CPU time)")
    print(f"Average reported occupancy:     {result['average_reported_nonblocked_threads']:.2f} threads")
    print(f"One reported nonblocked thread: {result['single_reported_thread_s']:.3f} s")
    print(f"No reported nonblocked thread:  {result['no_reported_nonblocked_thread_s']:.3f} s (not necessarily idle)")
    print(f"Reported query waits:           {result['reported_blocked_thread_s']:.3f} thread-s")
    print("Theoretical speedup ceiling: unknown, needs a causal dependency graph")
    for title, key in [("Single-thread occupancy by phase", "single_thread_by_phase"),
                       ("Single-thread occupancy by pass", "single_thread_by_pass"),
                       ("Single-thread occupancy by innermost label", "single_thread_by_label"),
                       ("Exclusive elapsed occupancy by label", "exclusive_elapsed_by_label")]:
        print(f"\n{title}")
        for label, seconds in result[key][:args.top]:
            note = KNOWN.get(label)
            print(f"{seconds:8.3f}  {label}" + (f"  (hypothesis: {note})" if note else ""))
    if args.json:
        with open(args.json, "w") as stream:
            json.dump(result, stream, indent=1)
            stream.write("\n")


if __name__ == "__main__":
    main()
