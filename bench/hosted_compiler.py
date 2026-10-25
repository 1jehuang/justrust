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


def _require(condition, message):
    if not condition:
        raise ValueError('invalid hosted compiler evidence: ' + message)


def _digest(value):
    return isinstance(value, str) and len(value) == 64 and all(c in '0123456789abcdef' for c in value)


def _positive(value):
    return type(value) is int and value > 0


def _validate_hosted(result):
    """Reconstruct the measurement contract, never trusting cached medians."""
    _require(isinstance(result, dict) and type(result.get('hosted_compiler')) is int
             and result['hosted_compiler'] == SCHEMA, 'schema mismatch')
    # Includes nested profiles and observations, not just headline samples.
    json.dumps(result, allow_nan=False)
    _require(result.get('ok') is True and result.get('measurements_ok') is True
             and 'error' not in result, 'failed or incomplete run')
    meta = result['metadata']
    _require(isinstance(meta, dict), 'missing metadata')
    workers = meta['workers']
    _require(isinstance(workers, list) and workers and all(_positive(w) for w in workers)
             and len(set(workers)) == len(workers), 'worker inventory')
    repeats, warmups = meta['repeats'], meta['warmups']
    _require(_positive(repeats) and repeats >= 7 and _positive(warmups),
             'at least seven measured samples and one warmup required')
    _require(type(meta['seed']) is int and meta['order'] in ('random', 'rotate'), 'execution order')
    _require(type(meta['timeout_seconds']) in (int, float) and meta['timeout_seconds'] > 0,
             'timeout')
    _require(meta['edition'] in ('2015', '2018', '2021', '2024')
             and meta['optimization'] == 'native off / rustc opt-level=0'
             and meta['debug_assertions'] == 'yes' and meta['overflow_checks'] == 'yes'
             and meta['panic'] == 'abort; native default source configuration, no panic CLI flag'
             and meta['stock_threads'] == 'rustc default, no -Zthreads'
             and meta['stock_invocation'] == 'direct sysroot rustc binary; rustup launcher resolution is untimed',
             'unsupported compilation flags')
    _require(isinstance(meta['cfg'], list) and all(isinstance(c, str) for c in meta['cfg']), 'cfg')
    _require(isinstance(meta['environment'], dict) and meta['environment']
             and all(isinstance(k, str) and isinstance(v, str) for k, v in meta['environment'].items()),
             'environment')
    _require(isinstance(meta['inputs'], dict) and meta['inputs']
             and all(isinstance(k, str) and _digest(v) for k, v in meta['inputs'].items())
             and meta['inputs'].get(meta['source']) == meta['source_sha256'], 'source identity')
    _require(_digest(meta['runner_sha256']), 'runner identity')
    native, stock, launcher = meta['binary'], meta['timed_rustc'], meta['rustc']
    _require(all(isinstance(p, str) and Path(p).is_absolute() for p in (native, stock, launcher))
             and native not in (stock, launcher), 'compiler paths')
    hashes = meta['binary_hashes']
    _require(isinstance(hashes, dict) and all(_digest(v) for v in hashes.values())
             and all(path in hashes for path in (native, stock, launcher)), 'compiler hashes')
    _require(isinstance(meta['rustc_verbose'], str) and meta['rustc_verbose'].strip()
             and str(Path(meta['rustc_sysroot'].strip()) / 'bin' / 'rustc') == stock,
             'actual stock toolchain')
    topology_data = meta['topology']
    _require(isinstance(topology_data, dict) and topology_data.get('host')
             and topology_data.get('machine') and topology_data.get('platform')
             and _positive(topology_data.get('logical_cpus')), 'host topology')
    affinity = topology_data['observation']['affinity']
    _require(isinstance(affinity, list) and affinity and all(type(n) is int and n >= 0 for n in affinity)
             and affinity == sorted(set(affinity)), 'host affinity')
    exclusions = meta['inventory_exclusions']
    _require(isinstance(exclusions, list) and len(exclusions) >= 2
             and all(isinstance(p, str) for p in exclusions)
             and Path(exclusions[0]).is_absolute() and Path(exclusions[1]).is_absolute(),
             'inventory exclusions')
    matrix = cells(workers)
    actual_cells = result['cells']
    _require(isinstance(actual_cells, list) and len(actual_cells) == len(matrix), 'cell matrix')
    events = result['events']
    _require(isinstance(events, list), 'events')
    has_profiles = any('profile_events' in c for c in actual_cells)
    expected = [('metadata', None, 0), ('metadata', None, 1)]
    for phase, rounds in [('warmup', warmups), ('measurement', repeats)]:
        for iteration, row in enumerate(schedule(len(matrix), rounds, meta['order'],
                                                 meta['seed'] + (phase == 'measurement'))):
            expected.extend((phase, index, iteration) for index in row)
    if has_profiles:
        expected.extend(('profile', index, 0) for index, c in enumerate(matrix) if c['backend'] == 'native')
    _require(len(events) == len(expected), 'missing or extra events')
    references = [{} for _ in matrix]
    directories = set()
    for position, (event, (phase, index, iteration)) in enumerate(zip(events, expected)):
        _require(isinstance(event, dict) and event.get('ok') is True
                 and type(event.get('returncode')) is int and event['returncode'] == 0
                 and 'error' not in event and event.get('phase') == phase
                 and event.get('timed') is (phase == 'measurement')
                 and isinstance(event.get('stdout'), str) and isinstance(event.get('stderr'), str),
                 f'failed or malformed event {position}')
        _require(('process_wall_ns' in event) == (phase == 'measurement'), 'profile/warmup mixed with samples')
        if phase == 'measurement':
            _require(_positive(event['process_wall_ns']), 'non-positive or non-integer sample')
        for side in ('before', 'after'):
            snapshot = event[side]
            _require(snapshot['affinity'] == affinity, 'event affinity changed')
            load = snapshot['loadavg']
            _require(isinstance(load, list) and len(load) == 3
                     and all(type(n) in (int, float) and math.isfinite(n) and n >= 0 for n in load),
                     'missing or non-finite load observation')
        cwd = Path(event['cwd'])
        _require(cwd.is_absolute() and Path(exclusions[1]) in cwd.parents, 'fresh output directory')
        if phase == 'metadata':
            name, args = [('rustc_verbose', ['--version', '--verbose']),
                          ('rustc_sysroot', ['--print', 'sysroot'])][iteration]
            _require(event.get('name') == name and event['command'] == [launcher, *args]
                     and event['stdout'] == meta[name], 'toolchain metadata event')
            continue
        _require(type(event.get('cell')) is int and event['cell'] == index
                 and type(event.get('iteration')) is int and event['iteration'] == iteration,
                 'event inventory or iteration')
        _require(str(cwd) not in directories, 'reused output directory')
        directories.add(str(cwd))
        cell = matrix[index]
        artifact = cwd / ('artifact.native' if cell['backend'] == 'native'
                          else 'artifact.rmeta' if cell['mode'] == 'check' else 'artifact.rlib')
        profile = cwd / 'profile.json' if phase == 'profile' else None
        _require(event['command'] == command(cell, native, stock, meta['source'], artifact,
                                            meta['edition'], meta['cfg'], profile), 'command/flags differ')
        references[index].setdefault(phase + '_events', []).append(position)
        if cell['backend'] == 'native':
            data = strict_json(event['stdout'])
            _require(isinstance(data, dict) and type(data.get('schema')) is int and data['schema'] == 1
                     and data.get('mode') == cell['mode'] and type(data.get('workers')) is int
                     and data['workers'] == cell['workers'] and data.get('capture_complete') is True
                     and type(data.get('failures')) is int and data['failures'] == 0
                     and type(data.get('events_dropped')) is int and data['events_dropped'] == 0
                     and _positive(data.get('total_ns')) and type(data.get('engine_ns')) is int
                     and 0 <= data['engine_ns'] <= data['total_ns']
                     and json.dumps(data, sort_keys=True) == json.dumps(event.get('native_summary'), sort_keys=True),
                     'native stdout/summary mismatch')
        if cell['backend'] == 'stock' or cell['mode'] == 'build':
            emitted = event['artifact']
            _require(_positive(emitted['bytes']) and _digest(emitted['sha256']), 'artifact identity')
        else:
            _require('artifact' not in event, 'CHECK unexpectedly emitted artifact')
        if phase == 'profile':
            data = event['profile']
            _require(isinstance(data, dict) and data.get('schema') == 1
                     and data.get('mode') == cell['mode'] and data.get('workers') == cell['workers']
                     and data.get('success') is True and _positive(data.get('compiler_wall_ns')),
                     'phase profile identity')
        else:
            _require('profile' not in event, 'profile data mixed into process samples')
    medians = []
    for index, (actual, cell) in enumerate(zip(actual_cells, matrix)):
        _require({key: actual.get(key) for key in cell} == cell, 'worker/cell inventory')
        _require(set(actual) == set(cell) | set(references[index]) | {'statistics'}, 'cell field inventory')
        for field, indices in references[index].items():
            _require(actual[field] == indices and all(type(n) is int for n in actual[field]),
                     'missing, duplicate or cross-linked event references')
        measured = [events[n]['process_wall_ns'] for n in references[index]['measurement_events']]
        recomputed = statistics_for(measured, repeats)
        _require(actual['statistics'] == recomputed, 'forged or stale cached statistics')
        medians.append(recomputed['median_ns'])
    # Ignore only explicitly volatile fields. New metadata fields must also
    # match, rather than silently escaping the comparison contract.
    contract = dict(meta)
    for field in ('recorded_at', 'binary', 'compiler_revision'):
        contract.pop(field, None)
    contract['binary_hashes'] = {k: v for k, v in hashes.items() if k != native}
    contract['topology'] = {k: v for k, v in topology_data.items() if k != 'observation'}
    contract['affinity'] = affinity
    contract['inventory_exclusions'] = exclusions[1:]
    contract['profiles'] = has_profiles
    for field in ('measurement', 'scope', 'backend_scopes', 'profiling'):
        _require(bool(result.get(field)), 'missing measurement/backend scope')
        contract[field] = result[field]
    return contract, matrix, medians


