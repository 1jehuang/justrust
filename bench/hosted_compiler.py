#!/usr/bin/env python3
"""Direct-host, warm-filesystem library CHECK/BUILD comparisons, not Cargo scores.

Run this script ON the measured machine with already-built compiler binaries.
There is no SSH, installation, source rewriting, cache dropping or compiler build.
Process-wall samples include compiler process launch, but never runner setup,
input hashing, metadata collection, artifact hashing, profiling or SSH startup.
Native phase timings are a separate untimed --profile pass, not rustc phases.

Example (unchanged Adler2 checkout):
  python3 bench/hosted_compiler.py --source /pinned/adler2/src/lib.rs \
    --input-root /pinned/adler2 --binary /pinned/jr-rust --rustc rustc \
    --cfg 'feature="std"' --workers 1 8 24 48 96 --repeats 7 \
    --profile --scratch /scratch --output /scratch/hosted-adler2.json
"""
from __future__ import annotations

import argparse
from datetime import datetime, timezone
import hashlib
import json
import math
import os
from pathlib import Path
import platform
import random
import shutil
import statistics
import subprocess
import sys
import tempfile
import time

SCHEMA = 1
DEFAULT_WORKERS = (1, 8, 24, 48, 96)
SKIP_DIRS = {'.git', 'target', '__pycache__', '.pytest_cache'}


def positive_int(value):
    number = int(value)
    if number <= 0:
        raise argparse.ArgumentTypeError('must be positive')
    return number


def positive_timeout(value):
    number = float(value)
    if not math.isfinite(number) or number <= 0:
        raise argparse.ArgumentTypeError('must be finite and positive')
    return number


def sha256(path):
    digest = hashlib.sha256()
    with Path(path).open('rb') as stream:
        for block in iter(lambda: stream.read(1024 * 1024), b''):
            digest.update(block)
    return digest.hexdigest()


def strict_json(text):
    def invalid(value):
        raise ValueError(f'non-finite JSON number: {value}')
    def finite_float(value):
        number = float(value)
        if not math.isfinite(number):
            invalid(value)
        return number
    return json.loads(text, parse_constant=invalid, parse_float=finite_float)


def resolve_binary(value):
    # Preserve rustup's dispatch basename rather than invoking its resolved shim.
    return str(Path(shutil.which(value) or value).absolute())


def inventory(roots, excluded=()):
    """Pin source trees, including modules/manifests; reject implicit symlinks."""
    result = {}
    excluded = [Path(path).absolute() for path in excluded]
    for root in roots:
        root = Path(root).resolve(strict=True)
        paths = [root] if root.is_file() else root.rglob('*')
        for path in sorted(paths):
            if any(part in SKIP_DIRS for part in path.relative_to(root).parts):
                continue
            if any(path == skip or skip in path.parents for skip in excluded):
                continue
            if path.is_symlink():
                raise ValueError(f'symlink input must be explicitly pinned: {path}')
            if path.is_file():
                result[str(path)] = sha256(path)
    return result


def cells(workers):
    return [dict(backend=backend, workers=worker, mode=mode)
            for backend, worker in [('stock', None), *(('native', n) for n in workers)]
            for mode in ('check', 'build')]


def schedule(count, rounds, order, seed):
    """One visit per cell per round, reproducible independently of execution."""
    rng = random.Random(seed)
    base = list(range(count))
    for iteration in range(rounds):
        if order == 'random':
            row = rng.sample(base, len(base))
        else:
            offset = iteration % count
            row = base[offset:] + base[:offset]
        yield row


def command(cell, binary, rustc, source, output, edition, cfg=(), profile=None):
    if cell['backend'] == 'native':
        cmd = [str(binary), str(source), '--lib', '--edition', edition,
               '--workers', str(cell['workers']), '--opt', 'off',
               '--debug-assertions', 'yes']
        cmd += ['--check'] if cell['mode'] == 'check' else ['-o', str(output)]
        if profile is not None:
            cmd += ['--profile', str(profile)]
    else:
        if profile is not None:
            raise ValueError('stock phase profiling is not part of stable rustc comparisons')
        cmd = [str(rustc), str(source), '--crate-name', 'hosted_bench', '--crate-type', 'rlib',
               '--edition=' + edition, '-C', 'opt-level=0', '-C', 'debug-assertions=yes',
               '-C', 'overflow-checks=yes', '-C', 'panic=abort',
               '--emit=' + ('metadata' if cell['mode'] == 'check' else 'link'), '-o', str(output)]
    for predicate in cfg:
        cmd += ['--cfg', predicate]
    return cmd


