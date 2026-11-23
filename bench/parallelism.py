#!/usr/bin/env python3
"""Deterministic compiler parallelism analyzer for jr-rustc.

Sweeps `-Zthreads=1,2,4,8,16` (configurable) with `-Ztime-passes` on a Rust
crate, classifies every compiler pass as SERIAL, CONTENDED, or SCALING, and
computes:
  - Per-pass thread scaling: T(1), T(N), speedup S = T(1)/T(N), efficiency S/N
  - Ranked theoretical wall-time savings if each pass scaled to N_target cores
  - Amdahl's Law floor at 16, 32, 96, and infinity cores

Usage:
  python3 bench/parallelism.py
  python3 bench/parallelism.py --threads 1,2,4,8,16 --mode full
  python3 bench/parallelism.py --mode incr --target-cores 96
  python3 bench/parallelism.py --json /tmp/par.json
"""

from __future__ import annotations

import argparse
import json
import os
import re
import shutil
import subprocess
import sys
from pathlib import Path

TIME_PASS_RE = re.compile(
    r"^time:\s+([0-9.]+);\s+rss:\s+\d+MB\s+->\s+\d+MB\s+\([^)]+\)\s+(.+?)\s*$"
)

# Top-level passes that wrap sub-passes (excluded from the "leaf" sum so we
# don't double-count nested timers in the Amdahl breakdown).
WRAPPER_PASSES = {
    "total",
    "expand_crate",
    "codegen_crate",
    "link",
    "link_crate",
    "link_binary",
    "resolve_crate",
    "lint_checking",
    "incr_comp_persist_result_cache",
}


def find_real_cargo() -> str:
    try:
        out = subprocess.check_output(["rustup", "which", "cargo"], text=True).strip()
        if out and Path(out).is_file():
            return out
    except Exception:
        pass
    cargo = shutil.which("cargo")
    if not cargo:
        raise RuntimeError("could not find cargo")
    return cargo


def default_rustc() -> str:
    dev = Path("toolchain/rustc/build/jr/dev/bin/rustc")
    if dev.is_file():
        return str(dev.resolve())
    rel = Path("toolchain/rustc/build/jr/release/bin/rustc")
    if rel.is_file():
        return str(rel.resolve())
    return subprocess.check_output(["rustup", "which", "rustc"], text=True).strip()


def run_once(
    cargo: str,
    rustc: str,
    target_dir: str,
    threads: int,
    mode: str,
    touch_file: Path,
    extra_cargo_args: list[str],
) -> dict[str, float]:
    touch_file.touch()
    env = os.environ.copy()
    env["RUSTC"] = rustc
    env["CARGO_TARGET_DIR"] = target_dir
    env["CARGO_INCREMENTAL"] = "0" if mode == "full" else "1"

    cmd = [
        cargo,
        "rustc",
        *extra_cargo_args,
        "--",
        f"-Zthreads={threads}",
        "-Ztime-passes",
    ]
    proc = subprocess.run(cmd, env=env, capture_output=True, text=True)
    if proc.returncode != 0:
        sys.stderr.write(proc.stderr[-2000:])
        raise RuntimeError(f"rustc failed (exit {proc.returncode}) at -Zthreads={threads}")

    passes: dict[str, float] = {}
    for line in proc.stderr.splitlines():
        m = TIME_PASS_RE.match(line.strip())
        if m:
            secs = float(m.group(1))
            name = m.group(2)
            # Keep max if a pass name appears multiple times.
            if secs > passes.get(name, 0.0):
                passes[name] = secs
    return passes


def classify(speedup: float, max_threads: int) -> str:
    if speedup < 1.25:
        return "SERIAL"
    if speedup < 0.35 * max_threads:
        return "CONTENDED"
    return "SCALING"


