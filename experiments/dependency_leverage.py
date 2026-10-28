#!/usr/bin/env python3
"""Controlled dependency breadth observations, not proof or speedup evidence.

Example: python3 experiments/dependency_leverage.py --binary /path/jr-rust
    --output /scratch/dependency-leverage --workers 1 8
"""
import argparse
from datetime import datetime, timezone
import hashlib
import json
import os
from pathlib import Path
import platform
import shutil
import subprocess
import sys
import time

# First callable has item ordinal zero. Only these item-keyed operations are
# selected, never file-keyed nodes that happen to share the same parameter.
TARGET_KINDS = (102, 103, 108, 109, 110, 111, 112, 116, 126, 130, 131, 132, 133)
BREADTH = ("direct_dependencies", "direct_consumers", "transitive_dependencies",
           "transitive_consumers")
TIMING = ("summed_wall_ns", "summed_wait_ns")
TARGET = "fn target(x: i32) -> i32 { x * 3 + 7 }"
MAIN = 'fn main() { if target(5) != 22 { fail(0); } println!("dependency probe ok"); }'
FAIL = 'fn fail(x: i32) { let bad = 1 / x; }'


def sources(extra_functions):
    def unrelated(index, changed=False):
        return f"fn unrelated_{index}(x: i32) -> i32 {{ x + {3 if changed else 2} }}"
    base = [TARGET, MAIN, FAIL, unrelated(0)]
    return {
        "base": "\n".join(base) + "\n",
        "unrelated-body-edit": "\n".join([TARGET, MAIN, FAIL, unrelated(0, True)]) + "\n",
        "unrelated-declarations-added": "\n".join(base + [unrelated(i) for i in range(1, extra_functions + 1)]) + "\n",
    }


def stable_key(row):
    return row["kind"], row["version"], tuple(row["params"])


def leverage(profile):
    return profile.get("graph", {}).get("dependency_leverage")


def compare_profiles(base, changed):
    """Match kind+version+params. Missing/bounded observations stay null."""
    left, right = leverage(base), leverage(changed)
    def index(section):
        result = {}
        for row in (section or {}).get("top_nodes", []):
            key = stable_key(row)
            if key in result:
                raise ValueError(f"duplicate stable key: {key}")
            result[key] = row
        return result
    a, b = index(left), index(right)
    observations = []
    selected = [(kind, (0,)) for kind in TARGET_KINDS] + [(101, ()), (106, ())]
    for kind, params in selected:
        versions = sorted({key[1] for key in a.keys() | b.keys()
                           if key[0] == kind and key[2] == params}) or [1]
        for version in versions:
            observations.append(compare_node(kind, version, params, a, b, left, right))
    def coverage(section):
        return None if section is None else {k: v for k, v in section.items() if k != "top_nodes"}
    return {"base_coverage": coverage(left), "variant_coverage": coverage(right),
            "target_nodes": [row for row in observations if row["params"]],
            "global_nodes": [row for row in observations if not row["params"]]}


def compare_node(kind, version, params, a, b, left, right):
    key = (kind, version, params)
    before, after = a.get(key), b.get(key)
    row = {"kind": kind, "version": version, "params": list(params),
           "available": before is not None and after is not None,
           "before": None, "after": None, "deltas": {}}
    for label, value in (("before", before), ("after", after)):
        if value is not None:
            row[label] = {name: value.get(name) for name in BREADTH + TIMING + ("closure_complete",)}
    for name in BREADTH + TIMING:
        values = [node.get(name) if node is not None else None for node in (before, after)]
        complete = all(section is not None and section.get(
            "intervals_complete" if name in TIMING else "edge_attribution_complete") is True
            for section in (left, right))
        if name.startswith("transitive_"):
            complete = complete and all(node is not None and node.get("closure_complete") is True
                                        for node in (before, after))
        row["deltas"][name] = values[1] - values[0] if complete and all(
            isinstance(v, (int, float)) and not isinstance(v, bool) for v in values) else None
    return row


