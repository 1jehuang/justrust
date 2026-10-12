#!/usr/bin/env python3
"""Aggregate every recorded justrust run: where agent wall time went."""
import json, os, sys, collections, glob

RUNS = os.path.expanduser("~/.justrust/runs")
out = []
for rid in sorted(os.listdir(RUNS)):
    p = os.path.join(RUNS, rid, "summary.json")
    try:
        s = json.load(open(p))
    except Exception:
        continue
    cwd = s.get("cwd") or ""
    bench = "/.justrust/bench/" in cwd or "/scratch/" in cwd or "/tmp/" in cwd
    ph = s.get("phases") or {}
    units = s.get("units") or {}
    tests = s.get("tests") or {}
    diag = s.get("diagnostics") or {}
    split = collections.Counter()
    for u in s.get("top_units") or []:
        for k, v in (u.get("split") or {}).items():
            split[k] += v or 0
    # pass level for codegen breakdown
    passes = collections.Counter()
    up = os.path.join(RUNS, rid, "units.jsonl")
    nunits_local = 0
    dep_wall = 0.0
    if os.path.exists(up):
        try:
            for line in open(up):
                u = json.loads(line)
                if not u.get("local"):
                    dep_wall += u.get("wall") or 0
                for pp in u.get("passes") or []:
                    passes[pp["name"]] += pp["secs"]
        except Exception:
            pass
    out.append(dict(
        id=rid, bench=bench, sub=s.get("subcommand"), wall=s.get("wall") or 0,
        exit=s.get("exit"), proj=os.path.basename(cwd.rstrip("/")),
        startup=ph.get("startup") or 0, lock=ph.get("lock_wait") or 0,
        compile=ph.get("compile") or 0, gaps=ph.get("build_gaps") or 0,
        test_run=ph.get("test_run") or 0, tail=ph.get("tail") or 0,
        compiled=units.get("compiled") or 0, deps=units.get("dependencies") or 0,
        local=units.get("local") or 0, bs=units.get("build_scripts") or 0,
        rustc_wall_sum=units.get("rustc_wall_sum") or 0,
        dep_wall=dep_wall,
        split=dict(split), passes=dict(passes),
        tpassed=tests.get("passed") or 0, tfiltered=tests.get("filtered_out") or 0,
        compile_failed=diag.get("compile_failed"),
        errors=diag.get("compile_errors") or 0,
        toolchain=s.get("toolchain"), start=s.get("start"),
    ))
json.dump(out, open(os.path.expanduser("~/.justrust/runlog.json"), "w"))
print(len(out), "runs")
