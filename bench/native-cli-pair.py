#!/usr/bin/env python3
"""Paired public jr-rust CLI timings, not the legacy Cargo benchmark score.

Supply separately built release binaries. This runner never builds or installs
compilers. Every build sample is executed and checked against a fresh stock
oracle. Results are per case, mode and worker count, not a general speed claim.
"""
import argparse
import hashlib
import json
import os
from pathlib import Path
import platform
import random
import statistics
import subprocess
import tempfile
import time


def digest(path):
    return hashlib.sha256(path.read_bytes()).hexdigest()


def run(command):
    return subprocess.run(command, capture_output=True, timeout=60)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--before', type=Path, required=True)
    parser.add_argument('--after', type=Path, required=True)
    parser.add_argument('--before-revision', required=True)
    parser.add_argument('--after-revision', required=True)
    parser.add_argument('--output', type=Path, required=True)
    parser.add_argument('--scratch', type=Path, required=True)
    parser.add_argument('--repeats', type=int, default=21)
    parser.add_argument('--warmups', type=int, default=2)
    parser.add_argument('--stock', default='/usr/bin/rustc')
    args = parser.parse_args()
    if args.repeats < 3 or args.warmups < 0:
        parser.error('need at least three repeats and nonnegative warmups')
    if args.output.exists():
        parser.error('refusing to overwrite result')
    binaries = {'before': args.before.resolve(), 'after': args.after.resolve()}
    hashes = {name: digest(path) for name, path in binaries.items()}
    root = Path(__file__).resolve().parents[1]
    sources = [root / 'tests/fixtures/full_programs' / (name + '.rs') for name in
               ['01_hello', '03_collatz', '05_gcd_fib']]
    sources.append(root / 'tests/fixtures/trait_objects/positive/dyn_drop_owner.rs')
    result = {
        'schema': 1,
        'runner_sha256': digest(Path(__file__)),
        'scope': 'warm filesystem, fresh process, native supported subset only; CLI labels measurements contended/uncontrolled',
        'before_revision': args.before_revision,
        'after_revision': args.after_revision,
        'binary_sha256': hashes,
        'source_sha256': {p.stem: digest(p) for p in sources},
        'host': {'system': platform.system(), 'machine': platform.machine(),
                 'cpu_count': os.cpu_count()},
        'repeats': args.repeats, 'warmups': args.warmups,
        'stock_version': run([args.stock, '--version']).stdout.decode().strip(),
        'cells': [],
    }
    args.scratch.mkdir(parents=True, exist_ok=True)
    randomizer = random.Random(1)
    with tempfile.TemporaryDirectory(prefix='native-pair-', dir=args.scratch) as work:
        work = Path(work)
        serial = 0
        for source in sources:
            expected = source.with_suffix('.stdout').read_bytes()
            stock = work / (source.stem + '-stock')
            compiled = run([args.stock, '--edition=2024', '-C', 'panic=abort',
                            str(source), '-o', str(stock)])
            if compiled.returncode:
                raise RuntimeError(compiled.stderr.decode())
            oracle = run([str(stock)])
            assert (oracle.returncode, oracle.stdout, oracle.stderr) == (0, expected, b'')
            for workers in [1, 8]:
                for mode in ['check', 'build']:
                    samples = {'before': [], 'after': []}
                    pairs = []
                    for iteration in range(-args.warmups, args.repeats):
                        order = ['before', 'after']
                        randomizer.shuffle(order)
                        pair = {}
                        for name in order:
                            serial += 1
                            output = work / f'output-{serial}'
                            command = [str(binaries[name]), str(source), '--workers', str(workers)]
                            command += ['--check'] if mode == 'check' else ['-o', str(output)]
                            start = time.perf_counter_ns()
                            compiled = run(command)
                            elapsed = time.perf_counter_ns() - start
                            if compiled.returncode or compiled.stderr:
                                raise RuntimeError(f'{name} {source.name}: {compiled!r}')
                            summary = json.loads(compiled.stdout)
                            assert summary['mode'] == mode and summary['workers'] == workers
                            assert summary['failures'] == 0 and summary['events_dropped'] == 0
                            assert summary['capture_complete'] is True
                            assert (summary['executable_bytes'] == 0) == (mode == 'check')
                            if mode == 'build':
                                executed = run([str(output)])
                                assert (executed.returncode, executed.stdout, executed.stderr) == (0, expected, b'')
                                output.unlink()
                            else:
                                assert not output.exists()
                            if iteration >= 0:
                                samples[name].append(elapsed)
                                pair[name] = elapsed
                        if iteration >= 0:
                            pairs.append(pair)
                    medians = {name: statistics.median(values) for name, values in samples.items()}
                    ratio = statistics.median(p['after'] / p['before'] for p in pairs)
                    spread = {
                        name: {'p10_ns': sorted(values)[len(values) // 10],
                               'p90_ns': sorted(values)[9 * len(values) // 10]}
                        for name, values in samples.items()
                    }
                    cell = {'source': source.stem, 'workers': workers, 'mode': mode,
                            'samples_ns': samples, 'pairs_ns': pairs, 'spread': spread,
                            'median_ns': medians, 'paired_median_after_over_before': ratio,
                            'correctness': 'all samples matched stock exit/stdout/stderr'}
                    result['cells'].append(cell)
                    print(f'{source.stem} w{workers} {mode}: {medians["before"]/1e6:.3f} -> '
                          f'{medians["after"]/1e6:.3f} ms, paired ratio {ratio:.3f}', flush=True)
    for source in sources:
        assert digest(source) == result['source_sha256'][source.stem], 'input changed during run'
    for name, path in binaries.items():
        assert digest(path) == hashes[name], f'{name} binary changed during run'
    args.output.parent.mkdir(parents=True, exist_ok=True)
    with args.output.open('x') as output:
        json.dump(result, output, indent=2)
        output.write('\n')


if __name__ == '__main__':
    main()
