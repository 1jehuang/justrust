#!/usr/bin/env python3
"""Small synthetic diagnostics, run INSIDE an already-authorized hosted job.

No job submission, builds of the compiler, installs, or package dependencies.
Required --execution-context hosted is an operator attestation, not detection.
Fresh processes/outputs, warm uncontrolled machine: not real-world evidence.
All shapes get CHECK at workers 1/8/96, baseline/candidate, three interleaved
repeats. BUILD is deliberately restricted to called counts 4/64/128, worker 1.
Separate candidate profiles cover each CHECK configuration and selected BUILDs;
optional allocator profiles cover counts 64/128, worker 1 only. No CFG traces.
A new output directory retains sources, hashes, raw logs, profiles and binaries.
Limits: 60s wall per process, 60s CPU, 8GiB address space, 16MiB per output file,
no core dumps; total wall budget defaults to 1800s. Failures are never retried.
JSONL log excerpts are bounded; raw logs retain errors up to the file limit.
Example (inside hosted job): python3 experiments/compiler_diagnostics.py \\
  --execution-context hosted --baseline /scratch/old --binary /scratch/new \\
  --allocator /scratch/alloc --rustc /usr/bin/rustc --output-dir /scratch/diag
"""

import argparse
import hashlib
import json
import os
from pathlib import Path
import platform
import resource
import shutil
import signal
import subprocess
import time

from compiler import source_for

COUNTS = (4, 16, 32, 64, 128)
WORKERS = (1, 8, 96)
MIB = 1024 * 1024


def identity(path):
    digest = hashlib.sha256()
    with path.open("rb") as stream:
        for block in iter(lambda: stream.read(MIB), b""):
            digest.update(block)
    return {"path": str(path), "bytes": path.stat().st_size,
            "sha256": digest.hexdigest()}


def limits():
    for key, value in ((resource.RLIMIT_CPU, 60), (resource.RLIMIT_AS, 8 * 1024 * MIB),
                       (resource.RLIMIT_FSIZE, 16 * MIB), (resource.RLIMIT_CORE, 0)):
        _, hard = resource.getrlimit(key)
        cap = value if hard == resource.RLIM_INFINITY else min(value, hard)
        resource.setrlimit(key, (cap, cap))


def fixtures():
    for count in COUNTS:
        text = source_for(count)
        yield "called", count, text, "compiled correctly\n"
        yield "independent", count, text.split("fn main(){", 1)[0] + (
            'fn main(){println!("compiled correctly");}'), "compiled correctly\n"
        steps = count * 8
        body = ["fn main(){", "let x0:i32=0;"]
        body.extend(f"let x{i}:i32=x{i-1}+{i};" for i in range(1, steps + 1))
        expected = steps * (steps + 1) // 2
        # Keep all arithmetic observable without requiring formatted println,
        # which is outside the native compiler's supported language subset.
        body.append(f'if x{steps} != {expected} {{ let zero:i32=0; let fail=1/zero; }}')
        body.append('println!("compiled correctly");}')
        yield "main_only", count, "\n".join(body), "compiled correctly\n"


