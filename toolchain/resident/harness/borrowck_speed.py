#!/usr/bin/env python3
"""borrowck_speed.py: how long the borrow checker takes, next to type inference, on real code.

usage: borrowck_speed.py [NAME...] [--bin ra-jit] [--apps-dir dir] [--modes polonius,nll]
                         [--jobs N] [--json out.json] [--md out.md] [--top 20]

Runs `ra-jit borrowck --project WS --timings OUT` on each app (those of apps/apps.json, the
harness's corpus, and EXTRA below), which checks every body in both modes, and reports for each:
the bodies, type inference's total, the checker's total with and without MIR lowering, the
checker's time per body (p50, p99, max), where it goes by phase, and the slowest bodies with
the sizes they had. Each run is one thread. Each body is inferred and checked once cold, paying
for the queries it is the first to need, and then inferred and checked in each mode as an edit
in the daemon pays it: the rest answered, the trait solver's cache empty ("edit" columns).

`--rustc TOOLCHAIN` measures rustc instead, as the baseline: `cargo +TOOLCHAIN check
--workspace --all-targets` with `-Zself-profile` (and `-Zpolonius=next` for polonius), and the
self time of the `mir_borrowck` and `typeck` queries in the workspace's own crates, read with
measureme's `summarize` (`--summarize`, default on PATH).
"""
import argparse, asyncio, glob, json, os, shutil, sys, time
from typing import Any

import borrowck_apps

HERE = os.path.dirname(os.path.abspath(__file__))
# Apps the speed of the checker is reported on that apps.json does not build.
EXTRA = {
    "hyperfine": {"repo": "https://github.com/sharkdp/hyperfine", "rev": "c5161524980522320c7d3425e714cc0965a869a5"},
    "just": {"repo": "https://github.com/casey/just", "rev": "d6fee036379c531369dc37f67f8d4640f6bf1e2d"},
    "axum": {"repo": "https://github.com/tokio-rs/axum", "rev": "86aa6153e47b7862737acdcfe3a0eb3a162b939a"},
    "biome": {"repo": "https://github.com/biomejs/biome", "rev": "e2e5a36e1be8581419742a85268ad82f020da62e"},
}
DEFAULT = ["ripgrep", "fd", "bat", "hyperfine", "just", "axum", "nushell", "helix", "biome", "corpus"]


def pct(sorted_values, q):
    if not sorted_values:
        return 0
    return sorted_values[min(len(sorted_values) - 1, int(q * len(sorted_values)))]


def summarize(rows, mode, top):
    """What one run of one app measured, in one mode: `rows` are the bodies, each with its
    inference time and its check in each mode."""
    rows = [dict(r, **r["modes"][mode]) for r in rows if mode in r["modes"] and r["modes"][mode]["counts"]["bodies"]]
    own = lambda r: r["borrowck_us"] - r["phases_us"]["mir"]
    checked = sorted(own(r) for r in rows)
    phases: dict[str, int] = {}
    for r in rows:
        for k, v in r["phases_us"].items():
            phases[k] = phases.get(k, 0) + v
    infer = sum(r["infer_us"] for r in rows)
    # As an edit in the daemon pays it, where the run measured that; else cold, as the check.
    warm = sum(r.get("infer_warm_us", r["infer_us"]) for r in rows)
    first = sum(r.get("first_us", 0) for r in rows)
    total = sum(r["borrowck_us"] for r in rows)
    excl = sum(checked)
    slowest = sorted(rows, key=own, reverse=True)[:top]
    return {
        "bodies": len(rows),
        "infer_ms": infer / 1000,
        "infer_warm_ms": warm / 1000,
        "first_check_ms": first / 1000,
        "borrowck_ms": total / 1000,
        "borrowck_excl_mir_ms": excl / 1000,
        "ratio_to_infer": excl / warm if warm else None,
        "per_body_us": {"p50": pct(checked, 0.5), "p99": pct(checked, 0.99), "max": checked[-1] if checked else 0},
        "phases_ms": {k: v / 1000 for k, v in phases.items()},
        "slowest": [{"name": r["name"], "path": r["path"], "us": own(r), "infer_us": r.get("infer_warm_us", r["infer_us"]),
                     "phases_us": r["phases_us"], "counts": r["counts"]} for r in slowest],
    }


