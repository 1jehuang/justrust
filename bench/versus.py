#!/usr/bin/env python3
"""System 3: the stock Rust toolchain against justrust, head to head.

One question, one table: how much faster is an agent's build with justrust
than with plain `cargo` + the system `rustc`? Every workload runs on every
"stack" on the same pinned sources, round-robin, so each round sees the
identical source edit and the same machine state:

  stock      /usr/bin/cargo + system rustc, no justrust anywhere
  jr-nocache justrust with every cache off (depcache, build-script cache,
             remote): what the toolchain, forks and integration buy alone
  justrust   justrust as agents run it (its default toolchain, a private
             depcache filled by the warmup round)

`stock -> jr-nocache` is the part that does not depend on caching (forked
rustc/cargo, flags, linker, scheduling). `jr-nocache -> justrust` is what
the caches add. Try a candidate component as an extra stack:

  bench/versus.py run --stack jr-rustc:RUSTC=$HOME/jr-rustc/build/host/stage1/bin/rustc

Besides wall time, every run counts user-space CPU instructions retired by
the whole process tree (perf_event_open, inherited by every rustc, linker
and build script). Instructions barely move with machine noise (well under
1% run to run, where wall time moves 5-20%), so a compiler change that does
less work shows up even when the wall-time ranges overlap. Work done by
processes outside the build's tree (a daemon, a remote host) is not counted.

  bench/versus.py list                     workloads and stacks
  bench/versus.py run [workload...]        default: every warm workload
  bench/versus.py run --cold               plus the fresh-target-dir workloads
  bench/versus.py report [file]            rebuild bench/VERSUS.md (default: newest result)
  bench/versus.py compare A.json B.json    same stacks, two result files
  bench/versus.py clean                    delete versus target dirs and caches

Results: bench/results/versus-<time>-<rev>.json (commit them and VERSUS.md).
"""

from __future__ import annotations

import argparse
import ctypes
import json
import math
import os
import re
import shutil
import statistics
import struct
import sys
import time
from dataclasses import dataclass, field
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))
import suite  # noqa: E402

WORK = Path(os.environ.get("JUSTRUST_VERSUS_WORK", suite.JR_HOME / "bench" / "versus"))
VERSUS_MD = suite.ROOT / "bench" / "VERSUS.md"
STOCK_CARGO = os.environ.get("VERSUS_STOCK_CARGO", "/usr/bin/cargo")


@dataclass
class Stack:
    name: str
    what: str
    justrust: bool  # run through justrust (else the stock cargo directly)
    env: dict[str, str] = field(default_factory=dict)
    private_cache: bool = False


NOCACHE = {"JUSTRUST_DEPCACHE": "0", "JUSTRUST_BUILD_SCRIPT_CACHE": "0", "JUSTRUST_PEER_CACHE": "0",
           "JUSTRUST_REMOTE_CACHE": "0"}
STACKS = [
    Stack("stock", "plain cargo + system rustc, no justrust", justrust=False),
    Stack("jr-nocache", "justrust, every cache off: toolchain, forks, integration only", True, dict(NOCACHE)),
    Stack("justrust", "justrust defaults, private depcache warmed by the warmup round", True, private_cache=True),
]
# Opt-in (`--remote`): every check/clippy/test goes to the remote machine
# through the agent's own path (sync daemon, ssh, remote agent), timed from
# the agent's seat. `build` always stays local, as for agents. The machine
# keeps its own warm target dirs, at the same absolute paths.
REMOTE = Stack("remote", "justrust, JUSTRUST_REMOTE=force: check/clippy/test on the remote machine, "
                         "wall includes sync and ssh", True, {"JUSTRUST_REMOTE": "force"})


@dataclass
class Workload:
    name: str
    scenario: str  # bench/suite.py scenario (snapshot, command, edit)
    what: str
    extra_args: list[str] = field(default_factory=list)  # appended to the scenario command


