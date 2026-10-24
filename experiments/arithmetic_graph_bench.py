#!/usr/bin/env python3
"""Fresh-process arithmetic timings and bounded native graph profiles.

Run on the measured host, not around a remote client. This is a tiny synthetic
workload, not a representative Rust benchmark. No cache eviction, source edits,
CPU affinity changes, or compiler builds are performed. Profile serialization
is outside the timing samples. Graph intervals are wall intervals, not CPU time.
"""
import argparse
import hashlib
import json
import os
from pathlib import Path
import platform
import statistics
import subprocess
import time

from compiler import source_for


def invoke(command):
    start = time.perf_counter_ns()
    result = subprocess.run(command, capture_output=True, timeout=180)
    elapsed = time.perf_counter_ns() - start
    if result.returncode:
        raise RuntimeError(f"{command}: {result.returncode}\n{result.stderr.decode()}\n{result.stdout.decode()}")
    return result, elapsed


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--binary", required=True, type=Path)
    parser.add_argument("--revision", required=True)
    parser.add_argument("--output", required=True, type=Path)
    parser.add_argument("--rustc", default="rustc")
    args = parser.parse_args()
    root = args.output.resolve()
    root.mkdir(parents=True, exist_ok=False)
    binary = args.binary.resolve(strict=True)
    environment = {
        "revision": args.revision,
        "binary_sha256": hashlib.sha256(binary.read_bytes()).hexdigest(),
        "host": platform.node(),
        "topology": invoke(["lscpu"])[0].stdout.decode(),
        "rustc": invoke([args.rustc, "-Vv"])[0].stdout.decode(),
        "load_start": os.getloadavg(),
        "warmups": 1,
        "repeats": 7,
        "cache": "warm filesystem, fresh process, no incremental",
        "measurement": "shared host, uncontrolled contention",
        "timing": "process wall including output collection; excluding executable run; profiles separately",
    }
    (root / "environment.json").write_text(json.dumps(environment, indent=2))
    cells = [("native", w, m) for w in (1, 8, 96) for m in ("check", "build")]
    cells += [("stock", None, m) for m in ("check", "build")]
    rows = []
    with (root / "samples.jsonl").open("x") as stream:
        for count in (4, 256):
            text = source_for(count)
            source = root / f"functions-{count}.rs"
            source.write_text(text)
            for repeat in range(-1, 7):
                for tool, workers, mode in (cells if repeat % 2 == 0 else cells[::-1]):
                    artifact = root / f"{count}-{tool}-{workers}-{mode}-{repeat}"
                    if tool == "native":
                        command = [str(binary), str(source), "--workers", str(workers)]
                        command += ["--check"] if mode == "check" else ["-o", str(artifact)]
                    else:
                        command = [args.rustc, "--edition=2024", "--crate-name", "arithmetic", "-C", "opt-level=0", "-C", "overflow-checks=yes", str(source), "-o", str(artifact)]
                        if mode == "check":
                            command += ["--emit=metadata"]
                    result, wall = invoke(command)
                    summary = json.loads(result.stdout) if tool == "native" else None
                    if summary is not None:
                        assert summary["capture_complete"] and not summary["failures"] and not summary["events_dropped"]
                    if mode == "build":
                        execution, _ = invoke([str(artifact)])
                        assert execution.stdout == b"compiled correctly\n" and not execution.stderr
                    row = {"functions": count + 2, "source_bytes": len(text.encode()), "source_sha256": hashlib.sha256(text.encode()).hexdigest(), "tool": tool, "workers": workers, "mode": mode, "repeat": repeat, "wall_ns": wall, "summary": summary, "load": os.getloadavg()}
                    stream.write(json.dumps(row) + "\n")
                    stream.flush()
                    if repeat >= 0:
                        rows.append(row)
            for workers in (1, 8, 96):
                for mode in ("check", "build"):
                    profile = root / f"bounded-profile-{count}-{workers}-{mode}.json"
                    artifact = root / f"bounded-executable-{count}-{workers}"
                    command = [str(binary), str(source), "--workers", str(workers), "--profile", str(profile)]
                    command += ["--check"] if mode == "check" else ["-o", str(artifact)]
                    invoke(command)
                    data = json.loads(profile.read_text())
                    assert data["success"] and data["graph"]["capture_complete"]
                    assert data["graph"]["events_dropped"] == data["graph"]["unmatched_intervals"] == 0
                    if mode == "build":
                        execution, _ = invoke([str(artifact)])
                        assert execution.stdout == b"compiled correctly\n" and not execution.stderr
            print(f"Completed {count + 2}-function timings and profiles", flush=True)
    summary = []
    for count in (6, 258):
        for tool, workers, mode in cells:
            values = [row["wall_ns"] / 1e6 for row in rows if (row["functions"], row["tool"], row["workers"], row["mode"]) == (count, tool, workers, mode)]
            summary.append({"functions": count, "tool": tool, "workers": workers, "mode": mode, "median_ms": statistics.median(values), "min_ms": min(values), "max_ms": max(values)})
    (root / "summary.json").write_text(json.dumps(summary, indent=2))
    print(json.dumps(summary, indent=2))


if __name__ == "__main__":
    main()
