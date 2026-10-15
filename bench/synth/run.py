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
import math
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


def _run(st: versus.Stack, plan: gen.Plan, cmd: list[str], tag: str, identity=None) -> dict:
    root = gen.snapshot_root(plan.snap)
    rid = suite.new_id()
    scen = suite.Scenario(f"synth-{plan.point.i}", plan.snap, cmd, None, "", expect_fail=plan.expect_fail)
    env = versus.stack_env(st, scen, _target(st, plan.snap), rid)
    argv = [suite.J, *cmd] if st.justrust else [versus.STOCK_CARGO, *cmd]
    log = versus.WORK / "logs" / f"synth-{st.name}-{tag}-{rid}.log"
    log.parent.mkdir(parents=True, exist_ok=True)
    if identity is not None:
        identity.verify_files()
    load, rustc = suite.foreign_load()
    r = versus.run_counted(argv, root / plan.cwd, env, log)
    if identity is not None:
        if not st.justrust:
            raise ValueError("bench: identity demand cannot attest a stock stack")
        r.update(identity.verify_run(rid, tag))
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


def run_point(st: versus.Stack, plan: gen.Plan, n: int, identity=None) -> dict:
    """Steps 1-4 for one stack. Returns the measured sample or an error."""
    gen.restore_all(plan.snap)
    options = {"identity": identity} if identity is not None else {}
    pre = _run(st, plan, plan.pre, "pre", **options)
    if pre["exit"] != 0:
        return {"error": f"pre-build failed (exit {pre['exit']}), log {pre['log']}"}
    _keep_log(pre)
    created = gen.apply(plan, n)
    try:
        if plan.prev:
            prev = _run(st, plan, plan.prev, "prev", **options)
            _keep_log(prev)
        # The unmeasured steps above can leave a GB of dirty pages (a
        # depcache restore writes faster than a compile). Without a flush the
        # measured call's cargo startup paid 0.5-1.1 s of io stall in some
        # runs, on both justrust stacks (FINDINGS 41). An agent has think
        # time between calls, so writeback is done by then; same for every
        # stack.
        os.sync()
        r = _run(st, plan, plan.cmd, "run", **options)
    finally:
        gen.restore(plan, created)
    if r.get("fell_back_local"):
        return {"error": f"forced remote fell back to a local build, log {r['log']}", "wall": r["wall"]}
    if r["compile_failed"] != plan.expect_fail or (r["exit"] != 0 and not r["compile_failed"] and not r["tests_failed"]):
        return {"error": f"exit {r['exit']}, expected {'a compile error' if plan.expect_fail else 'success'}, "
                         f"log {r['log']}", "wall": r["wall"]}
    _keep_log(r)
    r["pre_wall"] = pre["wall"]
    # The pre-build leaves the target dir warm for exactly these sources, so a
    # rerun (same command, nothing edited) must be near a no-op. If it is not,
    # this stack's cache was invalidated by something outside the edit (the
    # synth2 run saw stock rebuild 70 s on a `build` rerun): flag the point
    # so the report can leave it out instead of crediting the other stacks.
    if plan.point.shape.get("change") == "rerun" and not plan.edits and r["wall"] > max(5.0, 0.5 * pre["wall"]):
        r["suspect"] = f"rerun took {r['wall']:.1f}s after a {pre['wall']:.1f}s pre-build (cache invalidated?)"
    return r


def run(stacks: list[versus.Stack], seed: int, points: int, reps: int, quiet_wait: float,
        partial: Path | None = None, only: set[int] | None = None, resume: bool = False,
        identity=None) -> dict:
    """`partial`: rewritten after every point, so a killed run keeps what it measured.
    `only`: run just these point indices (debugging one point; not a comparable result).
    `resume`: keep the points already in `partial` when it is the same seed, size,
    generator and stacks (a spot interruption killed the run), run the rest."""
    if identity is not None and resume:
        raise ValueError("bench: identity demand cannot reuse unaudited resumed samples")
    pts, info = gen.sample(seed, points)
    plans = [gen.plan(p, seed) for p in pts]
    if only:
        plans = [p for p in plans if p.point.i in only]
    for snap in {p.snap for p in plans}:
        gen.restore_all(snap)
    base = int(time.time()) % 100000 * 100
    res_points = []
    if resume and partial and partial.exists():
        old = json.loads(partial.read_text())
        if (old.get("seed"), old.get("points"), old.get("reps"), old.get("generator"), old.get("stack_names")) == \
                (seed, points, reps, gen.fingerprint(), [s.name for s in stacks]):
            res_points = old["results"]
            print(f"  synth: resuming after {len(res_points)} measured points", file=sys.stderr)
        else:
            print("  synth: partial result is from another run, starting over", file=sys.stderr)
    done = {p["i"] for p in res_points}
    t_start = time.monotonic()
    for k, plan in enumerate(plans):
        if plan.point.i in done:
            continue
        samples: dict[str, list[dict]] = {st.name: [] for st in stacks}
        errors: dict[str, str] = {}
        for rep in range(reps):
            order = stacks[(k + rep) % len(stacks):] + stacks[:(k + rep) % len(stacks)]
            for st in order:
                if st.name in errors:
                    continue
                if quiet_wait:
                    suite.wait_quiet(quiet_wait)
                options = {"identity": identity} if identity is not None else {}
                r = run_point(st, plan, base + k * 10 + rep, **options)
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
        if partial:
            partial.parent.mkdir(parents=True, exist_ok=True)
            partial.write_text(json.dumps({"seed": seed, "points": points, "reps": reps, "partial": True,
                                           "generator": gen.fingerprint(), "sample_info": info,
                                           "stack_names": [s.name for s in stacks], "results": res_points}))
    for snap in {p.snap for p in plans}:
        gen.restore_all(snap)
    return {"seed": seed, "points": points, "reps": reps, "generator": gen.fingerprint(), "sample_info": info,
            "stack_names": [s.name for s in stacks], "results": res_points}


