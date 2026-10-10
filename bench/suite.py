#!/usr/bin/env python3
"""justrust benchmark suite: the agent edit -> check/test loop, reproducibly.

Every scenario runs on a pinned source snapshot (a `git archive` of a fixed
commit, extracted once under ~/.justrust/bench/work), with its own target dir,
so results do not depend on what is checked out in ~/jcode-desktop today or on
another agent's build state. Each iteration applies a unique one-line edit
(never reverting to an already-compiled state), runs the command through
justrust, and reads the recorded summary.json for the phase breakdown.

  bench/suite.py list                      scenarios and what they measure
  bench/suite.py run [-n 5] [scenario...]  run (default: every non-cold scenario)
  bench/suite.py compare A.json B.json     per-scenario median delta
  bench/suite.py show RESULT.json          print a saved result

Results go to bench/results/<timestamp>-<justrust rev>.json (commit them).
A sample is flagged `noisy` when another rustc was running at its start or
other processes averaged more than BENCH_NOISY_CORES (default 4) cores during
the build. Medians are over all samples, `quiet` medians over the rest. Use --quiet-wait to wait for a quiet machine before each sample.
"""

from __future__ import annotations

import argparse
import json
import os
import platform
import random
import shutil
import statistics
import subprocess
import sys
import time
from dataclasses import dataclass, field
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
HOME = Path.home()
JR_HOME = Path(os.environ.get("JUSTRUST_HOME", HOME / ".justrust"))
WORK = Path(os.environ.get("JUSTRUST_BENCH_WORK", JR_HOME / "bench" / "work"))
RESULTS = ROOT / "bench" / "results"
J = os.environ.get("JUSTRUST", "justrust")
# A sample is noisy when other processes averaged more than this many cores
# or another rustc was running when it started.
NOISY_CORES = float(os.environ.get("BENCH_NOISY_CORES", "4"))

# Pinned sources. Bump deliberately (and say so in FINDINGS.md): results from
# different pins are not comparable, and `compare` refuses to mix them.
SNAPSHOTS = {
    "desktop": {
        # Jcode Desktop needs the Jcode repo as a sibling (path dependencies
        # `../../../jcode/crates/...`).
        "repos": {
            "jcode-desktop": (HOME / "jcode-desktop", "7d38fb7"),
            "jcode": (HOME / "jcode", "abff94eeb"),
        },
        "cwd": "jcode-desktop",
        # 7d38fb7's lib tests miss a field its harness already uses (fixed
        # later upstream). Exact replacements, applied once at extraction.
        "fixups": [
            (
                "jcode-desktop/crates/jcode-desktop-ui/src/panel.rs",
                'ApiEvent::SessionStatus { session_id: "session-a".into(), status: "idle".into() }',
                'ApiEvent::SessionStatus { session_id: "session-a".into(), status: "idle".into(), pending_soft_interrupts: None }',
                2,
            ),
        ],
    },
    "justrust": {
        "repos": {"justrust": (ROOT, "f97fcc2")},
        "cwd": "justrust",
    },
}


@dataclass
class Edit:
    file: str  # relative to the snapshot root
    anchor: str  # must occur exactly once
    # Replacement for the anchor; `{n}` is a unique per-iteration number.
    template: str


@dataclass
class Scenario:
    name: str
    snapshot: str
    cmd: list[str]
    edit: Edit | None
    what: str
    expect_fail: bool = False
    cold: bool = False  # fresh target dir every iteration; opt-in
    env: dict[str, str] = field(default_factory=dict)


FPS = "jcode-desktop/crates/jcode-desktop-ui/src/fps_counter.rs"
DESKTOP_TEST = ["test", "-p", "jcode-desktop-ui", "--lib", "--", "fps_counter"]
DESKTOP_CHECK = ["check", "-p", "jcode-desktop-ui"]

