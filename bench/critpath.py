#!/usr/bin/env python3
"""Work, span and critical path of one rustc unit, from -Zself-profile.

    bench/critpath.py <profile.mm_profdata | chrome_profiler.json> [--top 25] [--json out.json]

Input: a measureme profile (converted with `crox`, which must be on PATH) or
crox's Chrome-trace JSON. Each event has a thread id, a start, a duration
and a label (a query, a pass such as `expand_crate`, or a generic activity),
and events nest on a thread.

What it computes, and why:

- Exclusive time per event: its duration minus its direct children on the
  same thread. Summed over all threads this is W, the total work. On one
  thread nothing can overlap, so W is the 1-core time.
- Span S: the wall clock from the first to the last event of the unit.
  With N threads available the unit can never take less than S, so W/S is
  the parallelism it achieved and the best speedup at that structure.
- Concurrency timeline: for each instant, how many threads are inside some
  event. Time with exactly one busy thread is serial time; the critical
  path runs through it, so it is where the next speedup must come from.
- Serial attribution: the serial seconds broken down by the top-level pass
  and by the innermost event (query) that was running on the single busy
  thread. That table is the map: each row is a stretch of the critical
  path, with the label that owns it.
- query-blocked events (if captured with -Zself-profile-events=...,
  query-blocked) count as waiting, not work.

The classification column (inherently serial / serial by implementation /
removable) is decided by a human per row; `KNOWN` below holds the labels we
have already classified, with the reason, so reruns keep the reasoning.
"""
import argparse, collections, json, os, subprocess, sys, tempfile

# label -> (class, why). Classes: "data" = real data dependency, "impl" =
# serial only because of how rustc runs it, "offpath" = can leave the critical
# path (background or cached). Unlisted labels print "?".
KNOWN = {
    "expand_crate": ("impl", "macro expansion loop; proc-macro calls are independent of each other until names resolve"),
    "expand_proc_macro": ("impl", "each proc-macro invocation is a pure token-stream function; cacheable and parallelizable"),
    "macro_expand_crate": ("impl", "parent of expansion"),
    "resolver_for_lowering_raw": ("data", "name resolution needs the fully expanded crate"),
    "late_resolve_crate": ("impl", "per-module late resolution is mostly independent"),
    "incr_comp_persist_dep_graph": ("offpath", "can run in the background after codegen (jr-rustc overlap-persist)"),
    "incr_comp_persist_result_cache": ("offpath", "same as above"),
    "incr_comp_serialize_result_cache": ("offpath", "same as above"),
    "serialize_dep_graph": ("offpath", "same as above"),
    "drop_ast": ("offpath", "freeing memory; can run on a helper thread"),
    "free_global_ctxt": ("offpath", "freeing memory at exit; the process can just exit"),
    "link": ("data", "needs every codegen unit; linker threads help"),
    "run_linker": ("data", "external linker"),
    "monomorphization_collector_graph_walk": ("impl", "graph walk, parallel in jr-rustc par-upstream-mono"),
    "partition_and_assert_distinct_symbols": ("impl", "partitioning is per-item and parallelizable"),
    "codegen_crate": ("impl", "main thread schedules CGUs; LLVM work runs on workers"),
    "metadata_register_crate": ("impl", "loading upstream metadata per crate is independent"),
    "incr_comp_load_dep_graph": ("offpath", "can be prefetched before parsing"),
    "incr_comp_prepare_load_dep_graph": ("offpath", "same"),
    "parse_crate": ("data", "parsing the root file; small"),
    "configure_and_expand": ("impl", "contains expansion"),
    "type_check_crate": ("impl", "parallel with -Zthreads per body"),
    "MIR_borrow_checking": ("impl", "parallel with -Zthreads per body"),
    "misc_checking_3": ("impl", "lints run per module"),
}


def bucket_of(label):
    """Phase of the build a query/activity label belongs to (prefix match,
    first rule wins), with its class. Used to sum the serial path per phase."""
    for name, cls, prefixes in BUCKETS:
        if label.startswith(prefixes):
            return name, cls
    return "other", "?"