WORKLOADS = [
    Workload("reference-loop", "desktop-test-edit", "edit a #[test] fn in the 118k-line UI crate, run its tests"),
    Workload("body-check", "desktop-body-check", "one-line body edit, `check` (the inner type-check loop)"),
    Workload("first-error", "desktop-type-error", "time to the first type error after an edit"),
    Workload("deep-edit", "desktop-jcode-core-body", "body edit deep in the graph (16 crates rebuild), run tests"),
    Workload("noop-test", "desktop-noop-test", "no edit: fixed cost of a `test` call"),
    # The snapshot's own tests are not all stable here (buildscript tests read
    # JUSTRUST_BUILD_SCRIPT_CACHE, which jr-nocache sets; slot GC tests race
    # under load), and one failure stops cargo before the other test
    # binaries. Every binary is still built and run, with a fixed filter.
    Workload("small-crate", "justrust-body-test", "body edit in a small crate (justrust), `test` (all binaries)",
             ["--", "summary::"]),
    Workload("cold-check", "desktop-cold-check", "fresh target dir (new checkout, worktree): `check`"),
]
BY_NAME = {w.name: w for w in WORKLOADS}


# --- instruction counting -------------------------------------------------

PERF_TYPE_HARDWARE = 0
PERF_COUNT_HW_INSTRUCTIONS = 1
SYS_PERF_EVENT_OPEN = 298  # x86_64
_libc = ctypes.CDLL(None, use_errno=True)


def _pmu_types() -> list[int]:
    """Core PMUs: a hybrid CPU (P + E cores) needs one counter per core type."""
    base = Path("/sys/bus/event_source/devices")
    hybrid = [int((base / d / "type").read_text()) for d in ("cpu_core", "cpu_atom") if (base / d / "type").exists()]
    return hybrid or [-1]


def open_instruction_counters(pid: int) -> list[int]:
    fds = []
    for pmu in _pmu_types():
        config = PERF_COUNT_HW_INSTRUCTIONS | ((pmu << 32) if pmu >= 0 else 0)
        # disabled | inherit | exclude_kernel | exclude_hv | enable_on_exec
        flags = (1 << 0) | (1 << 1) | (1 << 5) | (1 << 6) | (1 << 12)
        attr = struct.pack("<IIQQQQQ", PERF_TYPE_HARDWARE, 128, config, 0, 0, 0, flags).ljust(128, b"\0")
        buf = ctypes.create_string_buffer(attr, 128)
        fd = _libc.syscall(SYS_PERF_EVENT_OPEN, buf, pid, -1, -1, 0)
        if fd < 0:
            for f in fds:
                os.close(f)
            return []
        fds.append(fd)
    return fds


def read_counters(fds: list[int]) -> int | None:
    if not fds:
        return None
    total = 0
    for fd in fds:
        total += struct.unpack("<Q", os.read(fd, 8))[0]
        os.close(fd)
    return total


def run_counted(argv: list[str], cwd: Path, env: dict[str, str], log: Path) -> dict:
    """Run argv; wall, tree CPU, peak RSS and user instructions of the tree."""
    r, w = os.pipe()
    t0 = time.monotonic()
    pid = os.fork()
    if pid == 0:  # child: wait until the counters are attached, then exec
        try:
            os.close(w)
            os.read(r, 1)
            fd = os.open(log, os.O_WRONLY | os.O_CREAT | os.O_TRUNC, 0o644)
            os.dup2(fd, 1)
            os.dup2(fd, 2)
            os.chdir(cwd)
            os.execvpe(argv[0], argv, env)
        finally:
            os._exit(127)
    os.close(r)
    fds = open_instruction_counters(pid)
    os.write(w, b"x")
    os.close(w)
    _, status, ru = os.wait4(pid, 0)
    wall = time.monotonic() - t0
    return {
        "wall": round(wall, 3),
        "exit": os.waitstatus_to_exitcode(status),
        "cpu": round(ru.ru_utime + ru.ru_stime, 2),
        "max_rss_mb": round(ru.ru_maxrss / 1024, 1),
        "instructions": read_counters(fds),
    }