# --- reporting --------------------------------------------------------------


def _outlier(p: dict, names: list[str]) -> str | None:
    """A stack far off every other stack on this point: almost always a
    harness artifact (its warm cache invalidated by the unmeasured setup,
    synth2 points 7 and 8: stock 70 s and 30 s where the others took 0.4 s),
    not something a stack does better or worse per call. Also flagged by
    run_point for reruns that rebuilt."""
    for s in names:
        for x in p.get("samples", {}).get(s, []):
            if x.get("suspect"):
                return f"{s}: {x['suspect']}"
    if len(names) < 3:
        return None
    if any(type(p.get("stats", {}).get(s, {}).get("median")) not in (int, float) for s in names):
        return None
    for s in names:
        others = [p["stats"][o]["median"] for o in names if o != s]
        me = p["stats"][s]["median"]
        ref = statistics.median(others)
        if me > 10 and me > 5 * max(ref, 0.5) and max(others) < 0.5 * me:
            return f"{s} {me:.1f}s vs {ref:.1f}s for the others"
    return None


def _structure(synth: dict) -> str | None:
    """Validate containers before interpreting any report provenance."""
    if not isinstance(synth, dict):
        return "synthetic result must be an object"
    info = synth.get("sample_info")
    if (not isinstance(info, dict) or type(info.get("calls")) is not int or info["calls"] <= 0
            or type(info.get("coverage")) not in (int, float)
            or not math.isfinite(info["coverage"]) or not 0 <= info["coverage"] <= 1):
        return "sample_info needs positive calls and finite coverage between zero and one"
    if not isinstance(synth.get("generator"), str) or not synth["generator"]:
        return "generator must be a nonempty fingerprint string"
    names, pts = synth.get("stack_names"), synth.get("results")
    if not isinstance(names, list) or not names or any(not isinstance(s, str) or not s for s in names):
        return "stack_names must be a nonempty list of stack identities"
    if not isinstance(pts, list) or any(not isinstance(p, dict) for p in pts):
        return "results must be a list of point objects"
    for p in pts:
        for key in ("stats", "samples", "errors"):
            if not isinstance(p.get(key), dict):
                return f"point {p.get('i')}: {key} must be an object"
        if any(not isinstance(v, dict) for v in p["stats"].values()):
            return f"point {p.get('i')}: stats entries must be objects"
        if any("instructions" not in v for v in p["stats"].values()):
            return f"point {p.get('i')}: missing instruction summary provenance (explicit null allowed)"
        if any(not isinstance(v, list) or any(not isinstance(x, dict) for x in v)
               for v in p["samples"].values()):
            return f"point {p.get('i')}: samples must contain lists of sample objects"
        if any(key not in p for key in ("cmd", "pre", "prev", "edits", "expect_fail", "snap", "cwd", "shape")):
            return f"point {p.get('i')}: missing required actual workload plan fields"
        if (not isinstance(p["shape"], dict)
                or any(not isinstance(p["shape"].get(k), str) for k in ("command", "change"))
                or not isinstance(p["cmd"], list) or any(not isinstance(v, str) for v in p["cmd"])
                or not p["cmd"]
                or not isinstance(p["pre"], list) or not p["pre"] or any(not isinstance(v, str) for v in p["pre"])
                or (p["prev"] is not None and (not isinstance(p["prev"], list) or not p["prev"]
                                              or any(not isinstance(v, str) for v in p["prev"])))
                or any(not isinstance(p[k], str) or not p[k] for k in ("snap", "cwd"))
                or not isinstance(p["edits"], list) or any(not isinstance(v, str) for v in p["edits"])
                or type(p["expect_fail"]) is not bool):
            return f"point {p.get('i')}: malformed actual workload plan"
    return None


