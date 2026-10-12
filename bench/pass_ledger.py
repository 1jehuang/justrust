#!/usr/bin/env python3
"""Where the serial wall goes inside rustc: a per-pass ledger over every
recorded unit with `-Ztime-passes` data (`~/.justrust/runs/*/units.jsonl`).

-Ztime-passes timers nest and only time the main thread, so each unit's
passes are mapped onto exclusive buckets first (see `buckets()`):

- The mono collector and partitioning run inside whichever of
  `generate_crate_metadata` (with -Zthreads it prefetches exported
  symbols, which collects mono items) or `codegen_crate` closes first after
  them, and are subtracted from that one.
- LLVM optimization runs on worker threads during `codegen_crate` and
  `serialize_dep_graph`. Only the main thread's wait for it
  (`finish_ongoing_codegen`) is on the critical path.
- `rustc outside total` = unit wall minus rustc's `total`: process
  startup, loading the driver, freeing memory at exit.

Each unit's buckets are weighted by how much of the run's wall it owned:
`share` = integral over the unit of 1/k dt (k = units running), `solo` =
time it was the only unit running. Sum of `share` over all units equals
the run's busy compile wall, so the share column is "seconds of agent wall
per bucket" if each concurrent unit counted equally.

Usage: bench/pass_ledger.py [--since YYYYMMDD] [--filter agent|bench|remote|all]
                            [--crate NAME] [--json]
"""
import argparse, collections, glob, json, os, sys

RUNS = os.path.expanduser("~/.justrust/runs")

FRONT = [
    ("parse", ["parse_crate", "crate_injection"]),
    ("macro expansion", ["macro_expand_crate"]),
    ("test harness, AST validation", ["maybe_building_test_harness", "AST_validation",
                                      "maybe_create_a_macro_crate", "complete_gated_feature_checking"]),
    ("name resolution", ["resolve_crate"]),
    ("drop AST", ["drop_ast"]),
    ("write dep-info", ["write_dep_info"]),
    ("misc checking 1", ["misc_checking_1", "emit_ast_lowering_delayed_lints"]),
    ("coherence", ["coherence_checking"]),
    ("type check", ["type_check_crate"]),
    ("borrow check", ["MIR_borrow_checking"]),
    ("lints, privacy (misc_checking_3)", ["misc_checking_3"]),
    ("incr load (prepare, gc)", ["incr_comp_prepare_session_directory",
                                 "incr_comp_garbage_collect_session_directories"]),
]
MONO = ["monomorphization_collector_root_collections",
        "monomorphization_collector_graph_walk"]
PART = ["partition_and_assert_distinct_symbols"]


def buckets(passes, wall):
    P = collections.Counter()
    idx = {}
    for i, p in enumerate(passes):
        P[p["name"]] += p["secs"] or 0
        idx.setdefault(p["name"], i)
    total = P["total"]
    if total <= 0:
        return None
    out = collections.Counter()
    for b, names in FRONT:
        out[b] += sum(P[n] for n in names)
    mono = sum(P[n] for n in MONO)
    part = sum(P[n] for n in PART)
    out["mono collector"] += mono
    out["partition_and_assert_distinct_symbols"] += part
    # Which timer encloses mono+partition?
    mi = idx.get("monomorphization_collector_graph_walk")
    meta = P["generate_crate_metadata"] + P["generate_crate_metadata_stub"]
    cg = P["codegen_crate"]
    if mi is not None:
        gi = idx.get("generate_crate_metadata", 10**9)
        ci = idx.get("codegen_crate", 10**9)
        if gi > mi and gi < ci:
            meta -= mono + part
        else:
            cg -= mono + part
    out["metadata encode"] += max(meta, 0)
    out["codegen to LLVM IR (main thread)"] += max(cg, 0)
    out["wait for LLVM (finish_ongoing_codegen, excl. copy)"] += max(
        P["finish_ongoing_codegen"] - P["copy_all_cgu_workproducts_to_incr_comp_cache_dir"], 0)
    out["incr persist (serialize_dep_graph)"] += P["serialize_dep_graph"]
    out["incr workproducts, finalize"] += (P["copy_all_cgu_workproducts_to_incr_comp_cache_dir"]
                                          + P["serialize_work_products"]
                                          + P["incr_comp_finalize_session_directory"])
    out["linker (run_linker)"] += P["run_linker"]
    out["rlib archive (link_rlib)"] += P["link_rlib"]
    out["link other (remove temps, ...)"] += max(
        P["link"] - P["run_linker"] - P["link_rlib"] - P["serialize_work_products"]
        - P["incr_comp_finalize_session_directory"], 0)
    known = sum(out.values())
    out["other rustc (unattributed)"] += max(total - known, 0)
    out["rustc outside total (startup/exit)"] += max(wall - total, 0)
    return out


