#!/usr/bin/env python3
"""Verified warm-page-cache, fresh-process Rust single-program benchmark.

No cold-cache or incremental-build claims are made. Native currently has no
optimizer comparable to stock rustc -O. All timings include process startup.
"""
from __future__ import annotations

import argparse
from datetime import datetime, timezone
import hashlib
import json
import os
from pathlib import Path
import platform
import shutil
import socket
import statistics
import subprocess
import tempfile
import time
import zlib

ROOT = Path(__file__).resolve().parents[1]
CONFIGS = ("native", "stock-debug", "stock-release")


def positive_int(value: str) -> int:
    number = int(value)
    if number <= 0:
        raise argparse.ArgumentTypeError("must be a positive integer")
    return number


def positive_timeout(value: str) -> float:
    number = float(value)
    if not 0 < number < float("inf"):
        raise argparse.ArgumentTypeError("must be finite and positive")
    return number


def mandelbrot_oracle() -> bytes:
    rows = []
    for y in range(40):
        row = []
        for x in range(120):
            cr = float(x) * 3.0 / 120.0 - 2.0
            ci = float(y) * 2.0 / 40.0 - 1.0
            zr = zi = 0.0
            iteration = 0
            while zr * zr + zi * zi <= 4.0 and iteration < 200:
                next_zr = zr * zr - zi * zi + cr
                zi = 2.0 * zr * zi + ci
                zr = next_zr
                iteration += 1
            row.append("@" if iteration == 200 else "#" if iteration >= 32
                       else "+" if iteration >= 16 else "." if iteration >= 8 else " ")
        rows.append("".join(row) + "\n")
    return "".join(rows).encode("ascii")


def expected_stdout(oracle):
    if oracle == "adler2-ff5553":
        checksum = zlib.adler32(bytes([255]) * 5553)
        return f"ff_5553:{checksum}\n".encode("ascii")
    if oracle == "mandelbrot":
        return mandelbrot_oracle()
    raise ValueError(f"unknown oracle: {oracle}")


def cpu_model():
    cpuinfo = Path("/proc/cpuinfo")
    if cpuinfo.exists():
        for line in cpuinfo.read_text().splitlines():
            key, separator, value = line.partition(":")
            if separator and key.strip() in ("model name", "Hardware"):
                return value.strip()
    return platform.processor()


def rotate(values, iteration):
    offset = iteration % len(values)
    return values[offset:] + values[:offset]


def summary(samples):
    return {"samples_ns": samples, "count": len(samples),
            "median_ns": statistics.median(samples) if samples else None,
            "min_ns": min(samples) if samples else None,
            "max_ns": max(samples) if samples else None}


def sha256(path):
    digest = hashlib.sha256()
    with Path(path).open("rb") as stream:
        for block in iter(lambda: stream.read(1024 * 1024), b""):
            digest.update(block)
    return digest.hexdigest()


def compiler_command(config, binary, rustc, source, executable, workers):
    if config == "native":
        return [binary, str(source), "--workers", str(workers),
                "--debug-assertions", "yes", "-o", str(executable)]
    return [rustc, "--edition=2024", "-C", "opt-level=" +
            ("0" if config == "stock-debug" else "3"),
            "-C", "overflow-checks=yes", "-C", "debug-assertions=yes",
            "-C", "panic=abort", str(source), "-o", str(executable)]


class Recorder:
    def __init__(self, replacements, timeout):
        self.replacements = sorted(replacements, key=lambda pair: len(pair[0]), reverse=True)
        self.timeout = timeout
        self.events = []

    def sanitize(self, value):
        if isinstance(value, str):
            for original, replacement in self.replacements:
                value = value.replace(original, replacement)
            return value
        if isinstance(value, list):
            return [self.sanitize(item) for item in value]
        if isinstance(value, dict):
            return {key: self.sanitize(item) for key, item in value.items()}
        return value

    def run(self, command, phase, config=None, expected=None, timed=False):
        event = {"phase": phase, "config": config, "command": command,
                 "timed": timed, "ok": False}
        start = time.perf_counter_ns()
        stdout = stderr = b""
        try:
            result = subprocess.run(command, capture_output=True, timeout=self.timeout,
                                    cwd=ROOT, check=False)
            elapsed = time.perf_counter_ns() - start
            stdout, stderr = result.stdout, result.stderr
            event["returncode"] = result.returncode
            event["ok"] = result.returncode == 0 and (expected is None or stdout == expected)
            if result.returncode == 0 and expected is not None and stdout != expected:
                event["error"] = "complete stdout does not match independent oracle"
        except subprocess.TimeoutExpired as error:
            elapsed = time.perf_counter_ns() - start
            stdout, stderr = error.stdout or b"", error.stderr or b""
            event.update(error="timeout", returncode=None)
        except OSError as error:
            elapsed = time.perf_counter_ns() - start
            event.update(error=str(error), returncode=None)
        event.update(stdout=stdout.decode("utf-8", errors="backslashreplace"),
                     stderr=stderr.decode("utf-8", errors="backslashreplace"))
        if timed:
            event["elapsed_ns"] = elapsed
        self.events.append(event)
        return event