# --- running --------------------------------------------------------------


def stack_env(st: Stack, s: suite.Scenario, target: Path, rid: str) -> dict[str, str]:
    env = {k: v for k, v in os.environ.items() if not k.startswith(("JUSTRUST_", "CARGO_", "RUSTC", "RUSTFLAGS"))}
    env["CARGO_TARGET_DIR"] = str(target)
    if not st.justrust:
        # No justrust shim on PATH (~/.local/bin/cargo is justrust here).
        env["PATH"] = ":".join(p for p in env.get("PATH", "").split(":")
                               if not (Path(p) / "cargo").resolve().name.startswith("justrust"))
        env.update(s.env)
        env.update(st.env)
        return env
    env.setdefault("JUSTRUST_BUILDD", "0")  # no prebuilds between samples
    env.update({"JUSTRUST_RUN_ID_OVERRIDE": rid, "JUSTRUST_REMOTE": "0", "JUSTRUST_QUIET": "1",
                # Public store (on by default for users) off unless a stack
                # sets JUSTRUST_PREBUILT: results compare local caching.
                "JUSTRUST_PREBUILT": "off"})
    if st.private_cache:
        env["JUSTRUST_DEPCACHE_DIR"] = str(WORK / "cache" / st.name)
    env.update(s.env)
    env.update(st.env)
    return env


def target_dir(st: Stack, s: suite.Scenario, cold_tag: str | None) -> Path:
    base = WORK / "target" / s.snapshot / st.name
    return base.with_name(f"{st.name}-cold-{cold_tag}") if cold_tag else base


def run_one(st: Stack, s: suite.Scenario, root: Path, cold_tag: str | None, extra: list[str]) -> dict:
    target = target_dir(st, s, cold_tag)
    rid = suite.new_id()
    env = stack_env(st, s, target, rid)
    cmd = [*s.cmd, *extra]
    argv = [suite.J, *cmd] if st.justrust else [STOCK_CARGO, *cmd]
    log = WORK / "logs" / f"{st.name}-{rid}.log"
    log.parent.mkdir(parents=True, exist_ok=True)
    load, rustc = suite.foreign_load()
    r = run_counted(argv, root / suite.SNAPSHOTS[s.snapshot]["cwd"], env, log)
    out = log.read_text(errors="replace")
    failed = r["exit"] != 0
    compile_failed = "error[E" in out or "error: could not compile" in out
    # A failing test stops cargo before the remaining test binaries, so the
    # stacks would no longer do the same work: never compare that.
    if failed != s.expect_fail:
        sys.exit(f"versus: {s.name} on {st.name}: exit {r['exit']}, expected "
                 f"{'failure' if s.expect_fail else 'success'} (log {log})\n{out[-3000:]}")
    if s.expect_fail and not compile_failed:
        sys.exit(f"versus: {s.name} on {st.name}: failed without a compile error (log {log})\n{out[-3000:]}")
    if cold_tag:
        shutil.rmtree(target, ignore_errors=True)
    passed = sum(int(m) for m in re.findall(r"test result: \w+\. (\d+) passed", out))
    summ = suite.JR_HOME / "runs" / rid / "summary.json"
    if st.justrust and summ.exists():  # quiet mode prints no libtest lines
        passed = json.loads(summ.read_text()).get("tests", {}).get("passed", 0)
    log.unlink()  # keep only failures; justrust runs stay in `justrust log <id>`
    _mark_remote(st, r, out)
    return {**r, "id": rid if st.justrust else None, "tests_passed": passed, "noisy": rustc > 0 or load > suite.NOISY_CORES,
            "load_before": load, "foreign_rustc_before": rustc}