def telemetry_parity(unprofiled, profiled):
    fields = ("nodes", "edges", "failures", "events_dropped", "capture_complete")
    for telemetry in (unprofiled, profiled):
        if telemetry.get("capture_complete") is not True or telemetry.get("failures") != 0 or telemetry.get("events_dropped") != 0:
            raise ValueError("native telemetry is incomplete or failed")
        if any(name not in telemetry for name in fields):
            raise ValueError("native telemetry missing baseline counters")
    mismatches = [name for name in fields if unprofiled[name] != profiled[name]]
    if mismatches:
        raise ValueError(f"profiled/unprofiled telemetry mismatch: {mismatches}")
    return {name: unprofiled[name] for name in fields}


def positive(value):
    try:
        number = int(value)
    except ValueError as error:
        raise argparse.ArgumentTypeError("must be a positive integer") from error
    if number < 1:
        raise argparse.ArgumentTypeError("must be a positive integer")
    return number


def parser():
    result = argparse.ArgumentParser(description=__doc__)
    result.add_argument("--binary", type=Path, required=True)
    result.add_argument("--output", type=Path, required=True, help="new directory outside the repository")
    result.add_argument("--workers", type=positive, nargs="+", default=[1, 8])
    result.add_argument("--extra-functions", type=positive, default=16)
    result.add_argument("--repeats", type=positive, default=1)
    return result


def run(command, directory, label, records):
    command = [str(x) for x in command]
    start = time.perf_counter_ns()
    try:
        process = subprocess.run(command, capture_output=True, timeout=180)
    except subprocess.TimeoutExpired as error:
        (directory / (label + ".stdout")).write_bytes(error.stdout or b"")
        (directory / (label + ".stderr")).write_bytes(error.stderr or b"")
        raise RuntimeError(f"command timed out: {command}") from error
    elapsed = time.perf_counter_ns() - start
    (directory / (label + ".stdout")).write_bytes(process.stdout)
    (directory / (label + ".stderr")).write_bytes(process.stderr)
    records.append({"command": command, "wall_ns": elapsed, "returncode": process.returncode,
                    "stdout": label + ".stdout", "stderr": label + ".stderr"})
    if process.returncode:
        raise RuntimeError(f"command failed ({process.returncode}): {command}\n"
                           + process.stdout.decode(errors="replace") + process.stderr.decode(errors="replace"))
    return process


def git_text(repository, *args):
    result = subprocess.run(["git", "-C", str(repository), *args], capture_output=True, text=True)
    return {"returncode": result.returncode, "stdout": result.stdout, "stderr": result.stderr}


