#!/usr/bin/env python3
"""benchhistory.py: the benchmark history the site's /bench page charts, and the regression check.

    benchhistory.py build RESULT [--history FILE]
    benchhistory.py live LATENCY... [--history FILE] [--machine NAME]
    benchhistory.py check HISTORY [--last N] [--least N] [--noise PERCENT] [--report FILE]

`build` appends one line to FILE for a result of bench.py, by default its platform's
(benchplatforms.py): docs/benchmarks/history/<platform>.jsonl, e.g. linux-x86-64.jsonl; `live`
appends one line (default docs/benchmarks/history/macos-arm64-live.jsonl) for the files
live_latency.py wrote with LATENCY_JSON, one per program. A line keeps the medians and spreads
of each phase, not every sample: the full result is the workflow's artifact, and the latest of
each platform is docs/benchmarks/<platform>.json.

A build line:
    {"suite": "build", "date", "commit", "platform", "machine", "cpu", "cores", "runs", "run_url",
     "results": {"<project>/<backend>": {"tag", "store_mb", "runtime_s": {"ra-jit", "cargo debug", "cargo release"},
                 "runtime_mb": {"ra-jit", "cargo debug", "cargo release"},
                 "resident_ready_mb", "resident_end_mb", "builds", "disk_mb", "cargo_disk_mb",
                 "phases": {"<phase>": {"wall_s", "cargo_s", "min_s", "max_s", "cargo_min_s", "cargo_max_s",
                                        "rss_mb", "mem_mb", "cargo_mem_mb", "resident_mb", "built", "link",
                                        "compiled", "build_s", "warm_s", "setups": {"<setup>": seconds}}}}}}
Memory (bench.py's `memory`, docs/benchmarks.md "Memory and disk"): `mem_mb` and `cargo_mem_mb` are
the peak of each side during the phase, `resident_mb` what the daemon held after it;
`resident_ready_mb` and `resident_end_mb` what it held once ready and after the `builds` builds
of the run, `disk_mb` ra-jit's target directory, store and home, `cargo_disk_mb` cargo's target.
A live line:
    {"suite": "live", "date", "commit", "machine", "battery", "run_url",
     "results": {"<program>": {"<mode>": {"n", "p50_ms", "p90_ms", "p95_ms", "p99_ms", "max_ms"}}}}
Keys a run did not measure are left out. A run's test suites (`--tests`) are two phases:
`test-edit`, the tests after an edit, and `test`, with nothing changed.

`check` compares the last line of HISTORY with the lines before it from the same machine (at
most `--last`, default 7): a number regressed when it is above the median of those by more than
its noise band and by more than its floor (50 ms for build times, 1 ms for live latency, 100 MB
of memory, 50 MB of store and disk). Memory checked is ra-jit's: its peak in each phase, the
daemon after each phase, once ready and at the end of the run, and its disk; cargo's is charted,
not checked. The band is `--noise` percent (default 10) or three times the
spread the history shows (1.4826 times the median absolute deviation, as a percent of the
median), whichever is larger; with fewer than three earlier lines it is twice `--noise`. A
number with fewer than `--least` earlier values (default 1) is not checked: on a machine whose
own runs differ by more than that band, as a hosted runner's do, one or two runs say nothing of
its noise. A phase the daemon built that cargo now builds is a regression whatever the times. The
regressions are written to stdout and, as markdown, to `--report`; the exit status is 1 when
there is one, and 0 with nothing to compare against.
"""
import datetime, json, os, statistics, sys

from benchplatforms import platform, slug

HERE = os.path.dirname(os.path.abspath(__file__))
ROOT = os.path.dirname(HERE)
HISTORY = os.path.join(ROOT, "docs/benchmarks/history")
LIVE_HISTORY = os.path.join(HISTORY, "macos-arm64-live.jsonl")
PHASE_FIELDS = ("wall_s", "cargo_s", "rss_mb", "built", "link", "compiled", "build_s", "warm_s",
                "warm_store_s", "warm_store_build_s", "check_s")
# What is checked, and the least change that counts.
FLOORS = {"wall_s": 0.05, "build_s": 0.05, "warm_s": 0.05, "rss_mb": 100, "store_mb": 50,
          "mem_mb": 100, "resident_mb": 100, "resident_ready_mb": 100, "resident_end_mb": 100, "disk_mb": 50,
          "p50_ms": 1.0, "p90_ms": 1.0, "ra-jit": 0.005}


def spread(samples, prefix=""):
    if not samples:
        return {}
    return {f"{prefix}min_s": round(min(samples), 3), f"{prefix}max_s": round(max(samples), 3)}


