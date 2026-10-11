#!/usr/bin/env python3
"""Run a synthetic, usage-weighted agent workload on one or more stacks.

The engine behind `bench/versus.py run --synth SEED` (System 3) and
`bench/suite.py run --synth SEED` (one stack, regression check).
`bench/synth/gen.py` samples N points in proportion to how agents really
call cargo and turns each into a command plus concrete edits. Per point and
stack, round-robin with a rotating stack order:

  1. sources pristine; run the point's command without running tests
     (unmeasured): the state just before the agent's edit
  2. make the edits; for a `switch` point, run the previous command
     (unmeasured), the agent built these sources with something else first
  3. run the command, measured: wall, CPU, peak RSS, user instructions of
     the whole build tree
  4. restore the sources

Points are drawn in proportion to real call counts, so the plain mean over
points is the expected wait per agent call. That is the headline, with the
breakdown by command and by kind of change underneath.
"""

from __future__ import annotations

import json
import os
import re
import shutil
import statistics
import sys
import time
from pathlib import Path

HERE = Path(__file__).resolve().parent
sys.path.insert(0, str(HERE.parent))
sys.path.insert(0, str(HERE))
import gen  # noqa: E402
import suite  # noqa: E402
import versus  # noqa: E402


def _target(st: versus.Stack, snap: str) -> Path:
    return versus.WORK / "target" / snap / st.name


def _run(st: versus.Stack, plan: gen.Plan, cmd: list[str], tag: str) -> dict:
    root = gen.snapshot_root(plan.snap)
    rid = suite.new_id()
    scen = suite.Scenario(f"synth-{plan.point.i}", plan.snap, cmd, None, "", expect_fail=plan.expect_fail)
    env = versus.stack_env(st, scen, _target(st, plan.snap), rid)
    argv = [suite.J, *cmd] if st.justrust else [versus.STOCK_CARGO, *cmd]
    log = versus.WORK / "logs" / f"synth-{st.name}-{tag}-{rid}.log"
    log.parent.mkdir(parents=True, exist_ok=True)
    load, rustc = suite.foreign_load()
    r = versus.run_counted(argv, root / plan.cwd, env, log)
    out = log.read_text(errors="replace")
    r["compile_failed"] = "error[E" in out or "error: could not compile" in out
    r["tests_failed"] = sum(int(m) for m in re.findall(r"test result: \w+\. \d+ passed; (\d+) failed", out))
    passed = sum(int(m) for m in re.findall(r"test result: \w+\. (\d+) passed", out))
    summ = suite.JR_HOME / "runs" / rid / "summary.json"
    if st.justrust and summ.exists():  # quiet mode prints no libtest lines
        t = json.loads(summ.read_text()).get("tests", {})
        passed, r["tests_failed"] = t.get("passed", passed), t.get("failed", r["tests_failed"])
    r.update({"id": rid if st.justrust else None, "tests_passed": passed, "load_before": load,
              "foreign_rustc_before": rustc, "noisy": rustc > 0 or load > suite.NOISY_CORES, "log": str(log)})
    versus._mark_remote(st, r, out)
    if "remote" in r and r["remote"] is None and cmd[0] in ("check", "clippy", "test"):
        r["fell_back_local"] = True
    return r


def _keep_log(r: dict) -> None:
    Path(r.pop("log")).unlink(missing_ok=True)


