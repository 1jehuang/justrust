#!/usr/bin/env python3
"""System 1: rolling log of real agent compile time in the repos we work in.

Not a benchmark: what agents actually waited for, day by day, in Jcode,
Jcode Desktop, and justrust, from before justrust existed until today.

Two sources, both already on this machine:
  - Jcode session history (`justrust history --dump`): every foreground
    cargo/justrust check, test, build, clippy call an agent made, with the
    wall time the agent was blocked. Covers September 2026 onward, so it has
    the pre-justrust baseline. Same measure before and after.
  - justrust's own run records (~/.justrust/runs/*/summary.json): since
    2026-10-06. Adds the reuse ledger: units compiled vs fresh, depcache and
    build-script cache hits and the rustc seconds they saved, slot refresh.

  bench/trend.py update     rebuild bench/trend/*.csv and bench/trend/TREND.md
  bench/trend.py report     print TREND.md without writing

Commit the files under bench/trend/ after `update` (aggregates only, no
command text or session content).
"""

from __future__ import annotations

import argparse
import csv
import datetime as dt
import json
import os
import re
import statistics
import subprocess
import sys
import tempfile
from collections import defaultdict
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
OUT = ROOT / "bench" / "trend"
HOME = Path.home()
JR_HOME = Path(os.environ.get("JUSTRUST_HOME", HOME / ".justrust"))
J = os.environ.get("JUSTRUST", "justrust")
REPOS = ["jcode-desktop", "jcode", "justrust"]
# Agents switched to justrust's recorded check/test on 2026-10-07 (commit
# "Add agent interface"). Everything before this day is the baseline.
JUSTRUST_START = dt.date(2026, 10, 7)
KINDS = {"Test": "test", "Check": "check", "Build": "build"}
CD = re.compile(r"(?:^|[;&|(]\s*)cd\s+([^\s;&|)]+)")
SUB = re.compile(r"(?:cargo|justrust)\s+(?:\+\S+\s+)?(?:-\S+\s+)*(test|nextest|check|clippy|build|run)\b([^|;&>]*)")
PKG = re.compile(r"-p\s+(\S+)")
# cargo/justrust in command position (not inside a grep pattern, heredoc,
# or python string that merely mentions it).
RUNS_BUILD = re.compile(
    r"(?:^|[;&|(\n]|&&|\|\|)\s*(?:[A-Z_][A-Z0-9_]*=\S*\s+)*(?:timeout\s+\S+\s+)?(?:\S*/)?(?:cargo|justrust)\s+"
    r"(?:\+\S+\s+)?(?:-\S+\s+)*(?:test|nextest|check|clippy|build|run)\b"
)


def runs_build(c: dict) -> bool:
    for cmd in c["commands"]:
        if cmd.startswith("<"):
            # Self-dev tool calls. Sub-second ones are acknowledgements of a
            # build that runs later, not the build.
            if c["wall_secs"] >= 1.0:
                return True
        elif RUNS_BUILD.search(cmd):
            return True
    return False


def repo_of(path: str) -> str | None:
    p = path.replace("~", str(HOME)).rstrip("/")
    base = os.path.basename(p)
    return base if base in REPOS else None


def call_repo(c: dict) -> str | None:
    # An explicit `cd <repo>` in the command wins over the session's dir.
    for cmd in c["commands"]:
        m = CD.search(cmd)
        if m:
            r = repo_of(m.group(1))
            if r:
                return r
    return repo_of(c["working_dir"] or "")


def pattern(c: dict) -> str:
    """`test -p jcode-desktop-ui --lib`: kind, packages, --lib/--workspace."""
    for cmd in c["commands"]:
        if cmd.startswith("<desktop_selfdev:test>"):
            cmd = cmd.split(">", 1)[1]
        m = SUB.search(cmd)
        if not m:
            continue
        sub = {"nextest": "test", "clippy": "check", "run": "build"}.get(m.group(1), m.group(1))
        rest = m.group(2)
        out = [sub] + [f"-p {p}" for p in dict.fromkeys(PKG.findall(rest))]
        out += [f for f in ("--lib", "--workspace", "--release") if f in rest.split()]
        return " ".join(out)
    return KINDS.get(c["kind"], c["kind"].lower())