SCENARIOS = [
    Scenario(
        "desktop-test-edit",
        "desktop",
        DESKTOP_TEST,
        Edit(
            FPS,
            "let mut counter = FpsCounter::default();\n        let now = Instant::now();\n        assert_eq!(counter.label(now, || snapshot(&[])),",
            "let mut counter = FpsCounter::default();\n        std::hint::black_box({n}u64);\n        let now = Instant::now();\n        assert_eq!(counter.label(now, || snapshot(&[])),",
        ),
        "THE reference loop: one line inside a #[test] fn of the 118k-line UI crate, run its tests",
    ),
    Scenario(
        "desktop-body-test",
        "desktop",
        DESKTOP_TEST,
        Edit(
            FPS,
            "            let current = snapshot();\n",
            "            let current = snapshot();\n            std::hint::black_box({n}u64);\n",
        ),
        "one-line body edit in non-test code of the UI crate, run its tests",
    ),
    Scenario(
        "desktop-body-check",
        "desktop",
        DESKTOP_CHECK,
        Edit(
            FPS,
            "            let current = snapshot();\n",
            "            let current = snapshot();\n            std::hint::black_box({n}u64);\n",
        ),
        "same body edit, `check` only (the inner type-check loop)",
    ),
    Scenario(
        "desktop-type-error",
        "desktop",
        DESKTOP_CHECK,
        Edit(
            FPS,
            "    ) -> String {\n        if self",
            '    ) -> String {\n        let _: u32 = "type error {n}";\n        if self',
        ),
        "time to the first type error after an edit (the check must fail)",
        expect_fail=True,
    ),
    Scenario(
        "desktop-noop-test",
        "desktop",
        DESKTOP_TEST,
        None,
        "no edit: pure overhead of a fresh `test` (cargo startup, fingerprints, test run)",
    ),
    Scenario(
        "desktop-upstream-body",
        "desktop",
        DESKTOP_TEST,
        Edit(
            "jcode-desktop/crates/jcode-desktop-ui-core/src/render_stats.rs",
            "        stat.renders += 1;\n",
            "        stat.renders += 1;\n        std::hint::black_box({n}u64);\n",
        ),
        "body edit in jcode-desktop-ui-core, a dependency of the UI crate, run UI tests",
    ),
    Scenario(
        "desktop-upstream-sig",
        "desktop",
        DESKTOP_CHECK,
        Edit(
            "jcode-desktop/crates/jcode-desktop-ui-core/src/render_stats.rs",
            "/// Times one render of `name` until the returned guard drops.\n",
            "pub fn bench_probe_{n}() {}\n\n/// Times one render of `name` until the returned guard drops.\n",
        ),
        "new pub item in jcode-desktop-ui-core: metadata changes, downstream re-checks",
    ),
    Scenario(
        "desktop-jcode-core-body",
        "desktop",
        DESKTOP_TEST,
        Edit(
            "jcode/crates/jcode-core/src/id.rs",
            "pub fn session_icon(name: &str) -> &'static str {\n",
            "pub fn session_icon(name: &str) -> &'static str {\n    std::hint::black_box({n}u64);\n",
        ),
        "body edit in jcode-core (sibling Jcode repo, deep in the graph), run UI tests",
    ),
    Scenario(
        "justrust-body-test",
        "justrust",
        ["test"],
        Edit(
            "justrust/src/runs.rs",
            "    let d = (paths::now() - t).max(0.0);\n",
            "    let d = (paths::now() - t).max(0.0);\n    std::hint::black_box({n}u64);\n",
        ),
        "small crate: body edit in this repo, full `test` (the old bench/edit-loop.sh)",
    ),
    Scenario(
        "desktop-cold-check",
        "desktop",
        DESKTOP_CHECK,
        None,
        "fresh target dir, shared depcache on: what a new agent checkout pays",
        cold=True,
    ),
    Scenario(
        "desktop-cold-check-nocache",
        "desktop",
        DESKTOP_CHECK,
        None,
        "fresh target dir, every justrust cache off: the from-scratch floor",
        cold=True,
        env={"JUSTRUST_DEPCACHE": "0", "JUSTRUST_BUILD_SCRIPT_CACHE": "0"},
    ),
]
BY_NAME = {s.name: s for s in SCENARIOS}