def build_line(result):
    host = result.get("host") or {}
    line = dict(suite="build", date=result["date"], commit=result.get("commit"), platform=platform(result), machine=host.get("name"),
                cpu=host.get("cpu"), cores=host.get("cores") or result.get("cores"), runs=result.get("runs"),
                run_url=result.get("run_url") or actions_run(), results={})
    for key, measured in result["results"].items():
        phases = {}
        for label, phase in measured["phases"].items():
            kept = {f: phase[f] for f in PHASE_FIELDS if phase.get(f) is not None}
            kept.update(spread(phase.get("samples_s")))
            kept.update(spread(phase.get("cargo_samples_s"), "cargo_"))
            setups = {name: s["median_s"] for name, s in (phase.get("setups") or {}).items() if s.get("median_s") is not None}
            if setups:
                kept["setups"] = setups
            held = phase.get("memory") or {}
            for field, side, number in (("mem_mb", "ra-jit", "peak_mb"), ("cargo_mem_mb", "cargo", "peak_mb"), ("resident_mb", "ra-jit", "resident_mb")):
                if (held.get(side) or {}).get(number) is not None:
                    kept[field] = held[side][number]
            phases[label] = kept
            if label == "test" and phase.get("edit_s") is not None:
                # The tests after an edit are a phase of their own, as charted and checked.
                edited = dict(wall_s=phase["edit_s"], cargo_s=phase["edit_cargo_s"], built=phase.get("built"))
                edited.update(spread(phase.get("edit_samples_s")))
                edited.update(spread(phase.get("edit_cargo_samples_s"), "cargo_"))
                phases["test-edit"] = edited
                kept["built"] = phase.get("built_still", phase.get("built"))
        entry = dict(tag=measured.get("tag"), phases=phases)
        if measured.get("store_mb") is not None:
            entry["store_mb"] = measured["store_mb"]
        if measured.get("runtime"):
            entry["runtime_s"] = {label: r["median_s"] for label, r in measured["runtime"].items() if isinstance(r, dict) and "median_s" in r}
            peaks = {label: r["peak_mb"] for label, r in measured["runtime"].items() if isinstance(r, dict) and r.get("peak_mb") is not None}
            if peaks:
                entry["runtime_mb"] = peaks
        resident, disk = measured.get("resident") or {}, measured.get("disk_mb") or {}
        for field, value in (("resident_ready_mb", resident.get("ready_mb")), ("resident_end_mb", resident.get("end_mb")),
                             ("builds", resident.get("builds")),
                             ("disk_mb", sum(disk.get(k) or 0 for k in ("target", "store", "home")) if disk else None),
                             ("cargo_disk_mb", disk.get("cargo target"))):
            if value is not None:
                entry[field] = value
        line["results"][key] = entry
    return line


def percentile(values, p):
    values = sorted(values)
    return values[min(len(values) - 1, int(p / 100 * len(values)))]


def live_line(files, machine):
    line = dict(suite="live", results={})
    for path in files:
        latency = json.load(open(path))
        if "modes" not in latency:
            raise SystemExit(f"{path}: not a live_latency.py result with metadata (no `modes`)")
        for field in ("date", "commit", "run_url"):
            if latency.get(field) and not line.get(field):
                line[field] = latency[field]
        host = latency.get("host") or {}
        line.setdefault("machine", machine or host.get("name"))
        line.setdefault("battery", host.get("battery"))
        program = line["results"].setdefault(latency["program"], {})
        for mode, rows in latency["modes"].items():
            totals = [row["total"] for row in rows if isinstance(row.get("total"), (int, float))]
            if totals:
                program[mode] = dict(n=len(totals), **{f"p{p}_ms": round(percentile(totals, p), 2) for p in (50, 90, 95, 99)},
                                     max_ms=round(max(totals), 2))
    line.setdefault("date", time_now())
    line.setdefault("run_url", actions_run())
    return line


def actions_run():
    """The GitHub Actions run this is part of, if it is."""
    if os.environ.get("GITHUB_RUN_ID"):
        return f"{os.environ.get('GITHUB_SERVER_URL', 'https://github.com')}/{os.environ['GITHUB_REPOSITORY']}/actions/runs/{os.environ['GITHUB_RUN_ID']}"
    return None


def time_now():
    return datetime.datetime.now(datetime.timezone.utc).strftime("%Y-%m-%dT%H:%M:%SZ")


def append(path, line):
    os.makedirs(os.path.dirname(path), exist_ok=True)
    with open(path, "a") as f:
        f.write(json.dumps(line, separators=(",", ":")) + "\n")
    print(f"appended {line['suite']} {line.get('commit') or ''} {line['date']} to {path}")