def main() -> int:
    ap = argparse.ArgumentParser(description="Deterministic compiler parallelism analyzer")
    ap.add_argument("--rustc", default=default_rustc(), help="Path to rustc binary")
    ap.add_argument(
        "--target-dir",
        default=os.path.join(os.environ.get("JCODE_SCRATCH_DIR", "/tmp"), "rustc-par-bench"),
        help="Cargo target dir (reuses warmed deps)",
    )
    ap.add_argument(
        "--threads",
        default="1,2,4,8,16",
        help="Comma-separated -Zthreads values to sweep",
    )
    ap.add_argument(
        "--mode",
        choices=["full", "incr"],
        default="full",
        help="full (CARGO_INCREMENTAL=0) or incr (CARGO_INCREMENTAL=1)",
    )
    ap.add_argument("--touch", default="src/lib.rs", help="File to touch between runs")
    ap.add_argument("--target-cores", type=int, default=96, help="Target core count for theoretical savings")
    ap.add_argument("--min-secs", type=float, default=0.05, help="Minimum pass time at max threads to display")
    ap.add_argument("--json", help="Optional path to write full JSON report")
    ap.add_argument("cargo_args", nargs="*", default=["--lib"], help="Extra args to cargo rustc (default: --lib)")
    args = ap.parse_args()

    threads_list = sorted({int(x) for x in args.threads.split(",") if x.strip()})
    if 1 not in threads_list:
        threads_list.insert(0, 1)
    max_t = max(threads_list)

    cargo = find_real_cargo()
    touch_file = Path(args.touch)

    # Warm dependencies once so only the target crate is timed.
    env = os.environ.copy()
    env["RUSTC"] = args.rustc
    env["CARGO_TARGET_DIR"] = args.target_dir
    subprocess.run([cargo, "check", *args.cargo_args, "-q"], env=env, check=True)
    if args.mode == "incr":
        # One warm incremental pass to populate incremental cache.
        run_once(cargo, args.rustc, args.target_dir, max_t, "incr", touch_file, args.cargo_args)

    results: dict[int, dict[str, float]] = {}
    for t in threads_list:
        print(f"[sweep] mode={args.mode} -Zthreads={t} ...", file=sys.stderr, flush=True)
        results[t] = run_once(cargo, args.rustc, args.target_dir, t, args.mode, touch_file, args.cargo_args)

    t1_map = results[1]
    tn_map = results[max_t]
    total_1 = t1_map.get("total", 0.0)
    total_n = tn_map.get("total", 0.0)

    rows = []
    for name, tn in tn_map.items():
        if name in WRAPPER_PASSES or tn < args.min_secs:
            continue
        t1 = t1_map.get(name, tn)
        speedup = (t1 / tn) if tn > 0 else 1.0
        eff = speedup / max_t
        kind = classify(speedup, max_t)
        # Theoretical wall time if this pass scaled linearly from T(1) to target_cores:
        ideal_target = t1 / args.target_cores
        potential_save = max(0.0, tn - ideal_target)
        rows.append(
            {
                "pass": name,
                "t1": t1,
                "tn": tn,
                "speedup": speedup,
                "efficiency": eff,
                "kind": kind,
                "potential_save": potential_save,
            }
        )

    rows.sort(key=lambda r: r["potential_save"], reverse=True)

    print(f"\n=== Parallelism Analysis ({args.mode}, rustc={Path(args.rustc).name}) ===")
    print(
        f"Total wall time: T(1)={total_1:.3f}s -> T({max_t})={total_n:.3f}s "
        f"(overall {total_1 / total_n:.2f}x on {max_t} threads)"
    )
    print("\nScaling curve:")
    for t in threads_list:
        tot = results[t].get("total", 0.0)
        print(f"  -Zthreads={t:<2d}  total={tot:6.3f}s  speedup={total_1 / tot:5.2f}x")

    print(
        f"\nRanked bottlenecks by theoretical savings at {args.target_cores} cores "
        f"(leaf passes >= {args.min_secs:.2f}s):"
    )
    print(
        f"{'Pass':<42} {'T(1)':>7} {f'T({max_t})':>7} {'Spdup':>6} {'Class':<10} {f'Save@{args.target_cores}c':>9}"
    )
    print("-" * 88)
    for r in rows[:20]:
        print(
            f"{r['pass']:<42} {r['t1']:7.3f} {r['tn']:7.3f} {r['speedup']:5.2f}x "
            f"{r['kind']:<10} {r['potential_save']:8.3f}s"
        )

    serial_sum = sum(r["tn"] for r in rows if r["kind"] == "SERIAL")
    contended_sum = sum(r["tn"] for r in rows if r["kind"] == "CONTENDED")
    scaling_sum = sum(r["tn"] for r in rows if r["kind"] == "SCALING")
    serial_frac = min(1.0, serial_sum / total_n) if total_n > 0 else 0.0

    print("\n=== Theoretical Amdahl Bounds (from current leaf passes) ===")
    print(f"  Current T({max_t}):       {total_n:6.3f}s")
    print(f"  SERIAL passes:       {serial_sum:6.3f}s ({100 * serial_frac:4.1f}% of T({max_t}))")
    print(f"  CONTENDED passes:    {contended_sum:6.3f}s")
    print(f"  SCALING passes:      {scaling_sum:6.3f}s")
    print(f"  Amdahl floor (if SERIAL stays 1x, rest -> inf cores): {serial_sum:6.3f}s")
    ideal_all = total_1 / args.target_cores
    print(f"  Ideal linear scaling from T(1)={total_1:.2f}s to {args.target_cores}c: {ideal_all:6.3f}s")

    if args.json:
        payload = {
            "mode": args.mode,
            "rustc": args.rustc,
            "threads": threads_list,
            "target_cores": args.target_cores,
            "totals": {str(t): results[t].get("total", 0.0) for t in threads_list},
            "passes": rows,
            "serial_sum": serial_sum,
            "contended_sum": contended_sum,
            "scaling_sum": scaling_sum,
        }
        Path(args.json).write_text(json.dumps(payload, indent=2))
        print(f"\nWrote JSON report to {args.json}")

    return 0


if __name__ == "__main__":
    raise SystemExit(main())