async def measure(name, spec, modes, args, gate):
    """{mode: summary} of one run of the app, which checks every body in both modes."""
    async with gate:
        began = time.monotonic()
        try:
            if name in borrowck_apps.LOCAL:
                ws = borrowck_apps.LOCAL[name]
            else:
                ws = await borrowck_apps.checkout(name, spec, args.apps_dir)
            out = os.path.join(args.work, f"{name}.jsonl")
            env = dict(os.environ, RUST_BACKTRACE="0")
            argv = [args.bin, "borrowck", "--project", ws, "--timings", out]
            code, _, err = await borrowck_apps.run(argv, ws, args.timeout, env)
        except borrowck_apps.StepError as e:
            return {mode: {"failed": str(e)} for mode in modes}
        if code != 0:
            return {mode: {"failed": f"`{' '.join(argv)}` exited {code}: {err[-800:]}"} for mode in modes}
        rows = [json.loads(line) for line in open(out) if line.strip()]
        results = {}
        for mode in modes:
            result = summarize(rows, mode, args.top)
            result["secs"] = round(time.monotonic() - began)
            results[mode] = result
            print(f"{name} {mode}: {result['bodies']} bodies, borrowck {result['borrowck_excl_mir_ms']:.0f} ms "
                  f"(infer {result['infer_ms']:.0f} ms), max {result['per_body_us']['max']} us ({result['secs']}s)",
                  file=sys.stderr, flush=True)
        return results


async def rustc_profile(name, spec, modes, args, gate):
    return {mode: await rustc_profile_mode(name, spec, mode, args, gate) for mode in modes}


async def rustc_profile_mode(name, spec, mode, args, gate):
    """rustc's own time in `mir_borrowck` and `typeck` on the workspace's crates."""
    async with gate:
        began = time.monotonic()
        try:
            ws = borrowck_apps.LOCAL[name] if name in borrowck_apps.LOCAL else await borrowck_apps.checkout(
                name, spec, args.apps_dir)
            prof = os.path.join(args.work, f"rustc-{name}.{mode}")
            shutil.rmtree(prof, ignore_errors=True)
            os.makedirs(prof)
            flags = f"-Zself-profile={prof} -Zself-profile-events=default"
            if mode == "polonius":
                flags += " -Zpolonius=next"
            # A fresh target dir: a crate cargo finds fresh is not compiled, so not profiled.
            target = os.path.join(args.work, f"target-{name}.{mode}")
            shutil.rmtree(target, ignore_errors=True)
            env = dict(os.environ, RUSTFLAGS=flags, CARGO_TARGET_DIR=target)
            argv = ["cargo", f"+{args.rustc}", "check", "--workspace", "--all-targets", "--quiet"]
            code, _, err = await borrowck_apps.run(argv, ws, args.timeout, env)
            if code != 0:
                return {"failed": f"`{' '.join(argv)}` exited {code}: {err[-800:]}"}
            meta = json.loads(await borrowck_apps.checked(
                ["cargo", f"+{args.rustc}", "metadata", "--no-deps", "--format-version", "1"], ws, 600))
            own = {t["name"].replace("-", "_") for p in meta["packages"] for t in p["targets"]}
            totals = {"mir_borrowck": 0.0, "typeck": 0.0}
            crates = 0
            for data in sorted(glob.glob(os.path.join(prof, "*.mm_profdata"))):
                crate = os.path.basename(data).rsplit("-", 1)[0]
                if crate not in own:
                    continue
                crates += 1
                # `summarize --json` writes the summary beside the profile, as <name>.json.
                await borrowck_apps.checked([args.summarize, "summarize", "--json", data], ws, 600)
                with open(data.removesuffix(".mm_profdata") + ".json") as f:
                    summary = json.load(f)
                for q in summary["query_data"]:
                    # Newer nightlies call `typeck` `typeck_root`.
                    label = "typeck" if q["label"] == "typeck_root" else q["label"]
                    if label in totals:
                        t = q["self_time"]
                        totals[label] += t["secs"] + t["nanos"] / 1e9
        except borrowck_apps.StepError as e:
            return {"failed": str(e)}
        result = {"crates": crates, "mir_borrowck_ms": totals["mir_borrowck"] * 1000,
                  "typeck_ms": totals["typeck"] * 1000, "secs": round(time.monotonic() - began)}
        print(f"{name} rustc {mode}: {result}", file=sys.stderr, flush=True)
        return result


def rustc_markdown(results, modes):
    lines = ["| App | Mode | Crates | rustc typeck ms | rustc mir_borrowck ms | mir_borrowck / typeck |", "|---|---|---|---|---|---|"]
    for name, by_mode in results.items():
        for mode in modes:
            r = by_mode.get(mode, {})
            if "failed" in r or not r:
                lines.append(f"| {name} | {mode} | failed | | | |")
                continue
            ratio = r["mir_borrowck_ms"] / r["typeck_ms"] if r["typeck_ms"] else 0
            lines.append(f"| {name} | {mode} | {r['crates']} | {r['typeck_ms']:.0f} | {r['mir_borrowck_ms']:.0f} | {ratio:.1%} |")
    return "\n".join(lines) + "\n"


