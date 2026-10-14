#!/usr/bin/env python3
"""Bounded local measurements of the hosted-built, actual-source fixture."""
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
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--binary', type=Path, default=ROOT / 'target/release/allowance-tickets-baseline')
    parser.add_argument('--samples', type=int, default=15)
    parser.add_argument('--operations', type=int, default=10000)
    parser.add_argument('--correctness', action='store_true', help='one bounded run of each scenario, no performance claim')
    parser.add_argument('--out', type=Path, default=ROOT / 'results/baseline-local.json')
    parser.add_argument('--compiler', type=Path, default=Path.home() / '.justrust/toolchains/nightly-2026-10-08/bin/rustc')
    args = parser.parse_args()
    assert args.samples > 0 and args.operations > 0
    binary = args.binary.resolve()
    environment = os.environ.copy()
    for name in ('CARGO_MAKEFLAGS', 'MAKEFLAGS', 'MFLAGS'):
        environment.pop(name, None)
    scenarios = ['one', 'multi', 'immediate', 'surplus']
    records = []
    failures = []
    for scenario in scenarios:
        for sample in range(1 if args.correctness or scenario == 'surplus' else args.samples):
            start = time.perf_counter_ns()
            try:
                result = subprocess.run([str(binary), scenario, str(args.operations)], env=environment,
                                        capture_output=True, text=True, timeout=25)
                record = {'scenario_command': scenario, 'sample': sample,
                          'process_elapsed_ns': time.perf_counter_ns() - start,
                          'exit_code': result.returncode, 'stderr': result.stderr}
                if result.returncode:
                    record['stdout'] = result.stdout
                    failures.append(record)
                else:
                    record.update(json.loads(result.stdout))
                    records.append(record)
            except subprocess.TimeoutExpired as error:
                failures.append({'scenario_command': scenario, 'sample': sample, 'timeout_seconds': 25,
                                 'stdout': str(error.stdout), 'stderr': str(error.stderr)})
    summary = {}
    for scenario in scenarios:
        timings = [r['elapsed_ns'] for r in records if r['scenario_command'] == scenario and 'elapsed_ns' in r]
        if timings:
            summary[scenario] = {'samples': len(timings), 'min_ns': min(timings),
                                 'median_ns': statistics.median(timings), 'max_ns': max(timings)}
    compiler = subprocess.run([str(args.compiler), '-Vv'], capture_output=True, text=True, timeout=10)
    stamp = subprocess.run(['readelf', '-p', '.comment', str(binary)], capture_output=True, text=True, timeout=10)
    data = {'scope': 'actual archived Proxy microfixture, no resource identity or broad speed claim',
            'created_unix_ns': time.time_ns(), 'binary': str(binary.relative_to(ROOT)),
            'artifact_sha256': digest(binary), 'compiler_version': compiler.stdout,
            'local_pinned_compiler_binary_sha256': digest(args.compiler), 'artifact_compiler_stamp': stamp.stdout,
            'source_sha256': digest(ROOT / 'archive/baseline-jobserver.rs'),
            'harness_sha256': digest(ROOT / 'src/main.rs'), 'lockfile_sha256': digest(ROOT / 'Cargo.lock'),
            'machine': os.uname()._asdict() if hasattr(os.uname(), '_asdict') else list(os.uname()),
            'cpu_description': Path('/proc/cpuinfo').read_text().split('model name')[1].splitlines()[0].strip(': \t'),
            'operations_per_thread': args.operations, 'samples': records, 'failures': failures, 'summary': summary,
            'blockers': ['surplus callback completion has no acknowledgment in immutable baseline; available() is only a snapshot, not a callback or dequeue oracle']}
    args.out.parent.mkdir(parents=True, exist_ok=True)
    args.out.write_text(json.dumps(data, indent=2) + '\n')
    print(json.dumps({'summary': summary, 'failures': failures, 'out': str(args.out)}, indent=2))
    if failures:
        raise SystemExit(1)

if __name__ == '__main__':
    main()