def run_point(st: versus.Stack, plan: gen.Plan, n: int) -> dict:
    """Steps 1-4 for one stack. Returns the measured sample or an error."""
    gen.restore_all(plan.snap)
    pre = _run(st, plan, plan.pre, "pre")
    if pre["exit"] != 0:
        return {"error": f"pre-build failed (exit {pre['exit']}), log {pre['log']}"}
    _keep_log(pre)
    created = gen.apply(plan, n)
    try:
        if plan.prev:
            prev = _run(st, plan, plan.prev, "prev")
            _keep_log(prev)
        r = _run(st, plan, plan.cmd, "run")
    finally:
        gen.restore(plan, created)
    if r.get("fell_back_local"):
        return {"error": f"forced remote fell back to a local build, log {r['log']}", "wall": r["wall"]}
    if r["compile_failed"] != plan.expect_fail or (r["exit"] != 0 and not r["compile_failed"] and not r["tests_failed"]):
        return {"error": f"exit {r['exit']}, expected {'a compile error' if plan.expect_fail else 'success'}, "
                         f"log {r['log']}", "wall": r["wall"]}
    _keep_log(r)
    r["pre_wall"] = pre["wall"]
    return r


def run(stacks: list[versus.Stack], seed: int, points: int, reps: int, quiet_wait: float) -> dict:
    pts, info = gen.sample(seed, points)
    plans = [gen.plan(p, seed) for p in pts]
    for snap in {p.snap for p in plans}:
        gen.restore_all(snap)
    base = int(time.time()) % 100000 * 100
    res_points = []
    t_start = time.monotonic()
    for k, plan in enumerate(plans):
        samples: dict[str, list[dict]] = {st.name: [] for st in stacks}
        errors: dict[str, str] = {}
        for rep in range(reps):
            order = stacks[(k + rep) % len(stacks):] + stacks[:(k + rep) % len(stacks)]
            for st in order:
                if st.name in errors:
                    continue
                if quiet_wait:
                    suite.wait_quiet(quiet_wait)
                r = run_point(st, plan, base + k * 10 + rep)
                if "error" in r:
                    errors[st.name] = r["error"]
                    print(f"  synth {k + 1}/{len(plans)} {st.name:12s} ERROR {r['error']}", file=sys.stderr)
                    continue
                samples[st.name].append(r)
                ins = f" {r['instructions'] / 1e9:6.1f}G" if r["instructions"] else ""
                print(f"  synth {k + 1}/{len(plans)} {st.name:12s} {r['wall']:7.2f}s{ins}  "
                      f"{' '.join(plan.cmd)}{' noisy' if r['noisy'] else ''}", file=sys.stderr)
        elapsed = time.monotonic() - t_start
        eta = elapsed / (k + 1) * (len(plans) - k - 1)
        print(f"JCODE_PROGRESS {json.dumps({'current': k + 1, 'total': len(plans), 'unit': 'points', 'eta_seconds': round(eta)})}",
              file=sys.stderr)
        res_points.append({**plan.to_json(), "samples": samples, "errors": errors,
                           "stats": {s: versus.stats(v) for s, v in samples.items() if v}})
    for snap in {p.snap for p in plans}:
        gen.restore_all(snap)
    return {"seed": seed, "points": points, "reps": reps, "generator": gen.fingerprint(), "sample_info": info,
            "stack_names": [s.name for s in stacks], "results": res_points}


# --- reporting --------------------------------------------------------------


def _complete(synth: dict) -> list[dict]:
    """Points every stack measured (others would bias the means)."""
    names = synth["stack_names"]
    return [p for p in synth["results"] if all(s in p["stats"] for s in names)]


def summarize(synth: dict) -> dict:
    names = synth["stack_names"]
    pts = _complete(synth)

    def agg(group: list[dict]) -> dict:
        out = {}
        for s in names:
            walls = [p["stats"][s]["median"] for p in group]
            ins = [p["stats"][s]["instructions"] for p in group if p["stats"][s]["instructions"]]
            out[s] = {"n": len(walls), "mean_wall": round(statistics.fmean(walls), 3) if walls else None,
                      "total_wall": round(sum(walls), 2),
                      "instructions": sum(ins) if len(ins) == len(group) else None}
        return out

    by = {}
    for axis in ("command", "change"):
        groups: dict[str, list[dict]] = {}
        for p in pts:
            groups.setdefault(p["shape"][axis], []).append(p)
        by[axis] = {k: agg(v) for k, v in sorted(groups.items(), key=lambda kv: -len(kv[1]))}
    return {"points_measured": len(pts), "points": synth["points"], "overall": agg(pts), "by": by}


