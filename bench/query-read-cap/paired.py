#!/usr/bin/env python3
"""Alternate preserved before/after binaries with matching explicit core affinity."""
import argparse
from datetime import datetime, timezone
import hashlib
import json
import os
from pathlib import Path
import statistics
import subprocess

from run import CASES


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--before", required=True, type=Path)
    parser.add_argument("--after", required=True, type=Path)
    parser.add_argument("--output", required=True, type=Path)
    parser.add_argument("--rounds", type=int, default=3)
    args = parser.parse_args()
    binaries = {"before": args.before.resolve(), "after": args.after.resolve()}
    original_affinity = os.sched_getaffinity(0)
    evidence = {
        "schema": 1,
        "scope": "paired fixed-affinity actual-module diagnostic, not whole compiler or broad gate",
        "started_utc": datetime.now(timezone.utc).isoformat(),
        "binary_sha256": {name: hashlib.sha256(path.read_bytes()).hexdigest() for name, path in binaries.items()},
        "order_per_round": ["before", "after", "after", "before"],
        "quiet_system_guaranteed": False,
        "affinity_policy": "one-thread CPU0, eight-thread CPU0..7",
        "samples": [], "summary": [],
    }
    try:
        for case, threads, events in CASES:
            affinity = {0} if threads == 1 else set(range(8))
            assert affinity <= original_affinity
            os.sched_setaffinity(0, affinity)
            samples = {name: [] for name in binaries}
            for round_index in range(args.rounds):
                for position, name in enumerate(["before", "after", "after", "before"]):
                    result = subprocess.run([str(binaries[name]), case, str(threads), str(events)], capture_output=True, text=True, check=True)
                    sample = json.loads(result.stdout)
                    sample.update(binary=name, round=round_index + 1, position=position + 1, affinity=sorted(affinity))
                    samples[name].append(sample["seconds"])
                    evidence["samples"].append(sample)
                    print(json.dumps(sample), flush=True)
                    args.output.write_text(json.dumps(evidence, indent=2) + "\n")
            evidence["summary"].append({
                "case": case, "threads": threads, "events": events,
                "before_median_seconds": statistics.median(samples["before"]),
                "after_median_seconds": statistics.median(samples["after"]),
                "before_range_seconds": [min(samples["before"]), max(samples["before"])],
                "after_range_seconds": [min(samples["after"]), max(samples["after"])],
            })
        evidence["finished_utc"] = datetime.now(timezone.utc).isoformat()
        args.output.write_text(json.dumps(evidence, indent=2) + "\n")
    finally:
        os.sched_setaffinity(0, original_affinity)


if __name__ == "__main__":
    main()