def _blocked(synth: dict) -> list[str]:
    """Reasons observations cannot estimate the entire sampled workload."""
    reasons = []
    if why := _structure(synth):
        return [why + "; repair report provenance before comparing"]
    if type(synth.get("seed")) is not int or not synth.get("generator") or not synth.get("sample_info"):
        reasons.append("missing seed, generator or sampling identity provenance")
    names = synth.get("stack_names", [])
    count, reps = synth.get("points"), synth.get("reps")
    if not names or len(set(names)) != len(names):
        reasons.append("missing or duplicate stack identities")
    if type(count) is not int or count < 1 or type(reps) is not int or reps < 1:
        return reasons + ["positive points and required replicate count are missing"]
    if synth.get("partial"):
        reasons.append("partial run: finish the full workload")
    pts = synth.get("results", [])
    ids = [p.get("i") for p in pts]
    valid_ids = [i for i in ids if type(i) is int and 0 <= i < count]
    if len(valid_ids) != len(ids) or len(set(valid_ids)) != len(ids):
        reasons.append("malformed, duplicate or out-of-range point IDs")
    if len(ids) != count or set(valid_ids) != set(range(count)):
        reasons.append(f"incomplete coverage: {len(ids)}/{count} points; measure every point")
    for p in pts:
        tag = f"point {p.get('i')}"
        if p.get("errors"):
            reasons.append(f"{tag}: measurement errors; rerun failed stacks")
        for s in names:
            samples = p.get("samples", {}).get(s, [])
            st = p.get("stats", {}).get(s, {})
            if len(samples) != reps or type(st.get("n")) is not int or st.get("n") != reps:
                reasons.append(f"{tag} {s}: missing required sample/replicate coverage ({len(samples)}/{reps})")
            if any(x.get("noisy") is not False for x in samples):
                reasons.append(f"{tag} {s}: noisy or missing noise provenance; rerun under controlled conditions")
            for x in samples:
                expected = p["expect_fail"]
                if (type(x.get("exit")) is not int or type(x.get("compile_failed")) is not bool
                        or type(x.get("tests_failed")) is not int or x["tests_failed"] != 0
                        or x["compile_failed"] != expected
                        or x["exit"] < 0 or (x["exit"] == 0) == expected):
                    reasons.append(f"{tag} {s}: missing or inconsistent exit/compile/test outcome provenance for plan.expect_fail")
            wall = st.get("median")
            if type(wall) not in (int, float) or not math.isfinite(wall) or wall <= 0:
                reasons.append(f"{tag} {s}: missing or invalid median wall")
            walls = [x.get("wall") for x in samples]
            if any(type(w) not in (int, float) or not math.isfinite(w) or w <= 0 for w in walls):
                reasons.append(f"{tag} {s}: invalid raw sample wall")
            elif walls and wall != round(statistics.median(walls), 3):
                reasons.append(f"{tag} {s}: summary median does not match raw samples")
            ins = st.get("instructions")
            if ins is not None and (type(ins) not in (int, float) or not math.isfinite(ins) or ins < 0):
                reasons.append(f"{tag} {s}: invalid instruction summary")
            raw_ins = [x.get("instructions") for x in samples if x.get("instructions")]
            if ins is not None and (any(type(v) not in (int, float) or not math.isfinite(v) or v < 0 for v in raw_ins)
                                    or not raw_ins or ins != statistics.median(raw_ins)):
                reasons.append(f"{tag} {s}: instruction summary does not match raw samples")
        if not reasons and names and _outlier(p, names):
            reasons.append(f"{tag}: suspect harness artifact; rerun rather than dropping a workload point")
    return reasons


def _complete(synth: dict) -> list[dict]:
    """All sampled points or none. A clean subset is not comprehensive."""
    return [] if _blocked(synth) else synth["results"]


def _instructions(p: dict, name: str) -> float | None:
    """Never infer counter coverage from a median of the available subset."""
    samples = p["samples"].get(name, [])
    values = [x.get("instructions") for x in samples]
    if not values or any(type(v) not in (int, float) or not math.isfinite(v) or v <= 0 for v in values):
        return None
    summary = p["stats"][name].get("instructions")
    return summary if summary == statistics.median(values) else None


def _instruction_note(synth: dict) -> str:
    missing = [name for name in synth["stack_names"]
               if any(_instructions(p, name) is None for p in synth["results"])]
    return ("Instructions unavailable for " + ", ".join(missing)
            + ": missing or partial counter coverage; no instruction ratio claimed.") if missing else ""


