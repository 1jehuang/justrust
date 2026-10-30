"""Scoped persistent-cache timings through the public release compiler CLI.

Interleave off, empty and warm caches with identical inputs and fresh processes.
Not the historical Cargo synthetic score or a claim about general agent wait.
"""
import argparse
import hashlib
import json
import os
from pathlib import Path
import platform
import random
import re
import statistics
import subprocess
import tempfile
import time

SCHEMA = 1
STATES = ('off', 'cold', 'warm')
FIXTURES = ('01_hello', '03_collatz', '05_gcd_fib', '06_matrix',
            '18_tokenizer_calculator')


def digest(path):
    return hashlib.sha256(Path(path).read_bytes()).hexdigest()


def measurement_environment(locality):
    environment = {'PATH': '/usr/bin:/bin', 'LANG': 'C', 'LC_ALL': 'C'}
    if locality == 'llc':
        environment['JR_GRAPH_LOCALITY'] = 'llc'
    elif locality != 'inherit':
        raise ValueError('unknown graph worker locality')
    return environment


def invoke(command, cwd, env, timeout):
    start = time.perf_counter_ns()
    process = subprocess.run(command, cwd=cwd, env=env, capture_output=True, timeout=timeout)
    return process, time.perf_counter_ns() - start


def checked_summary(process, workers, locality='inherit'):
    summary = json.loads(process.stdout)
    if (process.returncode != 0 or summary.get('mode') != 'build'
            or summary.get('workers') != workers or summary.get('failures') != 0
            or summary.get('capture_complete') is not True
            or summary.get('events_dropped') != 0):
        raise ValueError(f'compiler failed: {process.stdout!r} {process.stderr!r}')
    if locality == 'llc':
        cpus = summary.get('locality_cpus')
        if (summary.get('locality_workers') != workers or not isinstance(cpus, list)
                or len(cpus) < workers
                or any(type(cpu) is not int or cpu < 0 for cpu in cpus)
                or cpus != sorted(set(cpus))):
            raise ValueError('requested shared-cache worker locality was not applied')
    return summary


def compare(a, b, *, allow_binary_change=False):
    """Compare matched runs, optionally allowing only the compiler hash to differ.

    The opt-in is deliberately an API argument, not a field trusted from either
    result file. The suite runner must explicitly forward any CLI opt-in.
    """
    if a.get('native_cache') != SCHEMA or b.get('native_cache') != SCHEMA:
        raise ValueError('compare native cache results only with native cache results')
    contracts = [result.get('contract') for result in (a, b)]
    if any(not isinstance(contract, dict) or not contract for contract in contracts):
        raise ValueError('cache measurement contracts differ')
    hashes = [contract.get('binary_sha256') for contract in contracts]
    if any(not isinstance(value, str) or re.fullmatch(r'[0-9a-fA-F]{64}', value) is None
           for value in hashes):
        raise ValueError('invalid compiler binary_sha256')
    identities = [{key: value for key, value in contract.items()
                   if not (allow_binary_change and key == 'binary_sha256')}
                  for contract in contracts]
    if identities[0] != identities[1]:
        raise ValueError('cache measurement contracts differ')
    contract = a['contract']
    fixtures = contract.get('fixtures')
    if (not isinstance(fixtures, list) or not fixtures
            or any(not isinstance(fixture, str) or not fixture for fixture in fixtures)
            or len(set(fixtures)) != len(fixtures)
            or type(contract.get('samples')) is not int or contract['samples'] < 3
            or type(contract.get('warmups')) is not int or contract['warmups'] < 1
            or contract.get('workers') != [1, 8]
            or contract.get('opts') != ['off', 'final']):
        raise ValueError('invalid benchmark coverage contract')
    expected = [(f, w, o) for f in contract['fixtures'] for w in (1, 8)
                for o in ('off', 'final')]
    for result in (a, b):
        if result.get('complete') is not True:
            raise ValueError('incomplete benchmark run')
        if result.get('state') not in STATES:
            raise ValueError('invalid cache state')
        cells = result.get('cells')
        if (not isinstance(cells, list)
                or any(not isinstance(cell, dict) for cell in cells)
                or [(c.get('fixture'), c.get('workers'), c.get('opt')) for c in cells] != expected):
            raise ValueError('missing or duplicate benchmark cells')
        for cell in result['cells']:
            for field, count in (('samples', contract['samples']), ('warmups', contract['warmups'])):
                samples = cell.get(field, [])
                if not isinstance(samples, list) or len(samples) != count or any(
                        not isinstance(s, dict) or s.get('ok') is not True
                        or type(s.get('wall_ns')) is not int or s['wall_ns'] <= 0
                        for s in samples):
                    raise ValueError('failed or incomplete samples')
    lines = [f'Scoped native cache only: {a["state"]} -> {b["state"]}.',
             f'Compiler binary_sha256: {hashes[0]} -> {hashes[1]} '
             f'({"binary change explicitly allowed" if allow_binary_change else "strict identity"}).',
             'Warm filesystem, fresh processes, local contended/uncontrolled. Not general agent-wait evidence.']
    for ca, cb in zip(a['cells'], b['cells']):
        ma, mb = (statistics.median(s['wall_ns'] for s in c['samples']) for c in (ca, cb))
        lines.append(f'{ca["fixture"]} w{ca["workers"]} opt={ca["opt"]}: '
                     f'{ma/1e6:.3f} -> {mb/1e6:.3f} ms, {ma/mb:.2f}x, {100*(mb/ma-1):+.1f}%')
    return '\n'.join(lines)