def observation():
    """Untimed contention/affinity snapshot, not a CPU-share estimator."""
    result = {'at': datetime.now(timezone.utc).isoformat(),
              'loadavg': list(os.getloadavg()) if hasattr(os, 'getloadavg') else None,
              'affinity': sorted(os.sched_getaffinity(0)) if hasattr(os, 'sched_getaffinity') else None}
    try:
        result['cpu_ticks'] = Path('/proc/stat').read_text().splitlines()[0]
    except OSError:
        result['cpu_ticks'] = None
    return result


def topology():
    result = {'host': platform.node(), 'platform': platform.platform(),
              'machine': platform.machine(), 'logical_cpus': os.cpu_count(),
              'observation': observation(), 'cpu_topology': []}
    cpuinfo = Path('/proc/cpuinfo')
    if cpuinfo.exists():
        result['cpu_models'] = sorted({line.partition(':')[2].strip()
                                      for line in cpuinfo.read_text().splitlines()
                                      if line.partition(':')[0].strip() in ('model name', 'Hardware')})
    for cpu in sorted(Path('/sys/devices/system/cpu').glob('cpu[0-9]*')):
        entry = {'cpu': int(cpu.name[3:])}
        for name in ('physical_package_id', 'core_id', 'thread_siblings_list'):
            path = cpu / 'topology' / name
            entry[name] = path.read_text().strip() if path.exists() else None
        result['cpu_topology'].append(entry)
    return result


def invoke(cmd, cwd, env, timeout, timed=False):
    event = {'command': cmd, 'cwd': str(cwd), 'timed': timed, 'ok': False,
             'before': observation()}
    stdout = stderr = b''
    start = time.perf_counter_ns()
    try:
        process = subprocess.run(cmd, cwd=cwd, env=env, capture_output=True,
                                 timeout=timeout, check=False)
        elapsed = time.perf_counter_ns() - start
        stdout, stderr = process.stdout, process.stderr
        event.update(returncode=process.returncode, ok=process.returncode == 0)
    except subprocess.TimeoutExpired as error:
        elapsed = time.perf_counter_ns() - start
        stdout, stderr = error.stdout or b'', error.stderr or b''
        event.update(returncode=None, error='timeout')
    except OSError as error:
        elapsed = time.perf_counter_ns() - start
        event.update(returncode=None, error=str(error))
    event.update(stdout=stdout.decode('utf-8', errors='backslashreplace'),
                 stderr=stderr.decode('utf-8', errors='backslashreplace'), after=observation())
    if timed:
        event['process_wall_ns'] = elapsed
        if type(elapsed) is not int or elapsed <= 0:
            event.update(ok=False, error='non-positive or non-integer process sample')
    return event


def accept(event, cell, output, profile=None):
    """Success requires diagnostics, typed summary and the requested artifact."""
    if not event['ok']:
        return
    try:
        if cell['backend'] == 'native':
            data = strict_json(event['stdout'])
            if (not isinstance(data, dict) or type(data.get('schema')) is not int or data['schema'] != 1
                    or data.get('mode') != cell['mode'] or type(data.get('workers')) is not int
                    or data['workers'] != cell['workers']
                    or type(data.get('failures')) is not int or data['failures'] != 0
                    or data.get('capture_complete') is not True
                    or type(data.get('events_dropped')) is not int or data['events_dropped'] != 0
                    or type(data.get('total_ns')) is not int or data['total_ns'] <= 0
                    or type(data.get('engine_ns')) is not int or data['engine_ns'] < 0):
                raise ValueError('invalid or incomplete native summary')
            event['native_summary'] = data
        if cell['backend'] == 'stock' or cell['mode'] == 'build':
            if not output.is_file() or output.stat().st_size == 0:
                raise ValueError('compiler did not produce the requested nonempty artifact')
            event['artifact'] = {'bytes': output.stat().st_size, 'sha256': sha256(output)}
        if profile is not None:
            data = strict_json(profile.read_text())
            if (not isinstance(data, dict) or data.get('schema') != 1
                    or data.get('mode') != cell['mode'] or data.get('workers') != cell['workers']
                    or data.get('success') is not True
                    or type(data.get('compiler_wall_ns')) is not int or data['compiler_wall_ns'] <= 0):
                raise ValueError('invalid native phase profile')
            event['profile'] = data
    except (OSError, ValueError, TypeError) as error:
        event.update(ok=False, error=str(error))