def sh(cmd: list[str], cwd: Path | None = None) -> str:
    return subprocess.run(
        cmd, cwd=cwd, check=True, capture_output=True, text=True
    ).stdout.strip()


def snapshot_dir(name: str) -> Path:
    spec = SNAPSHOTS[name]
    key = "-".join(f"{r}@{rev}" for r, (_, rev) in sorted(spec["repos"].items()))
    return WORK / name / key


def prepare(name: str) -> Path:
    """Extract the pinned snapshot once. Source files are restored per run."""
    root = snapshot_dir(name)
    for repo, (src, rev) in SNAPSHOTS[name]["repos"].items():
        dst = root / repo
        stamp = dst / ".bench-snapshot"
        if stamp.exists():
            continue
        print(f"bench: extracting {src}@{rev} -> {dst}", file=sys.stderr)
        if dst.exists():
            shutil.rmtree(dst)
        dst.mkdir(parents=True)
        archive = subprocess.Popen(
            ["git", "-C", str(src), "archive", rev], stdout=subprocess.PIPE
        )
        subprocess.run(["tar", "-x", "-C", str(dst)], stdin=archive.stdout, check=True)
        if archive.wait() != 0:
            sys.exit(f"bench: git archive {src}@{rev} failed")
        for rel, old, new, count in SNAPSHOTS[name].get("fixups", []):
            if not rel.startswith(repo + "/"):
                continue
            p = root / rel
            s = p.read_text()
            if s.count(old) != count:
                sys.exit(f"bench: fixup for {rel} matches {s.count(old)} times, expected {count}")
            p.write_text(s.replace(old, new))
        stamp.write_text(rev + "\n")
    return root


def pristine(root: Path, rel: str) -> Path:
    """Saved copy of a file as extracted, used to restore it."""
    p = root / ".pristine" / rel
    if not p.exists():
        p.parent.mkdir(parents=True, exist_ok=True)
        shutil.copy2(root / rel, p)
    return p


def apply_edit(root: Path, edit: Edit, n: int) -> None:
    orig = pristine(root, edit.file).read_text()
    if orig.count(edit.anchor) != 1:
        sys.exit(f"bench: anchor occurs {orig.count(edit.anchor)} times in {edit.file}")
    new = edit.template.replace("{n}", str(n))
    (root / edit.file).write_text(orig.replace(edit.anchor, new, 1))


def restore(root: Path, edit: Edit | None) -> None:
    if edit:
        shutil.copy2(pristine(root, edit.file), root / edit.file)


def foreign_load() -> tuple[float, int]:
    load = float(Path("/proc/loadavg").read_text().split()[0])
    try:
        rustc = int(sh(["pgrep", "-c", "-x", "rustc"]))
    except subprocess.CalledProcessError:
        rustc = 0
    return load, rustc


def wait_quiet(max_secs: float) -> None:
    deadline = time.time() + max_secs
    while time.time() < deadline:
        load, rustc = foreign_load()
        if rustc == 0 and load < 2.0:
            return
        time.sleep(5)


def new_id() -> str:
    t = time.time()
    stamp = time.strftime("%Y%m%d-%H%M%S", time.localtime(t)) + f"{int(t * 1000) % 1000:03d}"
    return f"{stamp}-{os.getpid()}{random.randrange(100, 999)}"