class Runner:
    def __init__(self, directory, budget):
        self.directory = directory
        self.deadline = time.monotonic() + budget
        self.sequence = 0
        self.failures = 0
        self.output = (directory / "results.jsonl").open("x")

    def emit(self, row):
        self.output.write(json.dumps(row, sort_keys=True) + "\n")
        self.output.flush()

    def run(self, argv):
        remaining = self.deadline - time.monotonic()
        if remaining <= 0:
            raise TimeoutError("total wall budget exhausted")
        self.sequence += 1
        prefix = self.directory / f"process-{self.sequence:04}"
        row = {"argv": [str(x) for x in argv], "exit": None, "timeout": False}
        started = time.perf_counter_ns()
        stdout, stderr = Path(str(prefix) + ".stdout"), Path(str(prefix) + ".stderr")
        with stdout.open("xb") as out, stderr.open("xb") as err:
            try:
                process = subprocess.Popen(row["argv"], stdin=subprocess.DEVNULL,
                                           stdout=out, stderr=err, shell=False,
                                           start_new_session=True, preexec_fn=limits)
                try:
                    row["exit"] = process.wait(timeout=min(60, remaining))
                except subprocess.TimeoutExpired:
                    row["timeout"] = True
                    os.killpg(process.pid, signal.SIGKILL)
                    row["exit"] = process.wait()
            except (OSError, subprocess.SubprocessError) as error:
                row["error"] = str(error)
        row["wall_ns"] = time.perf_counter_ns() - started
        for key, path in (("stdout", stdout), ("stderr", stderr)):
            with path.open("rb") as stream:
                row[key] = stream.read(65536).decode(errors="replace")
            row[key + "_artifact"] = identity(path)
            row[key + "_truncated"] = path.stat().st_size > 65536
        return row

    def sample(self, case, tool, binary, workers, mode, repeat, profile=False):
        label = f'{case["shape"]}-{case["count"]}-{tool}-{workers}-{mode}-{repeat}'
        artifact = self.directory / (label + ".elf")
        profile_path = self.directory / (label + ".profile.json")
        argv = [binary, case["source"]["path"]]
        if tool == "rustc":
            argv += ["--edition=2024", "--crate-name", "diagnostic", "-C", "opt-level=0",
                     "-C", "overflow-checks=yes", "-o", artifact]
        else:
            argv += ["--workers", str(workers)]
            argv += ["--check"] if mode == "check" else ["-o", artifact]
            if profile:
                argv += ["--profile", profile_path]
        row = {"record": "sample", **case, "tool": tool, "workers": workers,
               "mode": mode, "repeat": repeat, "profile_enabled": profile,
               "compile": self.run(argv), "artifact": None, "correct": False}
        if artifact.exists():
            row["artifact"] = identity(artifact)
        if row["compile"]["exit"] == 0 and not row["compile"]["timeout"]:
            if mode == "check":
                row["correct"] = not artifact.exists()
            elif artifact.exists():
                row["runtime"] = self.run([artifact])
                run = row["runtime"]
                row["correct"] = (run["exit"] == 0 and not run["timeout"] and
                                  run["stdout"] == case["expected_stdout"] and
                                  not run["stdout_truncated"] and
                                  run["stderr_artifact"]["bytes"] == 0)
        if profile:
            row["profile_artifact"] = identity(profile_path) if profile_path.exists() else None
            try:
                if not profile_path.exists() or profile_path.stat().st_size > MIB:
                    raise ValueError("profile absent or exceeds 1MiB JSON bound")
                with profile_path.open() as stream:
                    value = json.load(stream)
                if not isinstance(value, dict):
                    raise ValueError("profile must be a JSON object")
                row["profile"] = value
            except (OSError, ValueError, RecursionError) as error:
                row["profile_error"] = str(error)
                row["correct"] = False
        self.failures += not row["correct"]
        self.emit(row)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--baseline", required=True)
    parser.add_argument("--binary", required=True)
    parser.add_argument("--allocator", help="optional separately instrumented binary")
    parser.add_argument("--rustc", required=True)
    parser.add_argument("--output-dir", required=True, type=Path)
    parser.add_argument("--execution-context", required=True, choices=["hosted"])
    parser.add_argument("--budget-seconds", type=int, default=1800)
    args = parser.parse_args()
    if args.budget_seconds < 1 or platform.system() != "Linux":
        parser.error("requires Linux and a positive total wall budget")
    binaries = {}
    for name in ("baseline", "binary", "allocator", "rustc"):
        if getattr(args, name):
            found = shutil.which(getattr(args, name))
            if not found:
                parser.error(f"{name}: executable not found")
            binaries[name] = Path(found).resolve(strict=True)
    directory = args.output_dir.resolve()
    directory.mkdir(parents=True, exist_ok=False)
    runner = Runner(directory, args.budget_seconds)
    runner.emit({"record": "environment", "schema": 1, "machine": platform.platform(),
                 "logical_cpus": os.cpu_count(), "execution_context": args.execution_context,
                 "binaries": {k: identity(v) for k, v in binaries.items()},
                 "generator": identity(Path(__file__).with_name("compiler.py")),
                 "runner": identity(Path(__file__)), "counts": COUNTS, "workers": WORKERS,
                 "repeats": 3, "budget_seconds": args.budget_seconds,
                 "limits": {"wall_seconds": 60, "cpu_seconds": 60, "address_bytes": 8 * 1024 * MIB,
                            "file_bytes": 16 * MIB, "profile_json_bytes": MIB},
                 "measurement": "synthetic fresh-process, warm-machine, uncontrolled contention; "
                 "no large-real-world representativeness or aggregate speedup claim"})
    cases = []
    for shape, count, text, expected in fixtures():
        source = directory / f"{shape}-{count}.rs"
        source.write_text(text, encoding="utf-8")
        case = {"shape": shape, "count": count, "source": identity(source),
                "expected_stdout": expected}
        cases.append(case)
        runner.emit({"record": "source", **case})
    complete = False
    try:
        for case in cases:
            runner.sample(case, "rustc", binaries["rustc"], 1, "build", "reference")
        # Repeat outermost; rotate case and worker order and alternate tool order.
        for repeat in range(3):
            ordered = cases[repeat:] + cases[:repeat]
            for case in ordered:
                for workers in WORKERS[repeat:] + WORKERS[:repeat]:
                    modes = ["check"]
                    if case["shape"] == "called" and case["count"] in (4, 64, 128) and workers == 1:
                        modes.append("build")
                    for mode in modes:
                        tools = ("baseline", "binary") if repeat % 2 == 0 else ("binary", "baseline")
                        for tool in tools:
                            runner.sample(case, tool, binaries[tool], workers, mode, repeat)
        # Diagnostic profiles are separate from all no-profile timing comparisons.
        for case in cases:
            for workers in WORKERS:
                modes = ["check"]
                if case["shape"] == "called" and case["count"] in (4, 64, 128) and workers == 1:
                    modes.append("build")
                for mode in modes:
                    runner.sample(case, "binary", binaries["binary"], workers, mode, "profile", True)
                    if "allocator" in binaries and case["count"] in (64, 128) and workers == 1:
                        runner.sample(case, "allocator", binaries["allocator"], 1, mode, "profile", True)
        complete = True
    except TimeoutError as error:
        runner.emit({"record": "budget_exhausted", "error": str(error)})
    finally:
        runner.emit({"record": "summary", "complete": complete,
                     "failed_samples": runner.failures, "processes": runner.sequence})
        runner.output.close()
    return 2 if not complete else int(runner.failures > 0)


if __name__ == "__main__":
    raise SystemExit(main())
