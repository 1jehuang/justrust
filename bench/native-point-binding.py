#!/usr/bin/env python3
"""Paired native point-capture diagnostic, not a session-weighted speed gate."""
import argparse
import hashlib
import importlib.util
import json
import os
from pathlib import Path
import resource
import subprocess
import sys
import time

REPO = Path(__file__).resolve().parents[1]

def digest(path):
    return hashlib.sha256(path.read_bytes()).hexdigest()

def identity(binary):
    binary = binary.resolve()
    drivers = sorted((binary.parent.parent / 'lib').glob('librustc_driver*.so'))
    if len(drivers) != 1:
        raise RuntimeError('expected one source-matched native driver')
    return {'launcher_sha256': digest(binary), 'driver_sha256': digest(drivers[0]),
            'version': subprocess.run([str(binary), '-vV'], check=True, capture_output=True, text=True).stdout}

def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--before', required=True, type=Path)
    parser.add_argument('--after', required=True, type=Path)
    parser.add_argument('--root', required=True, type=Path)
    parser.add_argument('--out', required=True, type=Path)
    parser.add_argument('--rounds', type=int, default=2)
    args = parser.parse_args()
    if not 1 <= args.rounds <= 10:
        parser.error('rounds must be 1..10')
    args.root.mkdir(parents=True, exist_ok=True)
    if any(args.root.iterdir()):
        parser.error('root must be empty to prevent stale receipt reuse')
    sys.path.insert(0, str(REPO / 'tests'))
    spec = importlib.util.spec_from_file_location('native_fixture', REPO / 'tests/test_execution_graph_wait_native.py')
    fixture = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(fixture)
    source = args.root / 'fixture.rs'
    source.write_text(fixture.shared_query_fixture(functions=64, width=64, depth=32))
    compilers = {'before': args.before.resolve(), 'after': args.after.resolve()}
    identities = {label: identity(binary) for label, binary in compilers.items()}
    environment = os.environ.copy()
    environment.pop('JUSTRUST_EXECUTION_GRAPH_DIR', None)
    samples = []
    total = 2 * 2 * args.rounds * 4
    for threads in (1, 4):
        for capture in (False, True):
            for round_id in range(args.rounds):
                for order, label in enumerate(('before', 'after', 'after', 'before')):
                    number = len(samples)
                    output = args.root / 'program'
                    receipt_dir = args.root / f'receipts-{number}'
                    command = [str(compilers[label]), '--edition=2024', '--crate-name=point_capture_cost',
                               '-Copt-level=0', '-Cdebuginfo=0', f'-Zthreads={threads}', str(source), '-o', str(output)]
                    if capture:
                        command.append(f'-Zjr-query-graph={receipt_dir}')
                    usage = resource.getrusage(resource.RUSAGE_CHILDREN)
                    start = time.perf_counter()
                    compiled = subprocess.run(command, env=environment, capture_output=True, text=True)
                    elapsed = time.perf_counter() - start
                    after = resource.getrusage(resource.RUSAGE_CHILDREN)
                    if compiled.returncode:
                        print(compiled.stdout + compiled.stderr, flush=True)
                        raise RuntimeError(f'native compiler failed with exit {compiled.returncode}')
                    executed = subprocess.run([str(output)], env=environment, check=True, capture_output=True, text=True)
                    receipts = [json.loads(path.read_text()) for path in sorted(receipt_dir.glob('*.json'))] if capture else []
                    queries = [receipt for receipt in receipts if receipt['producer'] == 'rustc-query-admission']
                    if capture and not queries:
                        raise RuntimeError('capture requested without a native query receipt')
                    sample = {'label': label, 'threads': threads, 'capture': capture, 'round': round_id,
                              'abba_position': order, 'wall_seconds': elapsed,
                              'child_cpu_seconds': after.ru_utime + after.ru_stime - usage.ru_utime - usage.ru_stime,
                              'load_average': list(os.getloadavg()), 'output': executed.stdout,
                              'artifact_sha256': digest(output),
                              'query_schemas': [query['schema'] for query in queries],
                              'admission_lost': [query['lost_events'] for query in queries],
                              'read_lost': [query['query_reads']['lost_events'] for query in queries],
                              'binding_inventory': [query.get('task_bindings') and {
                                  key: query['task_bindings'][key] for key in
                                  ('expected_points', 'records_complete', 'all_targets_bound', 'lost_events')
                              } for query in queries]}
                    samples.append(sample)
                    print('JCODE_PROGRESS ' + json.dumps({'current': len(samples), 'total': total,
                          'message': f'{label} threads={threads} capture={capture} {elapsed:.3f}s'}), flush=True)
    outputs = {sample['output'] for sample in samples}
    if len(outputs) != 1:
        raise RuntimeError('native fixture output changed between compilers/capture modes')
    # Byte identity is a control per thread setting, not a timing assertion.
    for threads in (1, 4):
        for label in compilers:
            if len({sample['artifact_sha256'] for sample in samples
                    if sample['threads'] == threads and sample['label'] == label}) != 1:
                raise RuntimeError('native fixture artifact changed between capture modes for one compiler')
    payload = {'schema': 1, 'scope': 'native-fixed-small-fixture-only', 'source': source.read_text(),
               'source_sha256': digest(source), 'compilers': identities, 'order_per_round': ['before', 'after', 'after', 'before'],
               'quiet_provenance': 'unknown', 'affinity': sorted(os.sched_getaffinity(0)),
               'speed_claim_eligible': False, 'default_promotion_eligible': False, 'samples': samples}
    args.out.parent.mkdir(parents=True, exist_ok=True)
    args.out.write_text(json.dumps(payload, indent=2) + '\n')
    print(f'Saved {len(samples)} paired native controls to {args.out}', flush=True)

if __name__ == '__main__':
    main()
