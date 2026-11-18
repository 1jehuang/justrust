#!/usr/bin/env python3
"""Hosted encoded frontend archive tier experiment, not persistent typed heaps.

Run this on an authenticated hosted guest with an explicit compiler candidate.
No compiler builds, installation, or local native execution are performed here.
"""
import argparse
import hashlib
import json
import os
from pathlib import Path
import socket
import statistics
import sys
import time

import stable_resident as native
from stable_resident_dogfood import upload

TIERS = ('hot', 'compressed', 'cold')
MODES = ('fresh', 'control', 'frontend-hot', 'frontend-compressed', 'frontend-cold')


def memory(pid, proc=Path('/proc')):
    """Sample the live process, not ru_maxrss or a sum of compiler mappings."""
    result = {}
    fields = {'Rss': 'rss_kib', 'Pss': 'pss_kib', 'Private_Clean': 'private_clean_kib',
              'Private_Dirty': 'private_dirty_kib', 'Private_Hugetlb': 'private_hugetlb_kib',
              'Swap': 'swap_kib', 'SwapPss': 'swap_pss_kib'}
    text = (proc / str(pid) / 'smaps_rollup').read_text()
    for line in text.splitlines():
        key, _, value = line.partition(':')
        if key in fields:
            result[fields[key]] = int(value.split()[0])
    for key in ('rss_kib', 'pss_kib', 'private_clean_kib', 'private_dirty_kib', 'swap_kib'):
        if key not in result:
            raise AssertionError('Missing live /proc field: ' + key)
    result['private_kib'] = sum(result.get(key, 0) for key in
                                ('private_clean_kib', 'private_dirty_kib', 'private_hugetlb_kib'))
    return result


def control(worker, tier):
    """Legacy resident control frame, sharing Resident's sequence and response pipe."""
    if tier not in TIERS:
        raise ValueError('Unknown tier: ' + tier)
    worker.sequence += 1
    request = dict(id=worker.sequence, tier=tier)
    record = dict(request=request, memory_before=memory(worker.process.pid))
    start = time.monotonic()
    worker.transcript.write(json.dumps({'request': request}).encode() + b'\n')
    worker.transcript.flush()
    worker.process.stdin.write(json.dumps(request).encode() + b'\n')
    worker.process.stdin.flush()
    while b'\n' not in worker.buffer:
        remaining = worker.timeout - (time.monotonic() - start)
        if remaining <= 0:
            raise TimeoutError('Resident tier control timed out')
        if not worker.selector.select(min(remaining, .01)):
            continue
        chunk = os.read(worker.response_fd, 65536)
        if not chunk:
            raise RuntimeError('Resident closed control response FD')
        worker.buffer += chunk
        if len(worker.buffer) > 64 * 1024 * 1024:
            raise ValueError('Control response exceeded 64 MiB')
    line, worker.buffer = worker.buffer.split(b'\n', 1)
    worker.transcript.write(json.dumps({'response_b64': native.encoded(line)}).encode() + b'\n')
    worker.transcript.flush()
    response = json.loads(line)
    record.update(response=response, elapsed_seconds=time.monotonic() - start,
                  memory_after=memory(worker.process.pid))
    if (response.get('id') != worker.sequence or response.get('pid') != worker.process.pid
            or type(response.get('status')) is not int or response['status'] != 0
            or response.get('tier') != tier or not isinstance(response.get('memory'), dict)):
        raise AssertionError('Invalid tier acknowledgement: ' + repr(response))
    return record