def load_calls(dump: Path | None) -> list[dict]:
    if dump is None:
        tmp = Path(tempfile.mkstemp(suffix=".jsonl")[1])
        subprocess.run([J, "history", "--dump", str(tmp), "--top", "0"], check=True,
                       stdout=subprocess.DEVNULL)
        dump = tmp
    calls = []
    for line in dump.read_text().splitlines():
        c = json.loads(line)
        if c["background"] or c["kind"] not in KINDS or not c.get("wall_secs"):
            continue
        w = c["wall_secs"]
        if not (0.05 < w < 1800) or not runs_build(c):  # hung, or not a build
            continue
        repo = call_repo(c)
        if not repo:
            continue
        calls.append({
            "date": dt.date.fromtimestamp(c["timestamp"]),
            "repo": repo,
            "kind": KINDS[c["kind"]],
            "pattern": pattern(c),
            "wall": w,
            "error": c["compile_errors"] > 0,
            "via_justrust": any("justrust " in x for x in c["commands"]),
        })
    return calls


def load_runs() -> list[dict]:
    """Reuse ledger per recorded run (foreground agent runs in our repos)."""
    runs = []
    for d in sorted((JR_HOME / "runs").iterdir()):
        try:
            s = json.loads((d / "summary.json").read_text())
        except (OSError, ValueError):
            continue
        repo = None
        for r in REPOS:
            if f"/{r}" in s["cwd"] and repo_of(s["cwd"].split(f"/{r}")[0] + f"/{r}"):
                repo = r
                break
        if s["cwd"].startswith(str(JR_HOME / "bench")):
            continue  # benchmark runs are not agent work
        if not repo or s.get("subcommand") not in ("check", "test", "build", "clippy", "run"):
            continue
        u = s["units"]
        scripts_hit = scripts_saved = 0.0
        sp = d / "scripts.jsonl"
        if sp.exists():
            for line in sp.read_text().splitlines():
                try:
                    x = json.loads(line)
                except ValueError:
                    continue
                if x.get("hit"):
                    scripts_hit += 1
                    scripts_saved += max(0.0, x.get("saved_secs", 0.0) - x.get("wall", 0.0))
        slot = s.get("slot") or {}
        runs.append({
            "date": dt.date.fromtimestamp(s["start"]),
            "repo": repo,
            "wall": s["wall"],
            "compiled": u["compiled"] - u.get("depcache_hits", 0),
            "fresh": u.get("fresh", 0),
            "depcache_hits": u.get("depcache_hits", 0),
            "depcache_saved": max(0.0, u.get("depcache_saved_secs", 0.0)),
            "script_hits": scripts_hit,
            "script_saved": scripts_saved,
            "slot_refreshed": slot.get("refreshed_units", 0) or 0,
            "rustc_cpu": u.get("rustc_cpu_secs", 0.0),
        })
    return runs


def pct(xs: list[float], p: float) -> float:
    xs = sorted(xs)
    return xs[min(len(xs) - 1, int(p / 100 * len(xs)))] if xs else 0.0


def dist(xs: list[float]) -> dict:
    return {
        "n": len(xs),
        "p50": round(statistics.median(xs), 2) if xs else 0.0,
        "p90": round(pct(xs, 90), 2),
        "mean": round(statistics.fmean(xs), 2) if xs else 0.0,
        "hours": round(sum(xs) / 3600, 2),
    }


def week(d: dt.date) -> str:
    y, w, _ = d.isocalendar()
    return f"{y}-W{w:02d}"