def markdown(results, modes, top):
    lines = ["| App | Mode | Bodies | Infer ms (cold) | Infer ms (edit) | Borrowck ms (edit, excl. MIR) | MIR ms | "
             "Borrowck / infer (edit) | First check ms (cold) | p50 us | p99 us | max us |",
             "|---|---|---|---|---|---|---|---|---|---|---|---|"]
    for name, by_mode in results.items():
        for mode in modes:
            r = by_mode.get(mode, {})
            if "failed" in r or not r:
                lines.append(f"| {name} | {mode} | failed | | | | | | | | | |")
                continue
            p = r["per_body_us"]
            lines.append(f"| {name} | {mode} | {r['bodies']} | {r['infer_ms']:.0f} | {r.get('infer_warm_ms', 0):.0f} | "
                         f"{r['borrowck_excl_mir_ms']:.0f} | {r['phases_ms'].get('mir', 0):.0f} | {r['ratio_to_infer']:.1%} | "
                         f"{r.get('first_check_ms', 0):.0f} | {p['p50']} | {p['p99']} | {p['max']} |")
    phases = None
    for by_mode in results.values():
        for r in by_mode.values():
            if "phases_ms" in r:
                phases = list(r["phases_ms"])
                break
        if phases:
            break
    if phases:
        lines += ["", "Where the time goes (ms, summed over the apps):", "",
                  "| Mode | " + " | ".join(phases) + " |", "|---|" + "---|" * len(phases)]
        for mode in modes:
            sums = {k: sum(by[mode]["phases_ms"].get(k, 0) for by in results.values() if "phases_ms" in by.get(mode, {}))
                    for k in phases}
            lines.append(f"| {mode} | " + " | ".join(f"{sums[k]:.0f}" for k in phases) + " |")
    for mode in modes:
        every = [dict(s, app=name) for name, by in results.items() if "slowest" in by.get(mode, {})
                 for s in by[mode]["slowest"]]
        every.sort(key=lambda s: s["us"], reverse=True)
        lines += ["", f"Slowest bodies, {mode}:", "",
                  "| us | infer us | App | Body | points | regions | loans | constraints | localized | top phases |",
                  "|---|---|---|---|---|---|---|---|---|---|"]
        for s in every[:top]:
            c = s["counts"]
            ph = sorted(((v, k) for k, v in s["phases_us"].items() if k != "mir"), reverse=True)[:3]
            lines.append(f"| {s['us']} | {s['infer_us']} | {s['app']} | `{s['name']}` | {c['points']} | {c['regions']} | "
                         f"{c['loans']} | {c['constraints']} | {c['localized']} | {', '.join(f'{k} {v}' for v, k in ph)} |")
    return "\n".join(lines) + "\n"


async def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("names", nargs="*")
    ap.add_argument("--bin", default=os.environ.get("RA_JIT_BIN", "ra-jit"))
    ap.add_argument("--apps-dir", default=os.environ.get("APPS_DIR") or os.path.join(HERE, "vendor", "apps"))
    ap.add_argument("--work", default=os.path.join(HERE, "vendor", "borrowck-speed"))
    ap.add_argument("--modes", default="polonius,nll")
    ap.add_argument("--jobs", type=int, default=1)
    ap.add_argument("--timeout", type=int, default=7200)
    ap.add_argument("--top", type=int, default=20)
    ap.add_argument("--json")
    ap.add_argument("--md")
    ap.add_argument("--rustc", help="measure rustc at this toolchain instead")
    ap.add_argument("--summarize", default="summarize")
    args = ap.parse_args()
    args.bin = os.path.abspath(args.bin) if os.sep in args.bin else args.bin
    os.makedirs(args.work, exist_ok=True)
    specs = json.load(open(borrowck_apps.SPECS))
    specs.update(EXTRA)
    specs.update({name: {} for name in borrowck_apps.LOCAL})
    names = args.names or DEFAULT
    modes = args.modes.split(",")
    # Check each app out once before the runs that share it.
    for name in names:
        if name not in borrowck_apps.LOCAL:
            await borrowck_apps.checkout(name, specs[name], args.apps_dir)
    gate = asyncio.Semaphore(args.jobs)
    async with asyncio.TaskGroup() as group:
        run = rustc_profile if args.rustc else measure
        tasks = {name: group.create_task(run(name, specs[name], modes, args, gate)) for name in names}
    results: dict[str, dict[str, Any]] = {name: task.result() for name, task in tasks.items()}
    failed = [f"{n} {m}: {r['failed']}" for n, by in results.items() for m, r in by.items() if "failed" in r]
    report = rustc_markdown(results, modes) if args.rustc else markdown(results, modes, args.top)
    print(report)
    if args.json:
        json.dump(results, open(args.json, "w"), indent=1)
    if args.md:
        open(args.md, "w").write(report)
    for it in failed:
        print("FAILED", it, file=sys.stderr)
    sys.exit(1 if failed else 0)


if __name__ == "__main__":
    asyncio.run(main())