def order(rep, index):
    """Balanced Latin square including reversed rows, not always fresh first."""
    size = len(MODES)
    base = [0, 1, size - 1, 2, size - 2]
    rotation = (rep + index) % size
    row = [MODES[(value + rotation) % size] for value in base]
    return row if ((rep + index) // size) % 2 == 0 else row[::-1]


def validate(row):
    reference = native.observable(row['modes']['fresh']['compile'])
    if reference['status'] != (1 if row['expected_stdout'] is None else 0):
        raise AssertionError('Unexpected fresh compiler result: ' + repr(reference))
    for mode, result in row['modes'].items():
        if native.observable(result['compile']) != reference:
            raise AssertionError('Exact status/diagnostics differ for ' + mode)
        if row['expected_stdout'] is not None:
            expected = dict(status=0, stdout_b64=native.encoded(row['expected_stdout'].encode()), stderr_b64='')
            if native.observable(result['runtime']) != expected:
                raise AssertionError('Runtime differs for ' + mode)


def green_loads(before, after):
    native.validate_frontend(after)
    for key in ('archive_hits', 'value_loads', 'typeck_loads', 'mir_loads'):
        if after[key] <= before.get(key, 0):
            raise AssertionError('Unchanged wake has no genuine ' + key)


def summarize(rows):
    result = {}
    for mode in MODES:
        selected = [row['modes'][mode] for row in rows]
        result[mode] = dict(median_compile_wall_seconds=statistics.median(
            row['compile']['elapsed_seconds'] for row in selected))
        wakes = [row for row in selected if 'transition' in row]
        if wakes:
            result[mode].update(median_transition_seconds=statistics.median(
                row['transition']['elapsed_seconds'] for row in wakes),
                median_wake_wall_seconds=statistics.median(row['wake_wall_seconds'] for row in wakes),
                median_idle_rss_kib=statistics.median(row['idle_memory']['rss_kib'] for row in wakes),
                median_idle_pss_kib=statistics.median(row['idle_memory']['pss_kib'] for row in wakes),
                median_idle_private_kib=statistics.median(row['idle_memory']['private_kib'] for row in wakes),
                median_idle_swap_kib=statistics.median(row['idle_memory']['swap_kib'] for row in wakes))
    return result


def capture(compiler, root, repetitions=10, items=64, timeout=180, *, hosted=False):
    if not hosted:
        raise ValueError('Native executions are hosted-only. Run on the hosted guest with hosted=True')
    if repetitions < 1 or items < 1 or timeout <= 0:
        raise ValueError('Positive repetitions, items and timeout required')
    compiler, root = Path(compiler).resolve(), Path(root).resolve()
    workload = [row for row in native.frontend_cases(items)
                if row[0] not in ('delete-incremental', 'after-delete', 'corrupt-incremental', 'after-corrupt')]
    report = dict(schema=1, verdict='incomplete', scope='hosted-native-encoded-frontend-archive-tiers',
                  retention_scope='Encoded query archives and dependency graphs, rehydrated each request. '
                  'Not persistent typed heaps or retained GlobalCtxt.',
                  memory_note='Live Linux smaps_rollup KiB between requests. Not per-request peaks.',
                  host=dict(hostname=socket.gethostname(), platform=sys.platform),
                  inputs=dict(repetitions=repetitions, items=items, timeout=timeout),
                  modes=list(MODES), observations=[], sessions=[],
                  workload_sha256=hashlib.sha256(json.dumps(workload).encode()).hexdigest())
    try:
        report['identity_before'] = native.identity(compiler)
        for rep in range(repetitions):
            repdir = root / f'repetition-{rep}'
            repdir.mkdir(mode=0o700)
            caches = {mode: repdir / mode for mode in MODES}
            workers = {}
            for cache in caches.values():
                cache.mkdir(mode=0o700)
            try:
                for mode in MODES[1:]:
                    workers[mode] = native.Resident(compiler, caches[mode], root, False, timeout,
                                                    frontend=mode != 'control')
                    report['sessions'].append(dict(mode=mode, repetition=rep,
                        pid=workers[mode].process.pid, command=workers[mode].command,
                        frontend=mode != 'control', values=False))
                for index, (name, source, expected) in enumerate(workload):
                    row = dict(repetition=rep, name=name, source=source,
                               source_sha256=hashlib.sha256(source.encode()).hexdigest(),
                               expected_stdout=expected, order=order(rep, index), modes={})
                    report['observations'].append(row)
                    for mode in row['order']:
                        (root / 'fixture.rs').write_text(source)
                        binary = root / 'fixture'
                        binary.unlink(missing_ok=True)
                        argv = native.arguments(root, caches[mode])
                        result = row['modes'][mode] = {}
                        previous = {}
                        if mode == 'fresh':
                            result['compile'] = native.run([compiler, *argv], timeout, cwd=root,
                                                            env=native.compiler_env(False, False))
                        else:
                            worker = workers[mode]
                            previous = dict(worker.frontend_counters or {})
                            # Warm first, then measure the requested state while still alive.
                            if index:
                                result['transition'] = control(worker, 'hot' if mode == 'control'
                                                               else mode.split('-', 1)[1])
                            result['idle_memory'] = memory(worker.process.pid)
                            start = time.monotonic()
                            result['compile'] = worker.compile(argv)
                            result['wake_wall_seconds'] = time.monotonic() - start
                            result['awake_memory'] = memory(worker.process.pid)
                            if worker.counters['resident_hits']:
                                raise AssertionError('Scalar resident values must remain disabled')
                            if name == 'unchanged' and mode in ('frontend-hot', 'frontend-compressed'):
                                green_loads(previous, worker.frontend_counters)
                        if result['compile']['status'] == 0:
                            result['artifact_sha256'] = native.digest(binary)
                            result['runtime'] = native.run([binary], timeout)
                    validate(row)
                for mode in ('frontend-hot', 'frontend-compressed'):
                    native.validate_frontend_acceptance(workers['control'].frontend_counters,
                                                        workers[mode].frontend_counters)
                native.validate_frontend(workers['frontend-cold'].frontend_counters)
            finally:
                failing = sys.exc_info()[0] is not None
                errors = []
                for mode, worker in workers.items():
                    try:
                        worker.close()
                    except Exception as error:
                        errors.append(error)
                        report.setdefault('shutdown_errors', []).append(dict(mode=mode, error=str(error)))
                if errors and not failing:
                    raise errors[0]
        report['identity_after'] = native.identity(compiler)
        if report['identity_before'] != report['identity_after']:
            raise AssertionError('Compiler identity changed')
        report['summary'] = summarize(report['observations'])
        report['summary_by_case'] = {name: summarize([row for row in report['observations']
                                                     if row['name'] == name])
                                     for name, _, _ in workload}
        report['verdict'] = 'passed'
        return report
    except BaseException as error:
        report.update(verdict='failed', error=f'{type(error).__name__}: {error}')
        raise
    finally:
        # Transcript paths remain private and every raw byte is embedded in the receipt.
        report['raw_streams'] = {str(path.relative_to(root)): native.encoded(path.read_bytes())
                                 for path in sorted(root.rglob('resident.*')) if path.is_file()}
        (root / 'receipt.json').write_text(json.dumps(report, indent=2) + '\n')


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--compiler', type=Path, required=True)
    parser.add_argument('--output', type=Path, required=True)
    parser.add_argument('--hosted', action='store_true', help='Attest execution on authenticated hosted guest')
    parser.add_argument('-n', '--repetitions', type=int, default=10)
    parser.add_argument('--items', type=int, default=64)
    parser.add_argument('--timeout', type=float, default=180)
    args = parser.parse_args()
    if not args.hosted or args.repetitions < 1 or args.items < 1 or args.timeout <= 0:
        parser.error('--hosted and positive repetitions, items and timeout are required')
    os.umask(0o077)
    args.output.mkdir(mode=0o700, parents=True, exist_ok=False)
    try:
        report = capture(args.compiler, args.output, args.repetitions, args.items, args.timeout, hosted=True)
        print(json.dumps(dict(verdict=report['verdict'], summary=report['summary']), indent=2))
    finally:
        upload(args.output.resolve())


if __name__ == '__main__':
    main()
