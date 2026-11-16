#!/usr/bin/env python3
"""Offline, observational comparison of shipping Summary JSON records.

Schema sources: src/summary.rs (Summary/RemoteInfo/UnitStats/Split),
src/e2e.rs and src/remote_proto.rs. Input is a summary.json or the slim run
record payload, not `runs` index rows. No builds, network, or cache mutation.
Example:
  python3 bench/stable_reuse_attribution.py before.json after.json \
    --baseline-toolchain rustc-commit --candidate-toolchain rustc-commit \
    --baseline-backend llvm --candidate-backend llvm
Identity arguments are explicit caller attestations, not inferred from timing.
"""

import argparse
import json
import math
from pathlib import Path


# Each value is independently reported. These are NOT additive cost buckets.
METRICS = {
    "client_e2e_wall": ("e2e.wall", 1, "client elapsed, includes nested host and sync spans"),
    "client_remote_wall": ("remote.client_wall", 1, "client remote wait, not e2e wall"),
    "cargo_wall": ("wall", 1, "Cargo lifetime on execution machine, not full host lifetime"),
    "source_sync_critical_path": ("remote.sync_ms", .001, "sync remaining on critical path, not total background sync work"),
    "source_sync_lock": ("remote.sync.lock_ms", .001, "nested sync lock wait"),
    "source_sync_connection": ("remote.sync.link_ms", .001, "nested sync connection/reconnection"),
    "source_scan_diff_push": ("remote.sync.push_ms", .001, "nested sync scanning/diffing/sending"),
    "source_sync_barrier": ("remote.sync.barrier_ms", .001, "nested sync apply barrier"),
    "host_prepare": ("remote.run.prepare_ms", .001, "host preflight and scheduling"),
    "host_execute": ("remote.run.execute_ms", .001, "recorder child, contains Cargo lifetime"),
    "host_finalize": ("remote.run.finalize_ms", .001, "host post-build collection"),
    "host_records": ("remote.run.records_ms", .001, "host record packing"),
    "frontend_main_thread_sum": ("units.phase_split.frontend", 1, "sum across covered compiler units, not critical-path wall"),
    "codegen_main_thread_sum": ("units.phase_split.codegen", 1, "includes LLVM IR generation and waiting, not LLVM worker time"),
    "llvm_worker_thread_sum": ("units.llvm_thread_secs", 1, "worker-thread seconds, can exceed wall, ledger coverage required"),
    "link_pass_sum": ("units.phase_split.link", 1, "compiler link phases across covered units"),
    "linker_cpu": ("link.linker_cpu_secs", 1, "linker process CPU, overlaps compiler link phases"),
    "incremental_cache_io_sum": ("units.phase_split.incremental", 1, "incremental cache load/save, NOT query cache lookup or hit timing"),
    "phase_covered_unit_wall_sum": ("units.phase_wall_secs", 1, "coverage denominator, not an extra cost"),
    "rustc_unit_wall_sum": ("units.rustc_wall_sum", 1, "overlapping compiler unit wall sum"),
}


def lookup(record, path):
    value = record
    for part in path.split("."):
        if not isinstance(value, dict) or part not in value:
            return None
        value = value[part]
    return value


def number(value, path):
    if value is None:
        return None
    if isinstance(value, bool) or not isinstance(value, (int, float)) or not math.isfinite(value) or value < 0:
        raise ValueError(f"{path}: expected finite nonnegative number or null")
    return value


def observation(record, path, scale=1, note=""):
    value = number(lookup(record, path), path)
    return {"status": "unknown" if value is None else "recorded", "seconds": None if value is None else value * scale,
            "source": path, "scope": note}


def identity_check(baseline, candidate, baseline_toolchain, candidate_toolchain,
                   baseline_backend, candidate_backend):
    identities = [baseline_toolchain, candidate_toolchain, baseline_backend, candidate_backend]
    if any(not isinstance(x, str) or not x.strip() for x in identities):
        raise ValueError("explicit nonempty toolchain and backend identities required for both runs")
    if baseline_toolchain != candidate_toolchain or baseline_backend != candidate_backend:
        raise ValueError("same compiler/toolchain and backend required")
    for record, asserted in [(baseline, baseline_toolchain), (candidate, candidate_toolchain)]:
        if record.get("toolchain") is not None and record["toolchain"] != asserted:
            raise ValueError("recorded toolchain conflicts with explicit identity")
    # These optional provenance fields are not part of Summary today, but never
    # ignore explicit provenance attached to a run by a caller/exporter.
    for field in ("compiler", "rustc", "backend", "codegen_backend"):
        left, right = baseline.get(field), candidate.get(field)
        if left is not None and right is not None and left != right:
            raise ValueError(f"recorded {field} mismatch")
        if field in ("backend", "codegen_backend"):
            for value in (left, right):
                if value is not None and value != baseline_backend:
                    raise ValueError(f"recorded {field} conflicts with explicit identity")