def _mark_remote(st: Stack, r: dict, out: str) -> None:
    """For the remote stack: did the build really run there? A local fallback
    would silently measure the laptop. Instructions counted here are only
    the client's (ssh, sync), so they are dropped."""
    if st.env.get("JUSTRUST_REMOTE") != "force":
        return
    m = re.search(r"justrust remote: (?:ok|failed) on (.+?) in ([\d.]+)s \(sync (\d+) ms", out)
    r["remote"] = {"machine": m.group(1), "sync_ms": int(m.group(3))} if m else None
    r["instructions"] = None


def run_workload(w: Workload, stacks: list[Stack], n: int, warmup: int, quiet_wait: float) -> dict:
    s = suite.BY_NAME[w.scenario]
    root = suite.prepare(s.snapshot)
    samples: dict[str, list[dict]] = {st.name: [] for st in stacks}
    base = int(time.time()) % 100000 * 100
    total = warmup + n
    try:
        for i in range(total):
            if s.edit:
                suite.apply_edit(root, s.edit, base + i)
            # Rotate the order every round so drift and thermal state hit
            # every stack equally.
            order = stacks[i % len(stacks):] + stacks[:i % len(stacks)]
            for st in order:
                if quiet_wait:
                    suite.wait_quiet(quiet_wait)
                cold = f"{os.getpid()}-{i}" if s.cold else None
                r = run_one(st, s, root, cold, w.extra_args)
                tag = " (warmup)" if i < warmup else ""
                ins = f" {r['instructions'] / 1e9:.1f}G instr" if r["instructions"] else ""
                print(f"  {w.name} {i + 1}/{total} {st.name:12s} {r['wall']:7.2f}s{ins}"
                      f"{' noisy' if r['noisy'] else ''}{tag}", file=sys.stderr)
                if i >= warmup:
                    samples[st.name].append(r)
    finally:
        suite.restore(root, s.edit)
    counts = {k: sorted({x["tests_passed"] for x in v}) for k, v in samples.items()}
    if len({tuple(c) for c in counts.values()}) > 1:
        print(f"  WARNING {w.name}: stacks ran different tests {counts}, not the same work", file=sys.stderr)
    return {"what": w.what, "scenario": w.scenario, "cmd": [*s.cmd, *w.extra_args], "cold": s.cold,
            "samples": samples, "stats": {k: stats(v) for k, v in samples.items()}}


def med(xs):
    xs = [x for x in xs if x is not None]
    return statistics.median(xs) if xs else None


def stats(samples: list[dict]) -> dict:
    walls = [x["wall"] for x in samples]
    ins = [x["instructions"] for x in samples if x["instructions"]]
    return {
        "n": len(walls),
        "median": round(med(walls), 3),
        "min": round(min(walls), 3),
        "max": round(max(walls), 3),
        "quiet_median": round(med([x["wall"] for x in samples if not x["noisy"]]) or 0, 3),
        "cpu": round(med([x["cpu"] for x in samples]), 2),
        "max_rss_mb": round(med([x["max_rss_mb"] for x in samples]), 1),
        "instructions": med(ins),
        # Run-to-run spread of instructions: (max - min) / median.
        "instructions_spread": round((max(ins) - min(ins)) / med(ins), 4) if len(ins) > 1 else None,
    }


# --- reporting ------------------------------------------------------------


def verdict(a: dict, b: dict, floor: float = 10.0) -> str:
    """b relative to a: needs disjoint ranges and a median move above floor %."""
    pct = 100 * (b["median"] - a["median"]) / a["median"] if a["median"] else 0
    if b["max"] < a["min"] and pct < -floor:
        return "faster"
    if b["min"] > a["max"] and pct > floor:
        return "slower"
    return "~"


def fmt_s(st: dict) -> str:
    return f"{st['median']:.2f}s"


def speedup(a: dict, b: dict) -> float:
    return a["median"] / b["median"] if b["median"] else float("nan")