def run_once(s: Scenario, root: Path, target: Path) -> dict:
    rid = new_id()
    env = dict(os.environ)
    env.update(
        {
            "CARGO_TARGET_DIR": str(target),
            "JUSTRUST_RUN_ID_OVERRIDE": rid,
            "JUSTRUST_REMOTE": "0",
            "JUSTRUST_QUIET": "1",
            # Per-pass split for every crate. Safe here: the bench owns its
            # target dirs, so the mode never flips under a warm cache. Not for
            # cold runs: units compiled with -Ztime-passes are never stored
            # in the depcache, so forcing it would keep the cache empty.
            **({} if s.cold else {"JUSTRUST_PASSES": "always"}),
        }
    )
    env.pop("JUSTRUST_DISABLE", None)
    env.update(s.env)
    cwd = root / SNAPSHOTS[s.snapshot]["cwd"]
    load, rustc = foreign_load()
    t0 = time.monotonic()
    p = subprocess.run([J, *s.cmd], cwd=cwd, env=env, capture_output=True, text=True)
    wall = time.monotonic() - t0
    summ_path = JR_HOME / "runs" / rid / "summary.json"
    if not summ_path.exists():
        sys.exit(f"bench: {s.name}: no summary for run {rid} (exit {p.returncode})\n{p.stderr[-3000:]}")
    sm = json.loads(summ_path.read_text())
    failed = p.returncode != 0
    if failed != s.expect_fail:
        sys.exit(
            f"bench: {s.name}: exit {p.returncode}, expected {'failure' if s.expect_fail else 'success'}"
            f" (justrust log {rid})\n{p.stderr[-3000:]}"
        )
    local = [u for u in sm.get("top_units", []) if u.get("local")]
    split = {"frontend": 0.0, "codegen": 0.0, "link": 0.0, "incremental": 0.0, "other": 0.0}
    for u in local:
        for k, v in (u.get("split") or {}).items():
            split[k] = split.get(k, 0.0) + v
    res = sm.get("resources", {})
    return {
        "id": rid,
        "wall": round(wall, 3),
        "recorded_wall": round(sm["wall"], 3),
        "cpu": round(sm.get("cpu_secs", 0.0), 2),
        "startup": round(sm["phases"]["startup"], 3),
        "compile": round(sm["phases"]["compile"], 3),
        "test_run": round(sm["phases"]["test_run"], 3),
        "units": sm["units"]["compiled"],
        "local_units": sm["units"]["local"],
        "dep_units": sm["units"]["dependencies"],
        "split": {k: round(v, 3) for k, v in split.items()},
        "link_secs": round(sm.get("link", {}).get("link_pass_secs", 0.0), 3),
        "tests_passed": sm.get("tests", {}).get("passed", 0),
        "other_cores": round(res.get("avg_other_cores", 0.0), 2),
        "foreign_rustc_before": rustc,
        "load_before": load,
        "noisy": res.get("avg_other_cores", 0.0) > NOISY_CORES or rustc > 0,
        "top_unit": local[0]["name"] if local else None,
    }


def med(xs: list[float]) -> float:
    return round(statistics.median(xs), 3) if xs else 0.0


def stats(samples: list[dict]) -> dict:
    walls = [x["wall"] for x in samples]
    quiet = [x["wall"] for x in samples if not x["noisy"]]
    return {
        "n": len(walls),
        "median": med(walls),
        "min": round(min(walls), 3),
        "max": round(max(walls), 3),
        "quiet_n": len(quiet),
        "quiet_median": med(quiet),
        "phases": {
            k: med([x[k] for x in samples]) for k in ("startup", "compile", "test_run", "cpu", "link_secs")
        },
        "split": {k: med([x["split"][k] for x in samples]) for k in samples[0]["split"]},
        "units": med([x["units"] for x in samples]),
    }


def run_scenario(s: Scenario, n: int, warmup: int, quiet_wait: float) -> dict:
    root = prepare(s.snapshot)
    target = root / "target"
    samples = []
    # Unique edit numbers per invocation, so a re-run never lands on a state
    # the incremental cache has already seen.
    base = int(time.time()) % 100000 * 100
    # Cold runs still get a warmup: it fills the shared depcache, so measured
    # runs see the state a second fresh checkout sees.
    warm = warmup if (not s.cold or s.env.get("JUSTRUST_DEPCACHE") != "0") else 0
    total = warm + n
    try:
        for i in range(total):
            if s.cold:
                target = root / f"target-cold-{os.getpid()}-{i}"
                restore(root, s.edit)
            elif s.edit:
                apply_edit(root, s.edit, base + i)
            if quiet_wait:
                wait_quiet(quiet_wait)
            r = run_once(s, root, target)
            measured = i >= warm
            tag = "" if measured else " (warmup)"
            flag = " noisy" if r["noisy"] else ""
            print(
                f"  {s.name} {i + 1}/{total}: {r['wall']:.2f}s compile {r['compile']:.2f}s "
                f"units {r['units']} id {r['id']}{flag}{tag}",
                file=sys.stderr,
            )
            if measured:
                samples.append(r)
            if s.cold:
                shutil.rmtree(target, ignore_errors=True)
    finally:
        restore(root, s.edit)
    if s.edit is None and not s.cold and any(x["units"] for x in samples):
        print(f"  WARNING {s.name}: a no-op run compiled units (cache invalidation?)", file=sys.stderr)
    return {"what": s.what, "cmd": s.cmd, "samples": samples, "stats": stats(samples)}