def describe(record):
    if not isinstance(record, dict) or "id" not in record or "wall" not in record:
        raise ValueError("expected shipping Summary JSON object with id and wall")
    result = {name: observation(record, *spec) for name, spec in METRICS.items()}
    result["query_cache"] = {"status": "unknown", "seconds": None, "source": None,
                             "scope": "Summary has no query-cache timing field; incremental IO is not a substitute"}
    # Default-serialized zero in older UnitStats cannot establish LLVM ledger
    # coverage. No available Summary field proves that worker timing was captured.
    if result["llvm_worker_thread_sum"]["seconds"] == 0:
        result["llvm_worker_thread_sum"].update(status="unknown", seconds=None)
    result["host_wall"] = {"status": "unknown", "seconds": None, "source": None,
                           "scope": "no complete host elapsed wall field; do not relabel Cargo wall or sum host spans"}
    e2e = record.get("e2e")
    warnings = []
    if record.get("exit") not in (None, 0):
        warnings.append("run failed; its duration is not a successful-build comparison")
    if isinstance(e2e, dict):
        if e2e.get("derived") and result["client_e2e_wall"]["seconds"] is not None:
            result["client_e2e_wall"]["status"] = "derived"
            warnings.append("e2e is reconstructed, not directly measured client timing")
        segments = e2e.get("segments")
        if isinstance(segments, list):
            durations = [number(s.get("secs"), "e2e.segments.secs") for s in segments if isinstance(s, dict)]
            wall = number(e2e.get("wall"), "e2e.wall")
            if wall is not None and all(v is not None for v in durations) and not math.isclose(sum(durations), wall, abs_tol=.01, rel_tol=.001):
                warnings.append("e2e segment sum does not match wall; segments not used for attribution")
    return {"id": record["id"], "exit": record.get("exit"), "metrics": result, "warnings": warnings}


def compare(baseline, candidate, baseline_toolchain, candidate_toolchain,
            baseline_backend, candidate_backend):
    before, after = describe(baseline), describe(candidate)
    identity_check(baseline, candidate, baseline_toolchain, candidate_toolchain, baseline_backend, candidate_backend)
    deltas = {}
    for key, left in before["metrics"].items():
        right = after["metrics"][key]
        a, b = left["seconds"], right["seconds"]
        status = "unknown" if a is None or b is None else (
            "derived_difference" if "derived" in (left["status"], right["status"]) else "observed_difference")
        deltas[key] = {"candidate_minus_baseline_seconds": None if a is None or b is None else b - a,
                       "status": status}
    return {"schema_version": 1, "toolchain": baseline_toolchain, "backend": baseline_backend,
            "identity_basis": "caller attestation, checked against recorded provenance where present",
            "baseline": before, "candidate": after, "differences": deltas,
            "limitations": ["Independent nested/parallel observations are never added or subtracted to allocate wall time.",
                            "Recorded means present in JSON, not proof of complete instrumentation; missing/null is unknown.",
                            "Compiler phase sums cover instrumented units only. Top units/passes may be truncated and are not summed.",
                            "One pair is descriptive, not a causal reuse speedup, benchmark gate, or RAM claim.",
                            "Caller must control workload, flags, cache state, hardware and execution mode; toolchain labels must identify the exact compiler.",
                            "Query-cache timings and complete host elapsed wall are unavailable in shipping Summary."]}


def main(argv=None):
    parser = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    parser.add_argument("baseline", type=Path)
    parser.add_argument("candidate", type=Path)
    for label in ("baseline", "candidate"):
        parser.add_argument(f"--{label}-toolchain", required=True)
        parser.add_argument(f"--{label}-backend", required=True)
    args = parser.parse_args(argv)
    try:
        with args.baseline.open() as f:
            baseline = json.load(f)
        with args.candidate.open() as f:
            candidate = json.load(f)
        report = compare(baseline, candidate, args.baseline_toolchain, args.candidate_toolchain,
                         args.baseline_backend, args.candidate_backend)
    except (OSError, ValueError, TypeError) as exc:
        parser.error(str(exc))
    print(json.dumps(report, indent=2, allow_nan=False))


if __name__ == "__main__":
    main()