def geomean(xs: list[float]) -> float:
    xs = [x for x in xs if x and x > 0 and not math.isnan(x)]
    return math.exp(sum(map(math.log, xs)) / len(xs)) if xs else float("nan")


def scoreboard(res: dict) -> str:
    names = res["stacks"]
    base = names[0]
    head = ["workload"] + [f"{s}" for s in names] + [f"{s} vs {base}" for s in names[1:]]
    head += [f"instr {s}/{base}" for s in names[1:]]
    rows = ["| " + " | ".join(head) + " |", "|" + "---|" + "---:|" * (len(head) - 1)]
    speed: dict[str, list[float]] = {s: [] for s in names[1:]}
    for wname, w in res["workloads"].items():
        st = w["stats"]
        cells = [wname + (" (cold)" if w["cold"] else "")]
        cells += [f"{fmt_s(st[s])} ({st[s]['min']:.1f}-{st[s]['max']:.1f})" for s in names]
        for s in names[1:]:
            x = speedup(st[base], st[s])
            speed[s].append(x)
            v = verdict(st[base], st[s])
            cells.append(f"**{x:.2f}x**" if v != "~" else f"{x:.2f}x ~")
            if v == "slower":
                cells[-1] += " slower"
        for s in names[1:]:
            a, b = st[base]["instructions"], st[s]["instructions"]
            cells.append(f"{b / a:.2f}" if a and b else "-")
        rows.append("| " + " | ".join(cells) + " |")
    geo = ["**geomean**"] + [""] * len(names) + [f"**{geomean(speed[s]):.2f}x**" for s in names[1:]]
    geo += [""] * (len(names) - 1)
    rows.append("| " + " | ".join(geo) + " |")
    return "\n".join(rows)


def detail(res: dict) -> str:
    out = ["| workload | stack | wall median | quiet median | CPU s | peak RSS MB | instructions | instr spread |",
           "|---|---|---:|---:|---:|---:|---:|---:|"]
    for wname, w in res["workloads"].items():
        for s in res["stacks"]:
            st = w["stats"][s]
            ins = f"{st['instructions'] / 1e9:.2f}G" if st["instructions"] else "-"
            spr = f"{100 * st['instructions_spread']:.1f}%" if st["instructions_spread"] is not None else "-"
            out.append(f"| {wname} | {s} | {st['median']:.2f}s | {st['quiet_median']:.2f}s | {st['cpu']:.1f} "
                       f"| {st['max_rss_mb']:.0f} | {ins} | {spr} |")
    return "\n".join(out)


def write_synth_report(path: Path, res: dict) -> None:
    """VERSUS-SYNTH.md: the synthetic workload's scoreboard (VERSUS.md keeps
    the named workloads)."""
    sys.path.insert(0, str(Path(__file__).resolve().parent / "synth"))
    import run as synth_run  # noqa: E402

    e = res["env"]
    stacks = "\n".join(f"- **{s['name']}**: {s['what']}" + (f" Env: `{s['env']}`." if s["env"] else "")
                       for s in res["stack_defs"])
    sy = res["synth"]
    errs = [f"- point {p['i']} on {s}: {m}" for p in sy["results"] for s, m in p["errors"].items()]
    md = f"""# Stock toolchain vs justrust, synthetic agent workload (System 3)

Generated by `bench/versus.py report` from `{path.relative_to(suite.ROOT)}`
(justrust {e['justrust_rev']}{' dirty' if e['justrust_dirty'] else ''}, {e['cpu']}, {res['started']}).
Generator {sy['generator']}, seed {sy['seed']}, {sy['points']} points, {sy['reps']} run(s) per point and stack.

Each point is one agent build call drawn from the mined distribution of
real calls (`bench/synth/weights.json`, see `bench/synth/README.md`): a
command plus the edits an agent made before it, on the pinned snapshots.
Points are drawn in proportion to real call counts, so the mean wall over
points is the expected wait per agent call. `Nx` = stock mean / stack mean.
`instr` = total user-space instructions of the build trees relative to
stock.

{synth_run.table(sy)}

## Stacks

{stacks}

## Points

{synth_run.detail(sy)}

## Errors

{chr(10).join(errs) or 'none'}
"""
    out = VERSUS_MD.with_name("VERSUS-SYNTH.md")
    out.write_text(md)
    print(f"wrote {out}")


