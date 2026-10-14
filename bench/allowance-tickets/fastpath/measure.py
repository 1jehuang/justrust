#!/usr/bin/env python3
"""Bounded local measurements of hosted-built actual-source Proxy fixtures."""
import argparse
import hashlib
import json
import os
from pathlib import Path
import statistics
import subprocess
import time

ROOT = Path(__file__).resolve().parent

def digest(path):
    return hashlib.sha256(path.read_bytes()).hexdigest()

def main():
    p = argparse.ArgumentParser(description=__doc__)
    p.add_argument('--binary', type=Path, required=True)
    p.add_argument('--variant', choices=['before-disabled', 'after-disabled', 'before-enabled', 'after-enabled'], default='before-disabled')
    p.add_argument('--pending-rounds', type=int, default=200)
    p.add_argument('--event-capacity', type=int, default=4096)
    p.add_argument('--samples', type=int, default=15)
    p.add_argument('--operations', type=int, default=10000)
    p.add_argument('--correctness', action='store_true')
    p.add_argument('--out', type=Path, required=True)
    p.add_argument('--compiler', type=Path, default=Path.home() / '.justrust/toolchains/nightly-2026-10-08/bin/rustc')
    args = p.parse_args()
    assert args.samples > 0 and args.operations > 0 and args.event_capacity > 0
    stage, mode = args.variant.split('-')
    candidate = True
    source = ROOT / 'archive' / stage / 'jobserver.rs'
    binary = args.binary.resolve()
    env = os.environ.copy()
    for name in ('CARGO_MAKEFLAGS', 'MAKEFLAGS', 'MFLAGS', 'ALLOWANCE_CAPTURE_CAPACITY'):
        env.pop(name, None)
    if candidate:
        env['ALLOWANCE_CAPTURE_CAPACITY'] = str(args.event_capacity if args.variant.endswith('enabled') else 0)
    scenarios = ['one', 'multi', 'immediate', 'surplus', 'pending-one', 'pending-multi']
    records, failures = [], []
    for scenario in scenarios:
        for sample in range(1 if args.correctness or scenario == 'surplus' else args.samples):
            start = time.perf_counter_ns()
            case_operations = args.pending_rounds if scenario.startswith('pending-') else args.operations
            record = {'scenario_command': scenario, 'sample': sample, 'load_average_at_start': list(os.getloadavg()), 'cpu_affinity': sorted(os.sched_getaffinity(0))}
            try:
                result = subprocess.run([str(binary), scenario, str(case_operations)], env=env,
                                        capture_output=True, text=True, timeout=25)
                record.update(process_elapsed_ns=time.perf_counter_ns() - start,
                              exit_code=result.returncode, stderr=result.stderr)
                if result.returncode:
                    record['stdout'] = result.stdout
                    failures.append(record)
                    continue
                try:
                    for line in result.stdout.splitlines():
                        record.update(json.loads(line))
                    expected_variant = stage
                    assert record['variant'] == expected_variant, 'binary does not match requested variant'
                    expected_capture = args.variant.split('-')[-1] if candidate else 'not-applicable'
                    assert record['capture'] == expected_capture, 'capture mode mismatch'
                    expected_pairs = 8 * case_operations if scenario in ['one', 'multi', 'pending-one', 'pending-multi'] else (2 if scenario == 'surplus' else case_operations)
                    assert record['completed_pairs'] == expected_pairs
                    if scenario in ['one', 'multi', 'pending-one', 'pending-multi']:
                        assert record['peak_active'] <= record['capacity'] and record['operations'] == expected_pairs
                    if 'elapsed_ns' in record:
                        record['throughput_pairs_per_second'] = expected_pairs * 1e9 / record['elapsed_ns']
                    records.append(record)
                except (ValueError, KeyError, AssertionError) as error:
                    record.update(stdout=result.stdout, parse_or_contract_error=str(error))
                    failures.append(record)
            except subprocess.TimeoutExpired as error:
                record.update(timeout_seconds=25, stdout=str(error.stdout), stderr=str(error.stderr))
                failures.append(record)
    summary = {}
    for scenario in scenarios:
        matched = [r for r in records if r['scenario_command'] == scenario and 'elapsed_ns' in r]
        if matched:
            timings = [r['elapsed_ns'] for r in matched]
            summary[scenario] = {'samples': len(timings), 'min_ns': min(timings),
                                 'median_ns': statistics.median(timings), 'max_ns': max(timings),
                                 'median_throughput_pairs_per_second': statistics.median(r['throughput_pairs_per_second'] for r in matched)}
    compiler = subprocess.run([str(args.compiler), '-Vv'], capture_output=True, text=True, timeout=10)
    stamp = subprocess.run(['readelf', '-p', '.comment', str(binary)], capture_output=True, text=True, timeout=10)
    data = {'scope': 'actual archived Proxy microfixture, no resource identity, lease or broad speed claim',
            'variant': args.variant, 'capture_event_capacity': args.event_capacity if args.variant.endswith('enabled') else 0,
            'created_unix_ns': time.time_ns(), 'binary': os.path.relpath(binary, ROOT),
            'artifact_sha256': digest(binary), 'compiler_version': compiler.stdout,
            'local_pinned_compiler_binary_sha256': digest(args.compiler), 'artifact_compiler_stamp': stamp.stdout,
            'source_sha256': digest(source), 'harness_sha256': digest(ROOT / 'src/main.rs'),
            'lockfile_sha256': digest(ROOT / 'Cargo.lock'), 'runner_sha256': digest(Path(__file__)),
            'machine': list(os.uname()),
            'cpu_description': Path('/proc/cpuinfo').read_text().split('model name')[1].splitlines()[0].strip(': \t'),
            'operations_per_thread': args.operations, 'pending_rounds': args.pending_rounds, 'quiet_status': 'unknown', 'affinity_policy': 'unchanged inherited affinity, not pinned', 'samples': records, 'failures': failures, 'summary': summary,
            'blockers': [] if candidate else ['immutable baseline has no helper callback acknowledgment; available() snapshot is not a completion oracle']}
    if candidate:
        data['sibling_source_sha256'] = digest(ROOT / 'archive' / stage / 'jobserver_ticketed.rs')
    args.out.parent.mkdir(parents=True, exist_ok=True)
    args.out.write_text(json.dumps(data, indent=2) + '\n')
    print(json.dumps({'variant': args.variant, 'summary': summary, 'failures': failures, 'out': str(args.out)}, indent=2))
    if failures:
        raise SystemExit(1)

if __name__ == '__main__':
    main()
