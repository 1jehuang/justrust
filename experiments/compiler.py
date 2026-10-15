#!/usr/bin/env python3
"""Measure actual source-to-ELF compilation, checking every executable result."""
import argparse
import hashlib
import json
import os
from pathlib import Path
import platform
import subprocess
import tempfile
import time


def run(argv, **kwargs):
    result = subprocess.run(argv, capture_output=True, timeout=120, **kwargs)
    if result.returncode:
        raise RuntimeError(f"command failed {argv!r}: {result.stdout!r}\n{result.stderr!r}")
    return result


def source_for(count):
    functions = [f"fn f{i}(x:i32)->i32 {{ if x < 0 {{ -x }} else {{ x + {i} }} }}" for i in range(count)]
    calls = [f"if f{i}(2) != {i + 2} {{ fail(0); }}" for i in range(count)]
    return "\n".join(functions + ["fn fail(x:i32) { let y=1/x; }", "fn main(){" + "\n".join(calls) + 'println!("compiled correctly");}'])


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--binary", required=True, type=Path)
    parser.add_argument("--revision", required=True)
    parser.add_argument("--output", required=True, type=Path)
    parser.add_argument("--workers", default="1,8,96")
    parser.add_argument("--repeats", default=3, type=int)
    parser.add_argument("--reference-rustc", type=Path)
    args = parser.parse_args()
    binary = args.binary.resolve(strict=True)
    revision = run(["git", "rev-parse", "HEAD"]).stdout.decode().strip()
    if revision != args.revision or run(["git", "diff", "HEAD", "--"]).stdout:
        parser.error("requires exact revision and clean tracked source checkout")
    workers = [int(w) for w in args.workers.split(",")]
    if args.repeats < 1 or any(w < 1 or w > 4096 for w in workers):
        parser.error("invalid repeats/workers")
    with args.output.open("x") as out, tempfile.TemporaryDirectory(prefix="jr-compiler-") as temporary:
        directory = Path(temporary)
        def record(row):
            out.write(json.dumps(row, sort_keys=True) + "\n")
            out.flush()
        record({"record": "environment", "schema": 1, "revision": revision,
                "binary_sha256": hashlib.sha256(binary.read_bytes()).hexdigest(),
                "machine": platform.platform(), "logical_cpus": os.cpu_count(),
                "topology": run(["lscpu"]).stdout.decode(),
                "measurement": "contended/uncontrolled", "repeats": args.repeats,
                "bootstrap_rustc": run(["rustc", "-Vv"]).stdout.decode(),
                "reference_rustc": str(args.reference_rustc) if args.reference_rustc else None})
        for count in (4, 256):
            source = directory / f"functions-{count}.rs"
            source.write_text(source_for(count))
            if args.reference_rustc:
                reference = directory / f"reference-{count}"
                run([str(args.reference_rustc), "--edition=2024", "--crate-name", "reference", "-C", "overflow-checks=yes", "-o", str(reference), str(source)])
                assert run([str(reference)]).stdout == b"compiled correctly\n"
            for repeat in range(args.repeats):
                # Alternate worker order to reduce fixed-order drift.
                for worker in (workers if repeat % 2 == 0 else workers[::-1]):
                    for mode in ("check", "build"):
                        executable = directory / f"out-{count}-{repeat}-{worker}"
                        command = [str(binary), str(source), "--workers", str(worker)]
                        command += ["--check"] if mode == "check" else ["-o", str(executable)]
                        start = time.perf_counter_ns()
                        result = run(command)
                        wall = time.perf_counter_ns() - start
                        row = json.loads(result.stdout)
                        assert row["capture_complete"] and row["failures"] == 0 and row["events_dropped"] == 0
                        execute_ns = None
                        if mode == "build":
                            assert executable.read_bytes()[:4] == b"\x7fELF"
                            started = time.perf_counter_ns()
                            assert run([str(executable)]).stdout == b"compiled correctly\n"
                            execute_ns = time.perf_counter_ns() - started
                        row.update(record="sample", repeat=repeat, independent_functions=count,
                                   process_wall_ns=wall, execute_process_ns=execute_ns,
                                   source_sha256=hashlib.sha256(source.read_bytes()).hexdigest(),
                                   reference_checked=bool(args.reference_rustc), executable_checked=mode == "build")
                        record(row)
                        print(json.dumps({"functions":count,"workers":worker,"mode":mode,"repeat":repeat,"total_ns":row["total_ns"]}), flush=True)


if __name__ == "__main__":
    main()