def write_report(path: Path) -> None:
    res = json.loads(path.read_text())
    if "synth" in res:
        return write_synth_report(path, res)
    e = res["env"]
    stacks = "\n".join(f"- **{s['name']}**: {s['what']}" + (f" Env: `{s['env']}`." if s["env"] else "")
                       for s in res["stack_defs"])
    works = "\n".join(f"- **{n}**: {w['what']}. `{' '.join(w['cmd'])}` (suite scenario `{w['scenario']}`)"
                      for n, w in res["workloads"].items())
    md = f"""# Stock toolchain vs justrust (System 3)

Generated by `bench/versus.py report` from `{path.relative_to(suite.ROOT)}`
(justrust {e['justrust_rev']}{' dirty' if e['justrust_dirty'] else ''}, {e['cpu']}, {res['started']}).
Stock: {e['stock_cargo']}, {e['stock_rustc']}.

Every workload runs on every stack on the same pinned sources, round-robin
with a rotating order, {res['runs']} measured rounds after {res['warmup']} warmup.
Wall medians (min-max). `Nx` = stock median / stack median; **bold** when
the ranges are disjoint and the move is above 10%, `~` otherwise.
`instr` = user-space instructions retired by the whole build tree relative
to stock: nearly noise-free, so it shows compiler work saved even when wall
ranges overlap.

{scoreboard(res)}

## Stacks

{stacks}

## Workloads

{works}

## Detail

{detail(res)}
"""
    VERSUS_MD.write_text(md)
    print(f"wrote {VERSUS_MD}")


# --- commands -------------------------------------------------------------


def parse_stack(spec: str) -> Stack:
    """NAME:K=V,K=V -> a justrust stack with that env (on top of defaults)."""
    name, _, kv = spec.partition(":")
    env = dict(p.split("=", 1) for p in kv.split(",") if p)
    return Stack(name, f"justrust defaults plus {env}", True, env, private_cache=True)


def cmd_list(_args) -> None:
    print("stacks:")
    for s in STACKS:
        print(f"  {s.name:12s} {s.what}")
    print("workloads:")
    for w in WORKLOADS:
        s = suite.BY_NAME[w.scenario]
        print(f"  {w.name:16s} {w.what}{' [cold, opt-in]' if s.cold else ''}\n  {'':16s} {' '.join(s.cmd)}")