def statistics_for(samples, expected):
    if len(samples) != expected or any(type(n) is not int or n <= 0 for n in samples):
        raise ValueError('incomplete or non-finite process samples')
    return {'count': len(samples), 'min_ns': min(samples), 'median_ns': statistics.median(samples),
            'mean_ns': statistics.mean(samples), 'max_ns': max(samples)}


def report_failure(event):
    if not event['ok']:
        print('FAILED: ' + repr(event['command']), file=sys.stderr)
        print(event.get('error', f'exit {event["returncode"]}'), file=sys.stderr)
        print(event['stdout'], end='', file=sys.stderr)
        print(event['stderr'], end='', file=sys.stderr)


def benchmark(args, report):
    if len(set(args.workers)) != len(args.workers):
        raise ValueError('duplicate worker counts')
    output = args.output.absolute()
    scratch = args.scratch.absolute()
    scratch.mkdir(parents=True, exist_ok=True)
    source = args.source.resolve(strict=True)
    if not source.is_file():
        raise ValueError('source must be a library entry file')
    if scratch == source.parent or scratch in source.parents:
        raise ValueError('scratch must not contain the source tree')
    binary, rustc = resolve_binary(args.binary), resolve_binary(args.rustc)
    roots = [source.parent, *args.input_root]
    inputs = inventory(roots, [output, scratch])
    identities = {binary: sha256(binary), rustc: sha256(rustc)}
    env = {key: os.environ[key] for key in ('PATH', 'HOME', 'RUSTUP_HOME', 'CARGO_HOME',
                                         'RUSTUP_TOOLCHAIN', 'LD_LIBRARY_PATH') if key in os.environ}
    env.update(LANG='C', LC_ALL='C', TZ='UTC', JR_OPT='off')
    report.update(events=[], cells=cells(args.workers), metadata={
        'recorded_at': datetime.now(timezone.utc).isoformat(), 'topology': topology(),
        'source': str(source), 'inputs': inputs, 'source_sha256': sha256(source),
        'binary': binary, 'rustc': rustc, 'binary_hashes': identities,
        'runner_sha256': sha256(__file__), 'compiler_revision': args.compiler_revision,
        'environment': env, 'edition': args.edition, 'optimization': 'native off / rustc opt-level=0',
        'debug_assertions': 'yes', 'overflow_checks': 'yes',
        'panic': 'abort; native default source configuration, no panic CLI flag',
        'cfg': args.cfg, 'workers': args.workers, 'stock_threads': 'rustc default, no -Zthreads',
        'repeats': args.repeats, 'warmups': args.warmups, 'seed': args.seed, 'order': args.order,
        'timeout_seconds': args.timeout, 'input_roots': [str(Path(root).resolve()) for root in roots],
        'inventory_exclusions': [str(output), str(scratch), *sorted(SKIP_DIRS)],
    })
    with tempfile.TemporaryDirectory(prefix='hosted-compiler-', dir=scratch) as temporary:
        temporary = Path(temporary)
        for name, cmd in [('rustc_verbose', [rustc, '--version', '--verbose']),
                          ('rustc_sysroot', [rustc, '--print', 'sysroot'])]:
            event = invoke(cmd, temporary, env, args.timeout)
            event.update(phase='metadata', name=name)
            report['events'].append(event)
            report_failure(event)
            if not event['ok'] or not event['stdout'].strip():
                raise ValueError(f'{name} failed, cannot pin stock toolchain')
            report['metadata'][name] = event['stdout']
        actual_rustc = Path(report['metadata']['rustc_sysroot'].strip()) / 'bin' / 'rustc'
        identities[str(actual_rustc)] = sha256(actual_rustc)
        report['metadata']['timed_rustc'] = str(actual_rustc)
        report['metadata']['stock_invocation'] = 'direct sysroot rustc binary; rustup launcher resolution is untimed'

        def verify_identity():
            if (inputs != inventory(roots, [output, scratch])
                    or any(sha256(path) != digest for path, digest in identities.items())):
                raise ValueError('source or compiler identity changed during measurement')

        def collect(index, phase, iteration):
            verify_identity()
            cell = report['cells'][index]
            with tempfile.TemporaryDirectory(prefix='compile-', dir=temporary) as directory:
                directory = Path(directory)
                artifact = directory / ('artifact.native' if cell['backend'] == 'native'
                                        else 'artifact.rmeta' if cell['mode'] == 'check' else 'artifact.rlib')
                profile = directory / 'profile.json' if phase == 'profile' else None
                cmd = command(cell, binary, actual_rustc, source, artifact, args.edition, args.cfg, profile)
                event = invoke(cmd, directory, env, args.timeout, timed=phase == 'measurement')
                accept(event, cell, artifact, profile)
                event.update(cell=index, phase=phase, iteration=iteration)
                cell.setdefault(phase + '_events', []).append(len(report['events']))
                report['events'].append(event)
                report_failure(event)
            verify_identity()

        for phase, rounds in [('warmup', args.warmups), ('measurement', args.repeats)]:
            for iteration, row in enumerate(schedule(len(report['cells']), rounds, args.order,
                                                     args.seed + (phase == 'measurement'))):
                for index in row:
                    collect(index, phase, iteration)
            if phase == 'warmup' and any(not event['ok'] for event in report['events']):
                raise ValueError('warmup failed; no measured samples collected')
        if args.profile:
            for index, cell in enumerate(report['cells']):
                if cell['backend'] == 'native':
                    collect(index, 'profile', 0)
        verify_identity()
    complete = True
    for cell in report['cells']:
        samples = [report['events'][index] for index in cell.get('measurement_events', [])]
        warmups = [report['events'][index] for index in cell.get('warmup_events', [])]
        valid = (len(warmups) == args.warmups and all(event['ok'] for event in samples + warmups))
        cell['statistics'] = statistics_for([event['process_wall_ns'] for event in samples], args.repeats) if valid else None
        complete &= valid and cell['statistics'] is not None
        label = f'{cell["backend"]} {cell["mode"]} workers={cell["workers"]}'
        print(f'{label}: {cell["statistics"]["median_ns"] / 1e6:.3f} ms direct process wall'
              if cell['statistics'] else f'{label}: BLOCKED (no partial-sample median)', flush=True)
    report['measurements_ok'] = bool(complete)
    return 0 if complete and all(event['ok'] for event in report['events']) else 1