def write_csvs(calls: list[dict], runs: list[dict]) -> None:
    OUT.mkdir(parents=True, exist_ok=True)
    groups: dict[tuple, list[dict]] = defaultdict(list)
    for c in calls:
        groups[(c["date"].isoformat(), c["repo"], c["kind"])].append(c)
        groups[(c["date"].isoformat(), c["repo"], "all")].append(c)
    with open(OUT / "daily.csv", "w", newline="") as f:
        w = csv.writer(f)
        w.writerow(["date", "repo", "kind", "n", "p50_s", "p90_s", "mean_s", "blocked_h",
                    "error_share", "via_justrust_share"])
        for (d, repo, kind), cs in sorted(groups.items()):
            st = dist([c["wall"] for c in cs])
            w.writerow([d, repo, kind, st["n"], st["p50"], st["p90"], st["mean"], st["hours"],
                        round(sum(c["error"] for c in cs) / len(cs), 3),
                        round(sum(c["via_justrust"] for c in cs) / len(cs), 3)])
    rg: dict[tuple, list[dict]] = defaultdict(list)
    for r in runs:
        rg[(r["date"].isoformat(), r["repo"])].append(r)
    with open(OUT / "reuse.csv", "w", newline="") as f:
        w = csv.writer(f)
        w.writerow(["date", "repo", "runs", "units_compiled", "units_fresh", "depcache_hits",
                    "depcache_saved_s", "script_hits", "script_saved_s", "slot_refreshed_units",
                    "rustc_cpu_s", "reuse_share"])
        for (d, repo), rs in sorted(rg.items()):
            comp = sum(r["compiled"] for r in rs)
            reused = sum(r["fresh"] + r["depcache_hits"] for r in rs)
            w.writerow([d, repo, len(rs), comp, sum(r["fresh"] for r in rs),
                        sum(r["depcache_hits"] for r in rs),
                        round(sum(r["depcache_saved"] for r in rs), 1),
                        int(sum(r["script_hits"] for r in rs)),
                        round(sum(r["script_saved"] for r in rs), 1),
                        sum(r["slot_refreshed"] for r in rs),
                        round(sum(r["rustc_cpu"] for r in rs), 1),
                        round(reused / (reused + comp), 3) if reused + comp else 0.0])