def cmd_run(args) -> None:
    stacks = [s for s in STACKS if not args.only or s.name in args.only.split(",")]
    if args.remote:
        if os.environ.get("JUSTRUST_BENCH_SRC"):
            sys.exit("versus: --remote measures remote builds from the laptop; on the build machine itself "
                     "(bench/on-remote.sh) it would route to itself")
        stacks.append(REMOTE)
    stacks += [parse_stack(x) for x in args.stack]
    if stacks[0].name != "stock" and not args.only:
        sys.exit("versus: stock must be the first stack")
    if args.synth is not None:
        return cmd_run_synth(args, stacks)
    names = args.workloads or [w.name for w in WORKLOADS if not suite.BY_NAME[w.scenario].cold]
    if args.cold:
        names += [w.name for w in WORKLOADS if suite.BY_NAME[w.scenario].cold and w.name not in names]
    for n in names:
        if n not in BY_NAME:
            sys.exit(f"versus: unknown workload {n} (see `bench/versus.py list`)")
    if not open_instruction_counters(os.getpid()) and not args.no_instructions:
        sys.exit("versus: perf_event_open failed (kernel.perf_event_paranoid?); pass --no-instructions to skip")
    info = suite.env_info()
    info["stock_cargo"] = suite.sh([STOCK_CARGO, "-V"])
    info["stock_rustc"] = suite.sh(["/usr/bin/rustc", "-V"]) if Path("/usr/bin/rustc").exists() else suite.sh(["rustc", "-V"])
    res = {"started": time.strftime("%Y-%m-%dT%H:%M:%S%z"), "env": info, "runs": args.n, "warmup": args.warmup,
           "stacks": [s.name for s in stacks],
           "stack_defs": [{"name": s.name, "what": s.what, "env": s.env, "justrust": s.justrust} for s in stacks],
           "workloads": {}}
    for k, n in enumerate(names):
        print(f"versus: {n} ({k + 1}/{len(names)})", file=sys.stderr)
        res["workloads"][n] = run_workload(BY_NAME[n], stacks, args.n, args.warmup, args.quiet_wait)
    suite.verify_binary_identity(info)
    res["finished"] = time.strftime("%Y-%m-%dT%H:%M:%S%z")
    out = suite.RESULTS / (f"versus-{time.strftime('%Y%m%d-%H%M%S')}-{info['justrust_rev']}"
                           f"{'-dirty' if info['justrust_dirty'] else ''}.json")
    out.parent.mkdir(parents=True, exist_ok=True)
    out.write_text(json.dumps(res, indent=1) + "\n")
    print(scoreboard(res))
    print(f"\nsaved {out}")
    if not args.no_report:
        write_report(out)


def cmd_run_synth(args, stacks: list[Stack]) -> None:
    """The synthetic, usage-weighted workload (bench/synth/) on every stack."""
    sys.path.insert(0, str(Path(__file__).resolve().parent / "synth"))
    import run as synth_run  # noqa: E402

    if not open_instruction_counters(os.getpid()) and not args.no_instructions:
        sys.exit("versus: perf_event_open failed (kernel.perf_event_paranoid?); pass --no-instructions to skip")
    info = suite.env_info()
    info["stock_cargo"] = suite.sh([STOCK_CARGO, "-V"])
    res = {"started": time.strftime("%Y-%m-%dT%H:%M:%S%z"), "env": info, "stacks": [s.name for s in stacks],
           "stack_defs": [{"name": s.name, "what": s.what, "env": s.env, "justrust": s.justrust} for s in stacks]}
    only = {int(x) for x in args.only_points.split(",")} if args.only_points else None
    res["synth"] = synth_run.run(stacks, args.synth, args.points, args.n, args.quiet_wait,
                                 partial=suite.RESULTS / "partial" / f"synth{args.synth}.json", only=only,
                                 resume=args.resume)
    suite.verify_binary_identity(info)
    if only:  # a subset is for debugging, never a result to compare
        print(synth_run.table(res["synth"]))
        return
    res["finished"] = time.strftime("%Y-%m-%dT%H:%M:%S%z")
    out = suite.RESULTS / (f"versus-synth{args.synth}-{time.strftime('%Y%m%d-%H%M%S')}-{info['justrust_rev']}"
                           f"{'-dirty' if info['justrust_dirty'] else ''}.json")
    out.parent.mkdir(parents=True, exist_ok=True)
    out.write_text(json.dumps(res, indent=1) + "\n")
    print(synth_run.table(res["synth"]))
    print(f"\nsaved {out}")
    if not args.no_report:
        write_report(out)


def newest() -> Path:
    files = sorted(suite.RESULTS.glob("versus-2*.json"))
    if not files:
        sys.exit("versus: no results yet, run `bench/versus.py run`")
    return files[-1]


def cmd_report(args) -> None:
    write_report(Path(args.file).resolve() if args.file else newest())