def weights(units):
    """share and solo seconds per unit (index), from start/end intervals."""
    ev = []
    for i, u in enumerate(units):
        s, e = u.get("start"), u.get("end")
        if s is None or e is None or e <= s:
            continue
        ev.append((s, 1, i))
        ev.append((e, -1, i))
    ev.sort(key=lambda x: (x[0], x[1]))
    share = collections.Counter()
    solo = collections.Counter()
    active = set()
    last = None
    for t, d, i in ev:
        if last is not None and active and t > last:
            dt = t - last
            k = len(active)
            for j in active:
                share[j] += dt / k
                if k == 1:
                    solo[j] += dt
        if d == 1:
            active.add(i)
        else:
            active.discard(i)
        last = t
    return share, solo


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--since", default="")
    ap.add_argument("--filter", default="all", choices=["all", "agent", "bench", "remote", "local"])
    ap.add_argument("--crate", default="")
    ap.add_argument("--args", default="", help="substring of the joined run args")
    ap.add_argument("--rustc", default="", help="substring of the run's real rustc path (e.g. jr- for the fork)")
    ap.add_argument("--json", action="store_true")
    a = ap.parse_args()

    raw = collections.Counter()      # plain seconds
    shw = collections.Counter()      # share-weighted
    solw = collections.Counter()     # solo-weighted
    nounit = collections.Counter()   # units without passes (deps etc.)
    phases = collections.Counter()
    nruns = nunits = 0
    run_wall = 0.0
    for rd in sorted(os.listdir(RUNS)):
        if rd < a.since:
            continue
        up = os.path.join(RUNS, rd, "units.jsonl")
        sp = os.path.join(RUNS, rd, "summary.json")
        if not os.path.exists(up):
            continue
        try:
            s = json.load(open(sp))
        except Exception:
            s = {}
        cwd = s.get("cwd") or ""
        bench = "/.justrust/bench/" in cwd
        remote = bool(s.get("remote"))
        if a.filter == "agent" and (bench or not s.get("agent_session")):
            continue
        if a.filter == "bench" and not bench:
            continue
        if a.filter == "remote" and not remote:
            continue
        if a.filter == "local" and remote:
            continue
        if a.args and a.args not in " ".join(s.get("args") or []):
            continue
        if a.rustc:
            try:
                m = json.load(open(os.path.join(RUNS, rd, "meta.json")))
            except Exception:
                m = {}
            if a.rustc not in (m.get("real_rustc") or "") + " " + (m.get("env") or {}).get("RUSTC", ""):
                continue
        units = []
        for line in open(up):
            try:
                units.append(json.loads(line))
            except Exception:
                pass
        if not units:
            continue
        share, solo = weights(units)
        has = False
        for i, u in enumerate(units):
            w = u.get("wall") or 0
            if w <= 0:
                continue
            if a.crate and u.get("crate_name") != a.crate:
                continue
            b = buckets(u.get("passes") or [], w) if u.get("passes") else None
            if b is None:
                kind = ("depcache/localcache restore" if u.get("depcache") in ("hit", "local", "remote-hit")
                        else "local crate, no passes" if u.get("local") else "dependency crate (no passes)")
                raw[kind] += w
                shw[kind] += share[i]
                solw[kind] += solo[i]
                continue
            has = True
            nunits += 1
            fs, fo = share[i] / w, solo[i] / w
            for k, v in b.items():
                raw[k] += v
                shw[k] += v * fs
                solw[k] += v * fo
        if not has and a.crate:
            continue
        nruns += 1
        run_wall += s.get("wall") or 0
        for k, v in (s.get("phases") or {}).items():
            if k != "compile":
                phases[k] += v or 0
        rem = s.get("remote") or {}
        if rem.get("sync_ms"):
            phases["remote sync"] += rem["sync_ms"] / 1000
    rows = []
    for k in set(raw) | set(shw):
        rows.append((k, raw[k], shw[k], solw[k]))
    rows.sort(key=lambda r: -r[2])
    if a.json:
        json.dump(dict(runs=nruns, units=nunits, run_wall=run_wall,
                       rows=rows, phases=dict(phases)), sys.stdout, indent=1)
        return
    tot_sh = sum(r[2] for r in rows)
    print(f"runs {nruns}, units with passes {nunits}, run wall {run_wall:.0f} s, "
          f"compile share total {tot_sh:.0f} s")
    print(f"{'bucket':52} {'raw s':>9} {'share s':>9} {'%':>5} {'solo s':>9} {'per run':>8}")
    for k, r, sh, so in rows:
        print(f"{k:52} {r:9.1f} {sh:9.1f} {100*sh/max(tot_sh,1e-9):5.1f} {so:9.1f} {sh/max(nruns,1):8.3f}")
    print("non-compile phases (s):")
    for k, v in phases.most_common():
        print(f"  {k:50} {v:9.1f} {v/max(nruns,1):8.3f}/run")


if __name__ == "__main__":
    main()