def main(argv=None):
    cli = parser()
    args = cli.parse_args(argv)
    repository = Path(__file__).resolve().parents[1]
    binary = args.binary.resolve()
    output = args.output.resolve()
    if not binary.is_file() or not os.access(binary, os.X_OK):
        cli.error("--binary must be an executable file")
    if output == repository or repository in output.parents:
        cli.error("--output must be outside the repository")
    if output.exists():
        cli.error("--output must be a new directory")
    rustc = shutil.which("rustc")
    if rustc is None:
        cli.error("stock rustc is required on PATH")
    output.mkdir(parents=True)
    records = []
    report = {
        "schema": "dependency-leverage-v1", "started_utc": datetime.now(timezone.utc).isoformat(),
        "binary": str(binary), "binary_sha256": hashlib.sha256(binary.read_bytes()).hexdigest(),
        "repository_revision": git_text(repository, "rev-parse", "HEAD"),
        "repository_status": git_text(repository, "status", "--porcelain=v1"),
        "conditions": {"workers": args.workers, "extra_functions": args.extra_functions,
                       "repeats": args.repeats, "host": platform.node(), "platform": platform.platform(),
                       "cpu_count": os.cpu_count(), "load_start": os.getloadavg(),
                       "order": "repeat, workers, base/body-edit/declarations-added, native unprofiled/profiled check/build/run, stock check/build/run",
                       "cache": "fresh processes, no eviction, no incremental flags, no warmups",
                       "timing": "observed process wall including profile serialization and output capture, excluding log writes, uncontrolled shared-host contention"},
        "limitations": ["Synthetic supported i32 arithmetic only, not representative Rust performance.",
                        "No unnecessary-dependency proof or before/after speed claim.",
                        "Missing bounded top-node rows are unavailable, never zero.",
                        "Target selection is explicit item-keyed kinds with params [0], not all target facts.",
                        "Zero breadth deltas do not prove independence. Identical keys may hold changed payloads.",
                        "Fresh processes do not test incremental/cache invalidation.",
                        "summed_wait_ns is the node's own suspended wall, not producer-caused time or critical path.",
                        "Git metadata describes the checkout, not proof of binary provenance."],
        "commands": records, "comparisons": [], "telemetry_parity": [], "success": False,
    }
    try:
        report["rustc_version"] = run([rustc, "-Vv"], output, "rustc-version", records).stdout.decode()
        fixtures = sources(args.extra_functions)
        report["sources"] = {}
        for name, text in fixtures.items():
            (output / (name + ".rs")).write_text(text)
            report["sources"][name] = {"sha256": hashlib.sha256(text.encode()).hexdigest(), "bytes": len(text.encode())}
        for repeat in range(args.repeats):
            for workers in args.workers:
                profiles = {}
                for name in fixtures:
                    prefix = f"r{repeat}-w{workers}-{name}"
                    source = output / (name + ".rs")
                    for tool in ("native", "stock"):
                        for mode in ("check", "build"):
                            label = f"{prefix}-{tool}-{mode}"
                            executable = output / (label + ".bin")
                            if tool == "native":
                                profile = output / (label + ".profile.json")
                                command = [binary, source, "--workers", workers, "--profile", profile]
                                command += ["--check"] if mode == "check" else ["-o", executable]
                            else:
                                command = [rustc, source, "--edition=2024", "--crate-name", "dependency_probe",
                                           "-C", "opt-level=0", "-C", "overflow-checks=yes", "-o", executable]
                                if mode == "check":
                                    command += ["--emit=metadata"]
                            if tool == "native":
                                baseline_command = [binary, source, "--workers", workers]
                                baseline_command += ["--check"] if mode == "check" else ["-o", output / (label + "-unprofiled.bin")]
                                baseline = run(baseline_command, output, label + "-unprofiled", records)
                            result = run(command, output, label, records)
                            if tool == "native":
                                report["telemetry_parity"].append({"label": label, "matched_counters": telemetry_parity(
                                    json.loads(baseline.stdout), json.loads(result.stdout))})
                                data = json.loads(profile.read_text())
                                if data.get("success") is not True:
                                    raise RuntimeError(f"native profile did not report success: {profile}")
                                profiles[(name, mode)] = data
                            if mode == "build":
                                executed = run([executable], output, label + "-run", records)
                                if executed.stdout != b"dependency probe ok\n" or executed.stderr:
                                    raise RuntimeError(f"unexpected executable output: {label}")
                                if tool == "native":
                                    baseline_run = run([output / (label + "-unprofiled.bin")],
                                                       output, label + "-unprofiled-run", records)
                                    if (baseline_run.stdout, baseline_run.stderr) != (executed.stdout, executed.stderr):
                                        raise RuntimeError(f"profiling changed executable behavior: {label}")
                for name in fixtures:
                    if name == "base":
                        continue
                    for mode in ("check", "build"):
                        report["comparisons"].append({"repeat": repeat, "workers": workers,
                            "variant": name, "mode": mode,
                            **compare_profiles(profiles[("base", mode)], profiles[(name, mode)])})
        report["profile_comparisons_available"] = sum(
            any(row["available"] for row in comparison["target_nodes"])
            for comparison in report["comparisons"])
        report["success"] = True
    except (OSError, RuntimeError, ValueError, KeyError) as error:
        report["error"] = str(error)
    finally:
        report["conditions"]["load_end"] = os.getloadavg()
        (output / "summary.json").write_text(json.dumps(report, indent=2) + "\n")
    print(str(output / "summary.json"))
    if not report["success"]:
        print(report["error"], file=sys.stderr)
        return 1
    return 0


if __name__ == "__main__":
    sys.exit(main())
