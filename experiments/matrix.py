#!/usr/bin/env python3
"""Run a fixed synthetic engine matrix on the machine holding the binary.

No remote provisioning, subscription keys, billing bypass, or build tool calls.
Use the private hosted experiment lifecycle wrapper to invoke this over SSH.
"""
import argparse
import datetime
import hashlib
import json
import os
from pathlib import Path
import platform
import resource
import subprocess
import time


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--binary", type=Path, required=True)
    parser.add_argument("--revision", required=True)
    parser.add_argument("--output", type=Path, required=True)
    parser.add_argument("--workers", default="1,2,4,8,16,32,48,64,96")
    parser.add_argument("--repeats", type=int, default=1)
    args = parser.parse_args()
    workers = [int(x) for x in args.workers.split(",")]
    if not workers or any(w < 1 or w > 4096 for w in workers) or not 1 <= args.repeats <= 100:
        parser.error("invalid workers or repeats")
    if len(args.revision) not in (40, 64) or any(c not in "0123456789abcdef" for c in args.revision.lower()):
        parser.error("revision must be a full Git SHA")
    binary = args.binary.resolve(strict=True)
    # Refuse a misleading source revision or an uncommitted benchmark source.
    root = Path(__file__).resolve().parent.parent
    revision = subprocess.check_output(["git", "rev-parse", "HEAD"], cwd=root, text=True).strip()
    dirty = subprocess.check_output(["git", "status", "--porcelain", "--untracked-files=no"], cwd=root, text=True)
    if revision != args.revision or dirty:
        parser.error("checkout revision mismatch or tracked modifications")
    matrix = [("wide", 4096, work) for work in (0, 1000, 100000)]
    matrix += [("chain", 512, 1000), ("diamond", 128, 1000),
               ("dynamic", 512, 1000), ("multi-output", 16, 1000),
               ("build-check-test", 16, 1000)]
    with args.output.open("x") as output:
        def emit(record):
            output.write(json.dumps(record, sort_keys=True) + "\n")
            output.flush()
        topology = subprocess.run(["lscpu", "--json"], text=True, capture_output=True, check=True)
        emit({"record": "environment", "schema": 1, "revision": revision,
              "date_utc": datetime.datetime.now(datetime.timezone.utc).isoformat(),
              "binary_sha256": hashlib.sha256(binary.read_bytes()).hexdigest(),
              "bootstrap_rustc": subprocess.check_output(["rustc", "--version", "--verbose"], text=True),
              "bootstrap_cargo": subprocess.check_output(["cargo", "--version"], text=True).strip(),
              "platform": platform.platform(), "cpu_count": os.cpu_count(),
              "lscpu": json.loads(topology.stdout), "workers": workers,
              "repeats": args.repeats, "seed": 1,
              "instrumentation": "always-on", "measurement": "contended/uncontrolled",
              "max_rss_unit": "KiB on Linux, cumulative child high-water mark"})
        for repeat in range(args.repeats):
            for shape, size, work in matrix:
                for count in workers:
                    before = resource.getrusage(resource.RUSAGE_CHILDREN)
                    started = time.monotonic_ns()
                    process = subprocess.run([str(binary), "--shape", shape,
                        "--size", str(size), "--work", str(work), "--seed", "1",
                        "--workers", str(count), "--revision", revision],
                        text=True, capture_output=True, timeout=120)
                    after = resource.getrusage(resource.RUSAGE_CHILDREN)
                    if process.returncode:
                        emit({"record": "failure", "repeat": repeat, "shape": shape,
                              "workers": count, "exit_code": process.returncode,
                              "stdout": process.stdout, "stderr": process.stderr})
                        raise RuntimeError(f"fixture failed: {process.stdout}\n{process.stderr}")
                    row = json.loads(process.stdout)
                    row.update(record="sample", repeat=repeat,
                        process_wall_ns=time.monotonic_ns() - started,
                        process_cpu_s=(after.ru_utime + after.ru_stime) - (before.ru_utime + before.ru_stime),
                        cumulative_children_max_rss_kib=after.ru_maxrss)
                    emit(row)
                    print(json.dumps({"shape": shape, "work": work, "workers": count,
                                      "engine_ns": row["engine_ns"]}), flush=True)


if __name__ == "__main__":
    main()