def cmd_compare(args) -> None:
    a, b = (json.loads(Path(f).read_text()) for f in (args.a, args.b))
    if "synth" in a or "synth" in b:
        if not ("synth" in a and "synth" in b):
            sys.exit("versus: compare a synthetic result with another synthetic result")
        sys.path.insert(0, str(Path(__file__).resolve().parent / "synth"))
        import run as synth_run  # noqa: E402

        print(synth_run.compare(a["synth"], b["synth"]))
        return
    if a["env"]["snapshots"] != b["env"]["snapshots"]:
        print("WARNING: different pinned snapshots, numbers are not comparable", file=sys.stderr)
    print(f"A {args.a} (justrust {a['env']['justrust_rev']})\nB {args.b} (justrust {b['env']['justrust_rev']})")
    print(f"{'workload':16s} {'stack':12s} {'A':>8s} {'B':>8s} {'%':>6s} {'instr %':>8s}  verdict")
    for w in a["workloads"]:
        if w not in b["workloads"]:
            continue
        for s in a["stacks"]:
            if s not in b["stacks"]:
                continue
            sa, sb = a["workloads"][w]["stats"][s], b["workloads"][w]["stats"][s]
            pct = 100 * (sb["median"] - sa["median"]) / sa["median"]
            ia, ib = sa["instructions"], sb["instructions"]
            ipct = f"{100 * (ib - ia) / ia:+7.1f}%" if ia and ib else "       -"
            print(f"{w:16s} {s:12s} {sa['median']:8.2f} {sb['median']:8.2f} {pct:+5.0f}% {ipct}  {verdict(sa, sb)}")


def cmd_clean(_args) -> None:
    shutil.rmtree(WORK, ignore_errors=True)
    print(f"removed {WORK}")


def main() -> None:
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    sub = ap.add_subparsers(dest="cmd", required=True)
    sub.add_parser("list").set_defaults(fn=cmd_list)
    r = sub.add_parser("run")
    r.add_argument("workloads", nargs="*")
    r.add_argument("-n", type=int, default=5, help="measured rounds (default 5)")
    r.add_argument("--warmup", type=int, default=1, help="discarded rounds first (default 1)")
    r.add_argument("--cold", action="store_true", help="also the fresh-target-dir workloads")
    r.add_argument("--stack", action="append", default=[], metavar="NAME:K=V,...",
                   help="extra justrust stack with env overrides (repeatable)")
    r.add_argument("--only", metavar="A,B", help="run only these built-in stacks")
    r.add_argument("--remote", action="store_true",
                   help="add the `remote` stack: check/clippy/test forced onto the remote machine")
    r.add_argument("--quiet-wait", type=float, default=0, metavar="SECS",
                   help="before each run wait up to SECS for no foreign rustc and load < 2")
    r.add_argument("--no-instructions", action="store_true")
    r.add_argument("--no-report", action="store_true", help="do not rewrite bench/VERSUS.md")
    r.add_argument("--synth", type=int, metavar="SEED",
                   help="run the synthetic usage-weighted workload (bench/synth/) instead of the named "
                        "workloads; -n is then repetitions per point")
    r.add_argument("--points", type=int, default=40, help="synthetic points (default 40)")
    r.add_argument("--resume", action="store_true",
                   help="synthetic: keep the points of an interrupted run of the same seed and stacks")
    r.add_argument("--only-points", metavar="I,J",
                   help="synthetic: run only these point indices (debugging, nothing saved)")
    r.set_defaults(fn=cmd_run)
    p = sub.add_parser("report")
    p.add_argument("file", nargs="?")
    p.set_defaults(fn=cmd_report)
    c = sub.add_parser("compare")
    c.add_argument("a")
    c.add_argument("b")
    c.set_defaults(fn=cmd_compare)
    sub.add_parser("clean").set_defaults(fn=cmd_clean)
    args = ap.parse_args()
    args.fn(args)


if __name__ == "__main__":
    main()