def run(args):
    if args.n < 3 or args.warmup < 1:
        raise ValueError('need at least three repeats and one warmup')
    binary = Path(args.binary).resolve(strict=True)
    root = Path(__file__).resolve().parents[1]
    sources = {name: root / 'tests/fixtures/full_programs' / (name + '.rs') for name in FIXTURES}
    # Pin the whole sibling input tree because input capture sees it too.
    inputs = {str(p.relative_to(root)): digest(p) for p in sorted(next(iter(sources.values())).parent.iterdir()) if p.is_file()}
    binary_hash = digest(binary)
    environment = measurement_environment(getattr(args, 'graph_locality', 'inherit'))
    contract = {'binary_sha256': binary_hash, 'runner_sha256': digest(__file__),
                'suite_sha256': digest(root / 'bench/suite.py'), 'inputs': inputs,
                'fixtures': list(FIXTURES), 'workers': [1, 8], 'opts': ['off', 'final'],
                'samples': args.n, 'warmups': args.warmup, 'seed': 1,
                'environment': environment, 'host': platform.node(),
                'machine': platform.machine(), 'cpu_affinity': sorted(os.sched_getaffinity(0)),
                'cpu': Path('/proc/cpuinfo').read_text().split('model name')[1].splitlines()[0],
                'execution': 'local-direct-fresh-process-warm-filesystem',
                'timeout': args.timeout}
    outputs = {state: Path(args.out_prefix + '-' + state + '.json') for state in STATES}
    existing = any(path.exists() for path in outputs.values())
    if existing and not args.resume:
        raise ValueError('refusing to overwrite results, use --resume for checkpoints')
    results = {state: {'native_cache': SCHEMA, 'state': state, 'contract': contract,
                       'scope': 'Scoped native cache only. Not comprehensive synthetic evidence.',
                       'started': time.strftime('%Y-%m-%dT%H:%M:%S%z'), 'complete': False, 'cells': []}
               for state in STATES}
    if existing:
        results = {s: json.loads(p.read_text()) for s, p in outputs.items()}
        if any(r.get('contract') != contract or r.get('state') != s
               for s, r in results.items()):
            raise ValueError('resume contract differs')
        if len({len(r['cells']) for r in results.values()}) != 1:
            raise ValueError('checkpoint states differ, refusing partial overwrite')
    def checkpoint():
        for state, output in outputs.items():
            output.parent.mkdir(parents=True, exist_ok=True)
            temporary = output.with_suffix('.json.tmp')
            with temporary.open('w') as handle:
                json.dump(results[state], handle, indent=2)
                handle.write('\n')
            os.replace(temporary, output)
    checkpoint()
    rng = random.Random(1)
    serial = 0
    with tempfile.TemporaryDirectory(prefix='native-cache-', dir=args.scratch) as work:
        work = Path(work)
        for fixture, source in sources.items():
            oracle = work / 'stock'
            proc = subprocess.run([args.stock, str(source), '--edition=2024', '-Cpanic=abort', '-o', str(oracle)],
                                  capture_output=True, env=environment, timeout=args.timeout)
            if proc.returncode:
                raise ValueError(proc.stderr.decode(errors='replace'))
            expected, _ = invoke([str(oracle)], work, environment, args.timeout)
            oracle.unlink()
            if expected.returncode != 0 or expected.stdout != source.with_suffix('.stdout').read_bytes():
                raise ValueError('stock oracle disagrees with pinned fixture expectation')
            for workers in (1, 8):
                for opt in ('off', 'final'):
                    key = (fixture, workers, opt)
                    if any((c['fixture'], c['workers'], c['opt']) == key
                           for c in results['off']['cells']):
                        for _ in range(args.n + args.warmup):
                            order = list(STATES)
                            rng.shuffle(order)
                        continue
                    cells = {state: {'fixture': fixture, 'workers': workers, 'opt': opt,
                                     'samples': [], 'warmups': []} for state in STATES}
                    warm_cache = work / f'cache-{fixture}-{workers}-{opt}'
                    warm_cache.mkdir()
                    def compile_one(state, iteration):
                        nonlocal serial
                        serial += 1
                        output = work / f'out-{serial}'
                        command = [str(binary), str(source), '-o', str(output), '--workers', str(workers),
                                   '--opt', opt, '--verbose']
                        if state != 'off':
                            cache = warm_cache if state == 'warm' else work / f'cold-{serial}'
                            cache.mkdir(exist_ok=True)
                            command += ['--cache-dir', str(cache)]
                        if digest(binary) != binary_hash:
                            raise ValueError('binary changed during measurement')
                        process, wall = invoke(command, work, environment, args.timeout)
                        summary = checked_summary(process, workers,
                                                  getattr(args, 'graph_locality', 'inherit'))
                        actual, _ = invoke([str(output)], work, environment, args.timeout)
                        if (actual.returncode, actual.stdout, actual.stderr) != (expected.returncode, expected.stdout, expected.stderr):
                            raise ValueError(f'cached output disagrees with stock: {fixture}')
                        stderr = process.stderr.decode(errors='replace')
                        if state == 'warm' and iteration >= 0 and 'Native link cache: Hit' not in stderr:
                            raise ValueError('warm measurement did not hit persistent link cache')
                        if state == 'cold' and 'Native link cache: MissStored' not in stderr:
                            raise ValueError('cold measurement was not a populated miss')
                        sample = {'wall_ns': wall, 'ok': True, 'summary': summary,
                                  'stderr': stderr, 'executable_sha256': digest(output)}
                        output.unlink()
                        return sample
                    # Fill persistent warm entries before discarded timing warmups.
                    compile_one('warm', -1)
                    for iteration in range(-args.warmup, args.n):
                        order = list(STATES)
                        rng.shuffle(order)
                        for state in order:
                            sample = compile_one(state, iteration)
                            cells[state]['samples' if iteration >= 0 else 'warmups'].append(sample)
                    for state in STATES:
                        results[state]['cells'].append(cells[state])
                    checkpoint()
                    medians = {s: round(statistics.median(v['wall_ns'] for v in cells[s]['samples'])/1e6, 3) for s in STATES}
                    print(f'{fixture} w{workers} opt={opt}: {medians} ms', flush=True)
        if digest(binary) != binary_hash or inputs != {str(p.relative_to(root)): digest(p) for p in sorted(next(iter(sources.values())).parent.iterdir()) if p.is_file()}:
            raise ValueError('source or binary changed during measurement')
    for result in results.values():
        result['complete'] = True
    checkpoint()
    for output in outputs.values():
        print(f'saved {output}')


def add_parser(sub):
    p = sub.add_parser('native-cache', description=__doc__)
    p.add_argument('--binary', required=True)
    p.add_argument('--scratch', required=True)
    p.add_argument('--out-prefix', required=True)
    p.add_argument('--stock', default='/usr/bin/rustc')
    p.add_argument('--resume', action='store_true')
    p.add_argument('--graph-locality', choices=('inherit', 'llc'), default='inherit',
                   help='explicit worker locality experiment, pinned in measurement environment')
    p.add_argument('-n', type=int, default=9)
    p.add_argument('--warmup', type=int, default=2)
    p.add_argument('--timeout', type=float, default=120)
    p.set_defaults(fn=run)