def report(calls: list[dict], runs: list[dict]) -> str:
    today = max(c["date"] for c in calls)
    recent_from = today - dt.timedelta(days=6)
    out = [
        "# Rolling compile time (System 1)",
        "",
        "Generated by `bench/trend.py update` from Jcode session history and",
        "justrust run records. Wall time an agent was blocked on a foreground",
        "`check`/`test`/`build`/`clippy` call (cargo or justrust), per repo.",
        f"Baseline: every call before {JUSTRUST_START} (before agents used justrust).",
        f"Current: the last 7 days of data ({recent_from} .. {today}).",
        "",
        "Raw agent data: the mix of work changes week to week (cold builds,",
        "big refactors), so read the per-command table for like-for-like",
        "numbers and `bench/factors.py` for controlled ones.",
        "",
        "## Baseline vs current",
        "",
        "| repo | kind | baseline n | p50 | p90 | current n | p50 | p90 | p50 change |",
        "|---|---|---:|---:|---:|---:|---:|---:|---:|",
    ]
    for repo in REPOS:
        for kind in ("all", "test", "check", "build"):
            sel = [c for c in calls if c["repo"] == repo and (kind == "all" or c["kind"] == kind)]
            a = dist([c["wall"] for c in sel if c["date"] < JUSTRUST_START])
            b = dist([c["wall"] for c in sel if c["date"] >= recent_from])
            if a["n"] < 5 and b["n"] < 5:
                continue
            ch = f"{100 * (b['p50'] - a['p50']) / a['p50']:+.0f}%" if a["n"] >= 5 and b["n"] >= 5 and a["p50"] else "-"
            out.append(f"| {repo} | {kind} | {a['n']} | {a['p50']:.1f}s | {a['p90']:.1f}s | "
                       f"{b['n']} | {b['p50']:.1f}s | {b['p90']:.1f}s | {ch} |")

    out += ["", "## Weekly, all kinds", "",
            "| week | " + " | ".join(f"{r} n / p50 / p90" for r in REPOS) + " |",
            "|---|" + "---:|" * len(REPOS)]
    weeks = sorted({week(c["date"]) for c in calls})
    for wk in weeks:
        cells = []
        for repo in REPOS:
            st = dist([c["wall"] for c in calls if c["repo"] == repo and week(c["date"]) == wk])
            cells.append(f"{st['n']} / {st['p50']:.1f}s / {st['p90']:.0f}s" if st["n"] >= 5 else "-")
        out.append(f"| {wk} | " + " | ".join(cells) + " |")

    out += ["", "## Same command, before vs after", "",
            "Command patterns with at least 10 calls in both periods (all of",
            f"the baseline vs everything since {JUSTRUST_START}).", "",
            "| repo | command | before n | p50 | p90 | since n | p50 | p90 | p50 change |",
            "|---|---|---:|---:|---:|---:|---:|---:|---:|"]
    pats: dict[tuple, list[dict]] = defaultdict(list)
    for c in calls:
        pats[(c["repo"], c["pattern"])].append(c)
    rows = []
    for (repo, p), cs in pats.items():
        a = dist([c["wall"] for c in cs if c["date"] < JUSTRUST_START])
        b = dist([c["wall"] for c in cs if c["date"] >= JUSTRUST_START])
        if a["n"] >= 10 and b["n"] >= 10:
            rows.append((a["hours"] + b["hours"], repo, p, a, b))
    for _, repo, p, a, b in sorted(rows, reverse=True)[:15]:
        out.append(f"| {repo} | `{p}` | {a['n']} | {a['p50']:.1f}s | {a['p90']:.0f}s | "
                   f"{b['n']} | {b['p50']:.1f}s | {b['p90']:.0f}s | "
                   f"{100 * (b['p50'] - a['p50']) / a['p50']:+.0f}% |")

    out += ["", "## Work not redone (reuse ledger)", "",
            "From justrust run records (since 2026-10-06; agent runs only, bench",
            "runs excluded). `fresh` = units cargo found up to date, `depcache` =",
            "units restored from the shared dependency cache instead of compiled,",
            "`scripts` = build-script runs replayed from cache, `slot refresh` =",
            "units copied into an agent's slot from the shared target dir.",
            "Saved seconds are the recorded rustc/script time of the original build.", "",
            "| week | repo | runs | compiled | fresh | depcache hits | depcache saved | script hits | script saved | slot refreshed | reused share |",
            "|---|---|---:|---:|---:|---:|---:|---:|---:|---:|---:|"]
    rw: dict[tuple, list[dict]] = defaultdict(list)
    for r in runs:
        rw[(week(r["date"]), r["repo"])].append(r)
    for (wk, repo), rs in sorted(rw.items()):
        comp = sum(r["compiled"] for r in rs)
        reused = sum(r["fresh"] + r["depcache_hits"] for r in rs)
        out.append(
            f"| {wk} | {repo} | {len(rs)} | {comp} | {sum(r['fresh'] for r in rs)} | "
            f"{sum(r['depcache_hits'] for r in rs)} | {sum(r['depcache_saved'] for r in rs) / 60:.0f} min | "
            f"{int(sum(r['script_hits'] for r in rs))} | {sum(r['script_saved'] for r in rs) / 60:.1f} min | "
            f"{sum(r['slot_refreshed'] for r in rs)} | "
            f"{100 * reused / (reused + comp) if reused + comp else 0:.0f}% |")
    return "\n".join(out) + "\n"


def main() -> None:
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument("cmd", choices=["update", "report"])
    ap.add_argument("--dump", type=Path, help="use this `justrust history --dump` file instead of re-mining")
    args = ap.parse_args()
    calls = load_calls(args.dump)
    if not calls:
        sys.exit("trend: no calls found in session history")
    runs = load_runs()
    text = report(calls, runs)
    if args.cmd == "update":
        write_csvs(calls, runs)
        (OUT / "TREND.md").write_text(text)
        print(f"wrote {OUT}/daily.csv, reuse.csv, TREND.md ({len(calls)} calls, {len(runs)} runs)", file=sys.stderr)
    print(text)


if __name__ == "__main__":
    main()