def parser():
    result = argparse.ArgumentParser(description=__doc__)
    result.add_argument("source", nargs="?", type=Path,
                        default=ROOT / "experiments/programs/mandelbrot.rs")
    result.add_argument("--binary", required=True, help="native jr-rust CLI executable")
    result.add_argument("--rustc", default="rustc")
    result.add_argument("--compiler-revision", help="revision used to build the supplied native compiler")
    result.add_argument("--oracle", choices=("mandelbrot", "adler2-ff5553"), default="mandelbrot",
                        help="independent expected stdout calculation; pass source explicitly for Adler32")
    result.add_argument("--output", required=True, type=Path,
                        help="new JSON file, never overwritten")
    result.add_argument("--repeats", type=positive_int, default=5)
    result.add_argument("--runtime-repeats", type=positive_int, default=7)
    result.add_argument("--timeout", type=positive_timeout, default=120)
    result.add_argument("--workers", type=positive_int, default=1)
    result.add_argument("--scratch-dir", type=Path)
    return result


def resolve_binary(value):
    found = shutil.which(value)
    # Keep symlink names: rustup dispatches by the invoked rustc basename.
    return str(Path(found or value).absolute())


def benchmark(args, report):
    scratch = args.scratch_dir or Path(os.environ.get(
        "JCODE_SCRATCH_DIR", str(ROOT / "target/real-program-bench-scratch")))
    scratch.mkdir(parents=True, exist_ok=True)
    source = args.source.resolve()
    binary, rustc = resolve_binary(args.binary), resolve_binary(args.rustc)
    with tempfile.TemporaryDirectory(prefix="real-program-", dir=scratch) as directory:
        recorder = Recorder([(str(Path(directory).resolve()), "<scratch>"),
                             (str(ROOT), "<repo>"),
                             (str(Path.home()), "<home>")], args.timeout)
        report["events"] = recorder.events
        report["metadata"] = {
            "recorded_at": datetime.now(timezone.utc).isoformat(),
            "runner_sha256": sha256(__file__),
            "compiler_revision": args.compiler_revision,
            "oracle": args.oracle,
            "source": str(source), "source_sha256": sha256(source),
            "binary": binary, "binary_sha256": sha256(binary),
            "rustc": rustc, "hostname": socket.gethostname(),
            "platform": platform.platform(), "cpu": cpu_model(),
            "cpu_count": os.cpu_count(),
            "cpu_affinity": sorted(os.sched_getaffinity(0)) if hasattr(os, "sched_getaffinity") else None,
            "workers": args.workers,
            "repeats": args.repeats, "runtime_repeats": args.runtime_repeats,
            "timeout_seconds": args.timeout,
        }
        for name, command in (("rustc_verbose_version", [rustc, "--version", "--verbose"]),
                              ("git_revision", ["git", "rev-parse", "HEAD"]),
                              ("git_dirty_names", ["git", "status", "--porcelain", "--untracked-files=all"])):
            report["metadata"][name] = recorder.run(command, "metadata")
        expected = expected_stdout(args.oracle)
        report["oracle_sha256"] = hashlib.sha256(expected).hexdigest()
        results = {name: {"compile": summary([]), "runtime": summary([]),
                          "executable_size_bytes": None} for name in CONFIGS}
        report["results"] = results
        latest = {}

        def compile_and_verify(config, phase, iteration, timed):
            executable = Path(directory) / f"{config}-{phase}-{iteration}"
            command = compiler_command(config, binary, rustc, source, executable, args.workers)
            event = recorder.run(command, phase + "-compile", config, timed=timed)
            if not event["ok"]:
                return False
            if not executable.is_file():
                event.update(ok=False, error="compiler did not produce executable")
                return False
            validation = recorder.run([str(executable)], phase + "-validate", config,
                                      expected=expected)
            if not validation["ok"]:
                return False
            latest[config] = executable
            results[config]["executable_size_bytes"] = executable.stat().st_size
            if timed:
                samples = results[config]["compile"]["samples_ns"] + [event["elapsed_ns"]]
                results[config]["compile"] = summary(samples)
            return True

        try:
            warmups = [compile_and_verify(config, "warmup", 0, False) for config in CONFIGS]
            if not all(warmups):
                report["error"] = "warmup failed; no measured samples collected"
                return 1
            for iteration in range(args.repeats):
                for config in rotate(CONFIGS, iteration):
                    compile_and_verify(config, "measurement", iteration, True)
            for iteration in range(args.runtime_repeats):
                for config in rotate(CONFIGS, iteration):
                    event = recorder.run([str(latest[config])], "runtime", config,
                                         expected=expected, timed=True)
                    if event["ok"]:
                        samples = results[config]["runtime"]["samples_ns"] + [event["elapsed_ns"]]
                        results[config]["runtime"] = summary(samples)
            return 0 if all(event["ok"] for event in recorder.events
                            if event["phase"] != "metadata") else 1
        finally:
            sanitized = recorder.sanitize(report)
            report.clear()
            report.update(sanitized)


def main(argv=None):
    args = parser().parse_args(argv)
    report = {"schema_version": 1, "ok": False,
              "measurement": "warm-page-cache fresh processes; not cold or incremental; "
                             "subprocess timings include process startup",
              "comparability": "native has no optimizer comparable to stock O3",
              "runtime_caveat": "Mandelbrot runtime includes per-pixel print IO; stock and native "
                                "runtime libraries differ, so this is not a pure compute/codegen benchmark"}
    try:
        output = args.output.open("x", encoding="utf-8")
    except OSError as error:
        parser().error(str(error))
    status = 1
    with output:
        try:
            status = benchmark(args, report)
            report["ok"] = status == 0
        except (OSError, ValueError) as error:
            report["error"] = str(error).replace(str(ROOT), "<repo>").replace(str(Path.home()), "<home>")
        finally:
            sanitized = Recorder([(str(ROOT), "<repo>"),
                                  (str(Path.home()), "<home>")], args.timeout).sanitize(report)
            json.dump(sanitized, output, indent=2)
            output.write("\n")
    return status


if __name__ == "__main__":
    raise SystemExit(main())