def parser():
    p = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    p.add_argument('--source', type=Path, required=True, help='unchanged Rust library entry point')
    p.add_argument('--binary', required=True, help='already-built native compiler')
    p.add_argument('--rustc', default='rustc', help='stock rustc executable or rustup dispatch path')
    p.add_argument('--output', '-o', type=Path, required=True, help='new JSON file, never overwritten')
    p.add_argument('--workers', type=positive_int, nargs='+', default=list(DEFAULT_WORKERS))
    p.add_argument('--repeats', type=positive_int, default=5)
    p.add_argument('--warmups', type=positive_int, default=1)
    p.add_argument('--seed', type=int, default=20261025)
    p.add_argument('--order', choices=('rotate', 'random'), default='random')
    p.add_argument('--edition', choices=('2015', '2018', '2021', '2024'), default='2021')
    p.add_argument('--cfg', action='append', default=[])
    p.add_argument('--input-root', type=Path, action='append', default=[])
    p.add_argument('--profile', action='store_true', help='separate untimed native profile for each worker/mode')
    p.add_argument('--compiler-revision')
    p.add_argument('--timeout', type=positive_timeout, default=120)
    p.add_argument('--scratch', type=Path, default=Path(os.environ.get('JCODE_SCRATCH_DIR', 'target/hosted-compiler-scratch')))
    return p


def main(argv=None):
    args = parser().parse_args(argv)
    report = {'hosted_compiler': SCHEMA, 'ok': False, 'measurements_ok': False,
              'measurement': 'direct compiler process wall includes exec/startup; excludes runner setup, metadata, hashing and SSH; warm filesystem, fresh outputs, no incremental cache',
              'scope': 'unchanged single library, not full Rust compatibility, Cargo or corpus acceptance',
              'backend_scopes': {
                  'native': 'native --lib source admission and direct native library-mode artifact, NOT a reusable stock rlib',
                  'stock': 'rustc rlib crate; CHECK emits metadata, BUILD emits reusable rlib including rustc/LLVM backend',
              }, 'profiling': 'separate untimed native phase runs only; never merged with process samples; no stock phase equivalence'}
    args.output.parent.mkdir(parents=True, exist_ok=True)
    try:
        handle = args.output.open('x', encoding='utf-8')
    except OSError as error:
        parser().error(str(error))
    status = 1
    with handle:
        try:
            status = benchmark(args, report)
            report['ok'] = status == 0
        except (OSError, ValueError) as error:
            report['error'] = str(error)
            print(str(error), file=sys.stderr)
        finally:
            json.dump(report, handle, indent=2, allow_nan=False)
            handle.write('\n')
    print(f'saved {args.output}', flush=True)
    return status


if __name__ == '__main__':
    raise SystemExit(main())