def compare(a, b):
    """Strict matched-run comparison, including independent stock controls."""
    try:
        ca, matrix, ma = _validate_hosted(a)
        cb, other_matrix, mb = _validate_hosted(b)
        _require(ca == cb and matrix == other_matrix,
                 'source/flags/host/affinity/runner/stock toolchain contracts differ')
    except (KeyError, IndexError, TypeError, AttributeError, OverflowError) as error:
        raise ValueError(f'invalid hosted compiler evidence: missing or malformed {error}') from error
    lines = ['Scoped direct-host library process-wall comparison only, not a global compiler or corpus claim.',
             'Compiler exec/startup included; SSH/setup/hash/profile excluded. Stock rows are host-drift controls, not corrections.',
             f'A native {a["metadata"]["binary_hashes"][a["metadata"]["binary"]]}',
             f'B native {b["metadata"]["binary_hashes"][b["metadata"]["binary"]]}']
    for cell, before, after in zip(matrix, ma, mb):
        label = ('stock control' if cell['backend'] == 'stock' else f'native w{cell["workers"]}')
        lines.append(f'{label} {cell["mode"]}: {before / 1e6:.3f} -> {after / 1e6:.3f} ms '
                     f'({100 * (after / before - 1):+.2f}%), n={a["metadata"]["repeats"]}')
    return '\n'.join(lines)


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