def series(line):
    """Every number of a line that is checked, by name, and the `built` of each phase."""
    numbers, built = {}, {}
    for key, entry in line["results"].items():
        if line["suite"] == "build":
            if entry.get("store_mb") is not None:
                numbers[(key, "store", "store_mb")] = entry["store_mb"]
            if (entry.get("runtime_s") or {}).get("ra-jit") is not None:
                numbers[(key, "runtime", "ra-jit")] = entry["runtime_s"]["ra-jit"]
            for field in ("resident_ready_mb", "resident_end_mb", "disk_mb"):
                if entry.get(field) is not None:
                    numbers[(key, "daemon" if field.startswith("resident") else "disk", field)] = entry[field]
            for label, phase in entry["phases"].items():
                for field in ("wall_s", "rss_mb", "mem_mb", "resident_mb", "build_s", "warm_s"):
                    if phase.get(field) is not None:
                        numbers[(key, label, field)] = phase[field]
                if phase.get("built"):
                    built[(key, label)] = phase["built"]
        else:
            for mode, stats in entry.items():
                for field in ("p50_ms", "p90_ms"):
                    if stats.get(field) is not None:
                        numbers[(key, mode, field)] = stats[field]
    return numbers, built


def regressions(lines, last, noise, least=1):
    now = lines[-1]
    same = [l for l in lines[:-1] if l.get("machine") == now.get("machine") and l.get("cores") == now.get("cores")][-last:]
    if not same:
        return [], 0
    numbers, built = series(now)
    earlier = [series(l) for l in same]
    found = []
    for name, value in sorted(numbers.items()):
        history = [n[name] for n, _ in earlier if name in n]
        if len(history) < max(least, 1):
            continue
        median = statistics.median(history)
        if len(history) >= 3 and median > 0:
            mad = statistics.median(abs(h - median) for h in history) * 1.4826
            band = max(noise, 300 * mad / median)
        else:
            band = 2 * noise
        floor = FLOORS.get(name[2], 0)
        if value > median * (1 + band / 100) and value - median > floor:
            found.append(dict(series=" ".join(name), was=median, now=value, percent=round(100 * (value - median) / median), band=round(band)))
    for name, how in sorted(built.items()):
        was = [b[name] for _, b in earlier if name in b]
        if was and was[-1] == "daemon" and how != "daemon":
            found.append(dict(series=" ".join(name) + " built", was="daemon", now=how, percent=None, band=None))
    return found, len(same)


def report(now, found, compared, least=1):
    head = f"{now['suite']} benchmarks on {now.get('machine')}, ra-jit {(now.get('commit') or '?')[:10]}, {now['date']}"
    if now.get("run_url"):
        head += f" ([run]({now['run_url']}))"
    if compared < least:
        return f"{head}: not checked, {compared} runs before it on this machine of the {least} its noise is told from."
    if not found:
        return f"{head}: nothing regressed against the {compared} runs before it."
    lines = [f"{head}: {len(found)} regressed against the median of the {compared} runs before it.", "",
             "| What | Before | Now | Change | Noise band |", "|---|---:|---:|---:|---:|"]
    for f in found:
        change = f"+{f['percent']}%" if f["percent"] is not None else ""
        band = f"{f['band']}%" if f["band"] is not None else ""
        lines.append(f"| {f['series']} | {f['was']} | {f['now']} | {change} | {band} |")
    return "\n".join(lines)


def options(argv, name, default=None):
    if name in argv:
        at = argv.index(name)
        value = argv[at + 1]
        del argv[at:at + 2]
        return value
    return default


def main():
    argv = sys.argv[1:]
    if not argv:
        raise SystemExit(__doc__)
    command = argv.pop(0)
    if command == "build":
        result = json.load(open(argv[0]))
        history = options(argv, "--history", os.path.join(HISTORY, f"{slug(platform(result))}.jsonl"))
        append(history, build_line(result))
    elif command == "live":
        history = options(argv, "--history", LIVE_HISTORY)
        machine = options(argv, "--machine")
        append(history, live_line(argv, machine))
    elif command == "check":
        last, noise = int(options(argv, "--last", "7")), float(options(argv, "--noise", "10"))
        least = int(options(argv, "--least", "1"))
        out = options(argv, "--report")
        lines = [json.loads(l) for l in open(argv[0]) if l.strip()]
        found, compared = regressions(lines, last, noise, least)
        text = report(lines[-1], found, compared, least)
        print(text)
        if out:
            open(out, "w").write(text + "\n")
        sys.exit(1 if found else 0)
    else:
        raise SystemExit(__doc__)


if __name__ == "__main__":
    main()