# (phase, class, label prefixes). Order matters: first match wins.
BUCKETS = [
    ("lints + dead code", "impl", ("live_symbols", "lint", "early_lint", "misc_checking", "check_mod_", "privacy", "unused", "dead_code")),
    ("macro expansion", "impl", ("expand_", "macro_expand", "configure_and_expand", "resolve_macro", "compute_macro", "proc_macro")),
    ("incr persist", "offpath", ("encode_query_results", "incr_comp_query_cache_promotion", "incr_comp_persist", "incr_comp_serialize", "serialize_dep_graph", "incr_comp_encode", "incr_comp_finalize", "incr_comp_save", "drop_dep_graph")),
    ("incr load", "offpath", ("incr_comp_load", "incr_comp_prepare", "incr_comp_garbage")),
    ("mono collect + partition", "impl", ("monomorphization", "cgu_", "partition", "assert_symbols", "upstream_mono", "exported_generic", "exported_symbols", "items_of_instance", "collect_and_partition", "reachable", "symbol_name", "metadata_decode_entry_exported")),
    ("codegen main thread", "impl", ("codegen", "find_cgu_reuse", "LLVM", "llvm")),
    ("link", "data", ("link", "run_linker")),
    ("resolve + HIR lowering", "impl", ("late_resolve", "resolver", "resolve_", "hir_", "lower_to_hir", "effective_visibilities", "maybe_building_test_harness", "index_ast", "AST_validation", "get_lang_items", "complete_gated_feature", "visible_parent_map", "debugger_visualizers", "module_children", "opt_hir", "local_def")),
    ("typeck + borrowck", "impl", ("typeck", "type_check", "MIR_borrow", "mir_borrowck", "check_", "thir", "mir_", "coherence", "trait_impls", "implementations_of", "adt_", "fn_sig", "predicates", "type_of", "generics", "param_env", "layout")),
    ("metadata", "impl", ("metadata_", "crate_metadata", "write_crate_metadata", "generate_crate_metadata")),
    ("parse", "data", ("parse",)),
]


def load_events(path, min_us=0):
    if path.endswith(".json"):
        data = json.load(open(path))
    else:
        prefix = path[: -len(".mm_profdata")] if path.endswith(".mm_profdata") else path
        with tempfile.TemporaryDirectory(dir=os.environ.get("JCODE_SCRATCH_DIR")) as d:
            cmd = ["crox", os.path.abspath(prefix)]
            if min_us:
                cmd[1:1] = ["--minimum-duration", str(min_us)]
            p = subprocess.run(cmd, cwd=d, capture_output=True, text=True)
            if p.returncode != 0:
                sys.exit(f"crox failed: {p.stderr.strip()[-500:]}")
            data = json.load(open(os.path.join(d, "chrome_profiler.json")))
    evs = data["traceEvents"] if isinstance(data, dict) else data
    out = []
    for e in evs:
        if e.get("ph") != "X" or "dur" not in e:
            continue
        # The profiler's own work (string table, written at the end) is not
        # compilation; it would otherwise be the largest serial row.
        if e.get("name", "").startswith("self_profile"):
            continue
        out.append({"tid": e.get("tid", 0), "ts": float(e["ts"]), "dur": float(e["dur"]),
                    "name": e.get("name", "?"), "cat": e.get("cat", "")})
    return out