def table(synth: dict) -> str:
    names = synth["stack_names"]
    base = names[0]
    sm = summarize(synth)
    info = synth["sample_info"]
    lines = [f"Synthetic agent workload: seed {synth['seed']}, {sm['points_measured']}/{sm['points']} points "
             f"measured on every stack, sampled from {info['calls']} real calls "
             f"({100 * info['coverage']:.0f}% mappable to the snapshots). "
             f"Mean wall per point = expected wait per agent call.", ""]

    def row(label: str, a: dict, n: int) -> str:
        cells = [label, str(n)]
        cells += [f"{a[s]['mean_wall']:.2f}s" if a[s]["mean_wall"] is not None else "-" for s in names]
        for s in names[1:]:
            x, y = a[base]["mean_wall"], a[s]["mean_wall"]
            cells.append(f"{x / y:.2f}x" if x and y else "-")
        for s in names[1:]:
            x, y = a[base]["instructions"], a[s]["instructions"]
            cells.append(f"{y / x:.2f}" if x and y else "-")
        return "| " + " | ".join(cells) + " |"

    head = ["", "points", *[f"{s}" for s in names], *[f"{s} vs {base}" for s in names[1:]],
            *[f"instr {s}/{base}" for s in names[1:]]]
    lines += ["| " + " | ".join(head) + " |", "|---|" + "---:|" * (len(head) - 1)]
    lines.append(row("**expected wait per call**", sm["overall"], sm["points_measured"]))
    for axis, groups in sm["by"].items():
        for k, a in groups.items():
            lines.append(row(f"{axis}: {k}", a, a[base]["n"]))
    return "\n".join(lines)


def detail(synth: dict) -> str:
    names = synth["stack_names"]
    out = ["| # | command | change | edits | " + " | ".join(names) + " |",
           "|---:|---|---|---|" + "---:|" * len(names)]
    for p in synth["results"]:
        cells = [str(p["i"]), "`" + " ".join(p["cmd"]) + "`", p["shape"]["change"], str(len(p["edits"]))]
        for s in names:
            st = p["stats"].get(s)
            cells.append(f"{st['median']:.2f}s" if st else "ERR")
        out.append("| " + " | ".join(cells) + " |")
    return "\n".join(out)


def compare(a: dict, b: dict) -> str:
    """Same seed and generator: per stack, mean wall and instructions A -> B."""
    if (a["seed"], a["points"], a["generator"]) != (b["seed"], b["points"], b["generator"]):
        print("WARNING: different seed, size or generator: not the same work", file=sys.stderr)
    ok = {p["i"] for p in _complete(a)} & {p["i"] for p in _complete(b)}
    out = [f"{'stack':14s} {'A mean':>8s} {'B mean':>8s} {'%':>6s} {'instr %':>8s}  points {len(ok)}"]
    for s in a["stack_names"]:
        if s not in b["stack_names"]:
            continue
        pa = [p for p in a["results"] if p["i"] in ok]
        pb = [p for p in b["results"] if p["i"] in ok]
        wa = statistics.fmean(p["stats"][s]["median"] for p in pa)
        wb = statistics.fmean(p["stats"][s]["median"] for p in pb)
        ia = [p["stats"][s]["instructions"] for p in pa]
        ib = [p["stats"][s]["instructions"] for p in pb]
        ipct = f"{100 * (sum(ib) - sum(ia)) / sum(ia):+7.1f}%" if all(ia) and all(ib) else "       -"
        out.append(f"{s:14s} {wa:8.2f} {wb:8.2f} {100 * (wb - wa) / wa:+5.0f}% {ipct}")
    return "\n".join(out)
