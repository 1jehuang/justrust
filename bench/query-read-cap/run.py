#!/usr/bin/env python3
"""Run an immutable compiled binary and retain raw focused measurements."""
import argparse
import hashlib
import json
import os
from pathlib import Path
import platform
import statistics
import subprocess
from datetime import datetime, timezone

ROOT = Path(__file__).resolve().parents[2]
CASES = [("read", 1, 30_000_000), ("mixed", 1, 30_000_000), ("mixed", 8, 30_000_000), ("accepted", 1, 100_000)]


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--binary", required=True, type=Path)
    parser.add_argument("--source", required=True, type=Path)
    parser.add_argument("--output", required=True, type=Path)
    parser.add_argument("--compiler", required=True, type=Path)
    parser.add_argument("--build-run-id", required=True)
    parser.add_argument("--repetitions", type=int, default=3)
    args = parser.parse_args()
    binary = args.binary.resolve()
    source = args.source.resolve()
    output = {
        "schema": 1,
        "scope": "standalone actual-module capped event diagnostic, not whole compiler or broad gate",
        "started_utc": datetime.now(timezone.utc).isoformat(),
        "source_sha256": hashlib.sha256(source.read_bytes()).hexdigest(),
        "binary_sha256": hashlib.sha256(binary.read_bytes()).hexdigest(),
        "source": str(source.relative_to(ROOT)),
        "system_rustc": subprocess.check_output(["rustc", "--version"], text=True).strip(),
        "compiler_version": subprocess.check_output([str(args.compiler), "--version", "--verbose"], text=True).strip(),
        "build_run_id": args.build_run_id,
        "machine": platform.platform(),
        "cpu_model": next(line.split(":", 1)[1].strip() for line in Path("/proc/cpuinfo").read_text().splitlines() if line.startswith("model name")),
        "affinity": sorted(os.sched_getaffinity(0)),
        "samples": [],
        "summary": [],
    }
    for case, threads, events in CASES:
        samples = []
        for repetition in range(args.repetitions):
            result = subprocess.run([str(binary), case, str(threads), str(events)], text=True, capture_output=True, check=True)
            sample = json.loads(result.stdout)
            sample["repetition"] = repetition + 1
            samples.append(sample)
            output["samples"].append(sample)
            print(json.dumps(sample), flush=True)
            args.output.write_text(json.dumps(output, indent=2) + "\n")
        output["summary"].append({
            "case": case, "threads": threads, "events": events,
            "median_seconds": statistics.median(s["seconds"] for s in samples),
            "median_ns_per_event": statistics.median(s["ns_per_event"] for s in samples),
        })
    output["finished_utc"] = datetime.now(timezone.utc).isoformat()
    args.output.write_text(json.dumps(output, indent=2) + "\n")


if __name__ == "__main__":
    main()