def analyze(evs):
    by_tid = collections.defaultdict(list)
    for e in evs:
        by_tid[e["tid"]].append(e)
    # Nesting per thread: sort by start, longest first; a stack gives parents.
    for tid, lst in by_tid.items():
        lst.sort(key=lambda e: (e["ts"], -e["dur"]))
        stack = []
        for e in lst:
            end = e["ts"] + e["dur"]
            while stack and stack[-1]["ts"] + stack[-1]["dur"] <= e["ts"]:
                stack.pop()
            e["parent"] = stack[-1] if stack else None
            e["top"] = stack[0]["name"] if stack else e["name"]
            e["depth"] = len(stack)
            e["child"] = 0.0
            if stack:
                stack[-1]["child"] += min(e["dur"], stack[-1]["ts"] + stack[-1]["dur"] - e["ts"])
            stack.append(e)
    blocked = lambda e: "blocked" in e["name"].lower() or e["cat"] == "QueryBlocked"
    work = collections.Counter()
    for e in evs:
        if not blocked(e):
            work[e["name"]] += max(0.0, e["dur"] - e["child"])
    W = sum(work.values()) / 1e6
    t0 = min(e["ts"] for e in evs)
    t1 = max(e["ts"] + e["dur"] for e in evs)
    S = (t1 - t0) / 1e6
    # Per thread: busy intervals = top-level events (depth 0), minus blocked.
    # Innermost label at each instant comes from the deepest event covering it.
    points = []
    for tid, lst in by_tid.items():
        for e in lst:
            if blocked(e):
                continue
            points.append((e["ts"], 1, tid, e))
            points.append((e["ts"] + e["dur"], -1, tid, e))
    points.sort(key=lambda p: (p[0], p[1]))
    active = collections.defaultdict(list)  # tid -> stack of events open
    conc_hist = collections.Counter()
    serial_top = collections.Counter()
    serial_inner = collections.Counter()
    last = t0
    for ts, kind, tid, e in points:
        dt = ts - last
        if dt > 0:
            busy = [t for t, st in active.items() if st]
            conc_hist[len(busy)] += dt
            if len(busy) == 1:
                st = active[busy[0]]
                inner = max(st, key=lambda x: x["depth"])
                serial_top[inner["top"]] += dt
                serial_inner[inner["name"]] += dt
        last = ts
        if kind == 1:
            active[tid].append(e)
        else:
            if e in active[tid]:
                active[tid].remove(e)
    serial = conc_hist[1] / 1e6
    idle = conc_hist[0] / 1e6
    phases = collections.Counter()
    for k, v in serial_inner.items():
        phases[bucket_of(k)[0]] += v
    return {
        "W": W, "S": S, "parallelism": W / S if S else 0, "serial_s": serial, "idle_s": idle,
        "threads": len(by_tid),
        "concurrency_s": {k: v / 1e6 for k, v in sorted(conc_hist.items())},
        "serial_by_phase": [(k, v / 1e6) for k, v in phases.most_common()],
        "work_top": [(k, v / 1e6) for k, v in work.most_common(40)],
        "serial_by_pass": [(k, v / 1e6) for k, v in serial_top.most_common(40)],
        "serial_by_label": [(k, v / 1e6) for k, v in serial_inner.most_common(60)],
    }


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("profile")
    ap.add_argument("--top", type=int, default=25)
    ap.add_argument("--json")
    ap.add_argument("--min-us", type=int, default=50,
                    help="drop events shorter than this (crox --minimum-duration); "
                         "they still count inside their parent's exclusive time")
    a = ap.parse_args()
    r = analyze(load_events(a.profile, a.min_us))
    print(f"W (work, 1-core time)      {r['W']:7.3f} s")
    print(f"S (span, wall)             {r['S']:7.3f} s")
    print(f"W/S (achieved parallelism) {r['parallelism']:7.2f}x   over {r['threads']} threads")
    print(f"serial (1 thread busy)     {r['serial_s']:7.3f} s   ({100 * r['serial_s'] / r['S']:.0f}% of S)")
    print(f"idle (no event open)       {r['idle_s']:7.3f} s")
    print("seconds at each concurrency: " + ", ".join(f"{k}:{v:.2f}" for k, v in r["concurrency_s"].items()))
    print("\nserial time by phase (the critical path; class: impl = serial by implementation,")
    print("offpath = can leave the path, data = real dependency)")
    for k, v in r["serial_by_phase"]:
        print(f"{v:8.3f}  {dict((b[0], b[1]) for b in BUCKETS).get(k, '?'):7}  {k}")
    def table(title, rows):
        print(f"\n{title}")
        print(f"{'seconds':>8}  {'class':7}  label  (why)")
        for k, v in rows[: a.top]:
            cls, why = KNOWN.get(k, ("?", ""))
            print(f"{v:8.3f}  {cls:7}  {k}" + (f"  ({why})" if why else ""))
    table("serial time by top-level pass (the critical path, coarse)", r["serial_by_pass"])
    table("serial time by innermost label (the critical path, fine)", r["serial_by_label"])
    table("work by label (exclusive, all threads)", r["work_top"])
    if a.json:
        json.dump(r, open(a.json, "w"), indent=1)


if __name__ == "__main__":
    main()