def summarize(synth: dict) -> dict:
    if _structure(synth):
        return {"points_measured": 0, "points": synth.get("points") if isinstance(synth, dict) else None,
                "overall": {}, "by": {}, "blocked": _blocked(synth)}
    names = synth["stack_names"]
    pts = _complete(synth)

    def agg(group: list[dict]) -> dict:
        out = {}
        for s in names:
            walls = [p["stats"][s]["median"] for p in group]
            ins = [value for p in group if (value := _instructions(p, s)) is not None]
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
    reasons = _blocked(synth)
    if reasons:
        if _structure(synth):
            return "Expected-session report blocked:\n" + "\n".join(f"- {why}" for why in reasons)
        return (f"Synthetic agent workload: seed {synth.get('seed')}, "
                f"{len(synth.get('results', []))}/{synth.get('points')} points observed.\n"
                "Expected-session report blocked (raw observations remain in detail):\n"
                + "\n".join(f"- {why}" for why in reasons))
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
    if note := _instruction_note(synth):
        lines += ["", note]
    return "\n".join(lines)


def detail(synth: dict) -> str:
    if _structure(synth):
        return "Raw observations (malformed provenance):\n" + json.dumps(synth, indent=2)
    names = synth["stack_names"]
    out = ["| # | command | change | edits | " + " | ".join(names) + " |",
           "|---:|---|---|---|" + "---:|" * len(names)]
    for p in synth["results"]:
        cells = [str(p.get("i", "missing")), "`" + " ".join(p["cmd"]) + "`", p["shape"]["change"], str(len(p["edits"]))]
        for s in names:
            st = p["stats"].get(s)
            samples = p.get("samples", {}).get(s, [])
            noise = " (noisy/unknown)" if not samples or any(x.get("noisy") is not False for x in samples) else ""
            wall = st.get("median") if st else None
            cells.append(f"{wall:.2f}s{noise}" if type(wall) in (int, float) else f"ERR ({wall!r})")
        out.append("| " + " | ".join(cells) + " |")
    excl = [(p.get("i", "missing"), why) for p in synth["results"]
            if all(s in p["stats"] for s in names) and (why := _outlier(p, names))]
    if excl:
        out += ["", "Suspect harness artifacts (comprehensive means blocked):"]
        out += [f"- point {i}: {why}" for i, why in excl]
    return "\n".join(out)


def compare(a: dict, b: dict) -> str:
    """Same seed and generator: per stack, mean wall and instructions A -> B."""
    reasons = [f"{label}: {why}" for label, data in (("A", a), ("B", b)) for why in _blocked(data)]
    if _structure(a) or _structure(b):
        return "Synthetic comparison blocked:\n" + "\n".join(f"- {why}" for why in reasons)
    keys = ("seed", "points", "reps", "generator", "sample_info", "stack_names")
    for key in keys:
        if key not in a or key not in b or a[key] != b[key]:
            reasons.append(f"different or missing {key}: compare identical workload identities")
    if not reasons:
        def plans(data):
            return {p["i"]: {k: v for k, v in p.items() if k not in ("stats", "samples", "errors")}
                    for p in data["results"]}
        if plans(a) != plans(b):
            reasons.append("point plans differ: compare identical commands, edits and snapshots")
    if reasons:
        return "Synthetic comparison blocked:\n" + "\n".join(f"- {why}" for why in reasons)
    ok = {p["i"] for p in _complete(a)} & {p["i"] for p in _complete(b)}
    out = [f"{'stack':14s} {'A mean':>8s} {'B mean':>8s} {'%':>6s} {'instr %':>8s}  points {len(ok)}"]
    for s in a["stack_names"]:
        if s not in b["stack_names"]:
            continue
        pa = [p for p in a["results"] if p["i"] in ok]
        pb = [p for p in b["results"] if p["i"] in ok]
        wa = statistics.fmean(p["stats"][s]["median"] for p in pa)
        wb = statistics.fmean(p["stats"][s]["median"] for p in pb)
        ia = [_instructions(p, s) for p in pa]
        ib = [_instructions(p, s) for p in pb]
        ipct = f"{100 * (sum(ib) - sum(ia)) / sum(ia):+7.1f}%" if all(ia) and all(ib) else "       -"
        out.append(f"{s:14s} {wa:8.2f} {wb:8.2f} {100 * (wb - wa) / wa:+5.0f}% {ipct}")
    for label, data in (("A", a), ("B", b)):
        if note := _instruction_note(data):
            out.append(f"{label}: {note}")
    return "\n".join(out)