def env_info() -> dict:
    def tryrun(cmd):
        try:
            return sh(cmd)
        except Exception:
            return None

    cpu = None
    for line in Path("/proc/cpuinfo").read_text().splitlines():
        if line.startswith("model name"):
            cpu = line.split(":", 1)[1].strip()
            break
    return {
        "host": platform.node(),
        "cpu": cpu,
        "ncpu": os.cpu_count(),
        "mem_gb": round(os.sysconf("SC_PAGE_SIZE") * os.sysconf("SC_PHYS_PAGES") / 2**30, 1),
        "kernel": platform.release(),
        "justrust": tryrun([J, "-V"]),
        "justrust_rev": tryrun(["git", "-C", str(ROOT), "rev-parse", "--short=12", "HEAD"]),
        "justrust_dirty": bool(tryrun(["git", "-C", str(ROOT), "status", "--porcelain", "--untracked-files=no"])),
        "rustc": tryrun(["rustc", "-V"]),
        "cargo": tryrun(["cargo", "-V"]),
        "governor": tryrun(["cat", "/sys/devices/system/cpu/cpu0/cpufreq/scaling_governor"]),
        "on_ac": tryrun(["cat", "/sys/class/power_supply/AC/online"]),
        "snapshots": {k: {r: rev for r, (_, rev) in v["repos"].items()} for k, v in SNAPSHOTS.items()},
    }


def table(result: dict) -> str:
    rows = [
        f"{'scenario':28s} {'median':>7s} {'min':>7s} {'max':>7s} {'n':>3s} {'quiet':>7s}"
        f" {'start':>6s} {'compile':>7s} {'tests':>6s} {'front':>6s} {'codegen':>7s} {'incr':>6s} {'link':>6s} {'units':>5s}"
    ]
    for name, sc in result["scenarios"].items():
        st = sc["stats"]
        ph, sp = st["phases"], st["split"]
        q = f"{st['quiet_median']:.2f}" if st["quiet_n"] else "-"
        rows.append(
            f"{name:28s} {st['median']:7.2f} {st['min']:7.2f} {st['max']:7.2f} {st['n']:3d} {q:>7s}"
            f" {ph['startup']:6.2f} {ph['compile']:7.2f} {ph['test_run']:6.2f}"
            f" {sp['frontend']:6.2f} {sp['codegen']:7.2f} {sp['incremental']:6.2f} {sp['link']:6.2f} {st['units']:5.0f}"
        )
    return "\n".join(rows)


def cmd_list(_args) -> None:
    for s in SCENARIOS:
        tag = " [cold, opt-in]" if s.cold else ""
        print(f"{s.name:28s} {s.what}{tag}\n{'':28s} justrust {' '.join(s.cmd)}")


def cmd_run(args) -> None:
    names = args.scenarios or [s.name for s in SCENARIOS if not s.cold]
    if args.cold:
        names += [s.name for s in SCENARIOS if s.cold and s.name not in names]
    for n in names:
        if n not in BY_NAME:
            sys.exit(f"bench: unknown scenario {n} (see `bench/suite.py list`)")
    # Fail before hours of building, not after: every anchor must exist once.
    for n in names:
        s = BY_NAME[n]
        if s.edit:
            root = prepare(s.snapshot)
            c = pristine(root, s.edit.file).read_text().count(s.edit.anchor)
            if c != 1:
                sys.exit(f"bench: {n}: anchor occurs {c} times in {s.edit.file}")
    info = env_info()
    started = time.strftime("%Y-%m-%dT%H:%M:%S%z")
    result = {"started": started, "env": info, "runs_per_scenario": args.n, "scenarios": {}}
    for k, name in enumerate(names):
        print(f"bench: {name} ({k + 1}/{len(names)})", file=sys.stderr)
        result["scenarios"][name] = run_scenario(BY_NAME[name], args.n, args.warmup, args.quiet_wait)
    result["finished"] = time.strftime("%Y-%m-%dT%H:%M:%S%z")
    out = Path(args.out) if args.out else RESULTS / (
        time.strftime("%Y%m%d-%H%M%S") + f"-{info['justrust_rev']}{'-dirty' if info['justrust_dirty'] else ''}.json"
    )
    out.parent.mkdir(parents=True, exist_ok=True)
    out.write_text(json.dumps(result, indent=1) + "\n")
    print(table(result))
    print(f"\nsaved {out}")


def cmd_show(args) -> None:
    r = json.loads(Path(args.file).read_text())
    e = r["env"]
    print(f"{args.file}: justrust {e['justrust_rev']}{' dirty' if e['justrust_dirty'] else ''}, {e['rustc']}, {e['cpu']}, {r['started']}")
    print(table(r))


def cmd_compare(args) -> None:
    a, b = (json.loads(Path(f).read_text()) for f in (args.a, args.b))
    if a["env"]["snapshots"] != b["env"]["snapshots"]:
        print("WARNING: different pinned snapshots, numbers are not comparable", file=sys.stderr)
    if a["env"].get("host") != b["env"].get("host"):
        print("WARNING: different machines", file=sys.stderr)
    print(f"A {args.a} (justrust {a['env']['justrust_rev']})\nB {args.b} (justrust {b['env']['justrust_rev']})")
    print(f"{'scenario':28s} {'A med':>7s} {'B med':>7s} {'delta':>7s} {'%':>6s}  verdict")
    for name in a["scenarios"]:
        if name not in b["scenarios"]:
            continue
        sa, sb = a["scenarios"][name]["stats"], b["scenarios"][name]["stats"]
        d = sb["median"] - sa["median"]
        pct = 100 * d / sa["median"] if sa["median"] else 0.0
        # Conservative: only call it when the sample ranges do not overlap.
        if sb["max"] < sa["min"]:
            verdict = "faster"
        elif sb["min"] > sa["max"]:
            verdict = "slower"
        else:
            verdict = "within noise"
        print(f"{name:28s} {sa['median']:7.2f} {sb['median']:7.2f} {d:+7.2f} {pct:+5.0f}%  {verdict}")


def main() -> None:
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    sub = ap.add_subparsers(dest="cmd", required=True)
    sub.add_parser("list").set_defaults(fn=cmd_list)
    r = sub.add_parser("run")
    r.add_argument("scenarios", nargs="*")
    r.add_argument("-n", type=int, default=5, help="measured runs per scenario (default 5)")
    r.add_argument("--warmup", type=int, default=1, help="discarded runs first (default 1)")
    r.add_argument("--cold", action="store_true", help="also run the cold (fresh target dir) scenarios")
    r.add_argument("--quiet-wait", type=float, default=0, metavar="SECS",
                   help="before each run wait up to SECS for no foreign rustc and load < 2")
    r.add_argument("-o", "--out", help="result file (default bench/results/<time>-<rev>.json)")
    r.set_defaults(fn=cmd_run)
    s = sub.add_parser("show")
    s.add_argument("file")
    s.set_defaults(fn=cmd_show)
    c = sub.add_parser("compare")
    c.add_argument("a")
    c.add_argument("b")
    c.set_defaults(fn=cmd_compare)
    args = ap.parse_args()
    args.fn(args)


if __name__ == "__main__":
    main()
