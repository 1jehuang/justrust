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
import stat
import statistics
import sys
import time
import uuid

import stable_resident_dogfood as dogfood

import stable_resident as native
from stable_resident_dogfood import upload

OWNER_FIELDS = ('graph_bytes', 'archive_bytes', 'hot_entries', 'hot_bytes',
                'compressed_entries', 'compressed_bytes', 'pinned_entries')
TIERS = ('hot', 'compressed', 'cold')
MODES = ('fresh', 'control', 'frontend-hot', 'frontend-compressed', 'frontend-cold', 'frontend-disk')


def disk_snapshot(cache):
    """Content identity of durable incremental artifacts, never mtime identity."""
    files = [dict(path=str(path.relative_to(cache)), bytes=path.stat().st_size,
                  sha256=native.digest(path))
             for path in sorted(cache.rglob('*')) if path.is_file()]
    return dict(file_count=len(files), bytes=sum(row['bytes'] for row in files), files=files,
                sha256=hashlib.sha256(json.dumps(files, sort_keys=True).encode()).hexdigest())


def terminate(worker, cache, proc=Path('/proc')):
    """EOF, reap and prove absence before declaring a zero-worker idle state."""
    start = time.monotonic()
    pid = worker.process.pid
    worker.close()
    status = worker.process.wait(timeout=worker.timeout)
    absent = not (proc / str(pid)).exists()
    if status != 0 or worker.process.poll() is None or not absent:
        raise AssertionError('Disk worker was not successfully reaped and absent')
    before = disk_snapshot(cache)
    if not before['file_count']:
        raise AssertionError('Disk restart requires durable incremental artifacts')
    idle_start = time.monotonic()
    idle = dict(state='terminated', source='no-worker-alive', pid=None,
                rss_kib=0, pss_kib=0, private_kib=0, swap_kib=0)
    after = disk_snapshot(cache)
    if before != after or (proc / str(pid)).exists():
        raise AssertionError('Durable incremental artifacts changed during terminated idle')
    return dict(elapsed_seconds=time.monotonic() - start, old_pid=pid,
                wait_status=status, process_absent=absent, disk_before_idle=before,
                disk_after_idle=after, idle_seconds=time.monotonic() - idle_start), idle


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


def owner_memory(value):
    if not isinstance(value, dict):
        raise AssertionError('Missing owner memory')
    for key in OWNER_FIELDS:
        if type(value.get(key)) is not int or value[key] < 0:
            raise AssertionError('Invalid owner memory field: ' + key)
    if value['archive_bytes'] != value['hot_bytes'] + value['compressed_bytes']:
        raise AssertionError('Archive owner byte accounting mismatch')
    if value['pinned_entries'] != 0:
        raise AssertionError('Idle worker retains pinned archive entries')
    return value


def tier_gate(tier, before, after, frontend=True):
    before, after = owner_memory(before), owner_memory(after)
    if not frontend:
        if any(after.values()):
            raise AssertionError('Frontend-off control retains owner memory')
        return
    if tier == 'compressed' and before['archive_bytes'] > 0:
        if (after['compressed_entries'] <= 0 or after['compressed_bytes'] <= 0
                or after['archive_bytes'] >= before['archive_bytes']
                or after['graph_bytes'] != 0):
            raise AssertionError('Compressed tier did not actually reduce archive owner bytes')
    elif tier == 'cold':
        if any(after.values()):
            raise AssertionError('Cold tier did not release all archive/graph ownership')


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
    owner_memory(response['memory'])
    return record


def order(rep, index):
    """Balanced Latin square including reversed rows, not always fresh first."""
    size = len(MODES)
    base = [0] + [value for pair in ((i, size - i) for i in range(1, size // 2 + 1))
                  for value in pair][:size - 1]
    rotation = (rep + index) % size
    row = [MODES[(value + rotation) % size] for value in base]
    return row if ((rep + index) // size) % 2 == 0 else row[::-1]


def validate(row):
    reference = native.observable(row['modes']['fresh']['compile'])
    wanted_status = 1 if row.get('expectation') == 'compile-fail' else (
        0 if 'expectation' in row else (1 if row['expected_stdout'] is None else 0))
    if reference['status'] != wanted_status:
        raise AssertionError('Unexpected fresh compiler result: ' + repr(reference))
    for mode, result in row['modes'].items():
        if native.observable(result['compile']) != reference:
            raise AssertionError('Exact status/diagnostics differ for ' + mode)
        if 'expectation' in row:
            if wanted_status == 0:
                result['named_statuses'] = dogfood.statuses(result['runtime'], result['nonce'], row['expectation'])
            continue
        if row['expected_stdout'] is not None:
            expected = dict(status=0, stdout_b64=native.encoded(row['expected_stdout'].encode()), stderr_b64='')
            if native.observable(result['runtime']) != expected:
                raise AssertionError('Runtime differs for ' + mode)


def green_loads(before, after, typed=False):
    native.validate_frontend(after)
    # An unchanged build may validate green nodes without demanding Typeck/MIR
    # values. Ordinary body edits below must prove those separate load paths.
    keys = ('archive_hits', 'value_loads') + (('typeck_loads', 'mir_loads') if typed else ())
    for key in keys:
        if after[key] <= before.get(key, 0):
            raise AssertionError('Wake has no genuine ' + key)


def validate_tier_loads(control, current, mode, real_source):
    native.validate_frontend(control)
    native.validate_frontend(current)
    if any(control.values()):
        raise AssertionError('Frontend-off counters must all be zero')
    for key in ('archive_hits', 'value_loads'):
        if current[key] <= 0:
            raise AssertionError(mode + ' reports no actual ' + key)
    if mode == 'frontend-hot' and current['graph_hits'] <= 0:
        raise AssertionError('Hot tier reports no actual graph hits')
    if real_source:
        for key in ('typeck_loads', 'mir_loads'):
            if current[key] <= 0:
                raise AssertionError(mode + ' reports no actual ' + key)
    elif not any(current[key] > 0 for key in ('typeck_loads', 'mir_loads')):
        raise AssertionError(mode + ' reports no actual typed query loads')


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
        spawns = [row['spawn_seconds'] for row in selected if 'spawn_seconds' in row]
        if spawns:
            result[mode]['median_spawn_seconds'] = statistics.median(spawns)
        if mode == 'frontend-disk' and wakes:
            result[mode].update(idle_state='terminated',
                median_disk_bytes=statistics.median(row['transition']['disk_after_idle']['bytes'] for row in wakes),
                median_disk_file_count=statistics.median(row['transition']['disk_after_idle']['file_count'] for row in wakes))
        result[mode]['tier_control'] = {
            'fresh': 'new one-shot process, ordinary incremental disk cache',
            'control': 'live worker, frontend archive disabled',
            'frontend-hot': 'live worker, hot archive and graph',
            'frontend-compressed': 'live worker, compressed archive, graph released',
            'frontend-cold': 'live worker, archive and graph ownership released',
            'frontend-disk': 'worker terminated and reaped, same incremental disk cache, startup included in wake',
        }[mode]
    return result


def hosted_identity(marker):
    """Verify the existing production guest CPU share, never provision a marker."""
    marker = Path(marker)
    if marker != Path('/run/pool/share'):
        raise ValueError('Hosted CPU share must be exactly /run/pool/share')
    metadata = marker.lstat()
    if (not stat.S_ISREG(metadata.st_mode) or metadata.st_uid != 0
            or metadata.st_mode & 0o022):
        raise ValueError('Hosted marker must be a root-owned non-group/world-writable regular file')
    content = marker.read_text().strip()
    if not content.isdecimal() or int(content) <= 0:
        raise ValueError('Hosted CPU share must be a positive decimal integer')
    return dict(marker=str(marker), marker_kind='root-owned-hosted-share',
                allocated_cpus=int(content), uid=metadata.st_uid,
                mode=oct(stat.S_IMODE(metadata.st_mode)), marker_device=metadata.st_dev,
                marker_inode=metadata.st_ino, sha256=native.digest(marker), hostname=socket.gethostname())


def capture(compiler, root, repetitions=10, items=64, timeout=180, *, hosted=False,
            hosted_marker=Path('/run/pool/share'), source=None):
    if not hosted:
        raise ValueError('Native executions are hosted-only. Run on the hosted guest with hosted=True')
    if repetitions < 1 or items < 1 or timeout <= 0:
        raise ValueError('Positive repetitions, items and timeout required')
    host = hosted_identity(hosted_marker)
    compiler, root = Path(compiler).resolve(), Path(root).resolve()
    if any(root.iterdir()):
        raise FileExistsError('Refusing to overwrite prior capture evidence')
    source_path = Path(source).resolve() if source is not None else None
    original = source_path.read_bytes() if source_path else None
    workload = (dogfood.variants(original.decode('utf-8'), frontend=True) if source_path else
                [row for row in native.frontend_cases(items)
                 if row[0] not in ('delete-incremental', 'after-delete', 'corrupt-incremental', 'after-corrupt')])
    work = root / 'private-workload' if source_path else root
    report = dict(schema=1, verdict='incomplete', scope='hosted-native-encoded-frontend-archive-tiers',
                  retention_scope='Encoded query archives and dependency graphs, rehydrated each request. '
                  'Not persistent typed heaps or retained GlobalCtxt.',
                  memory_note='Live Linux smaps_rollup KiB between requests. Not per-request peaks. '
                  'Disk idle zeros mean a verified terminated worker, not /proc samples.',
                  host=dict(**host, platform=sys.platform),
                  inputs=dict(repetitions=repetitions, items=items, timeout=timeout),
                  modes=list(MODES), observations=[], sessions=[],
                  workload_sha256=hashlib.sha256(json.dumps(workload).encode()).hexdigest())
    try:
        if source_path:
            work.mkdir(mode=0o700)
            (work / 'wrapper.rs').write_text(dogfood.WRAPPER)
            (root / 'source-original.rs').write_bytes(original)
            report.update(workload_kind='shipping-remote_sync_queue-module', source_path=str(source_path),
                          source_sha256_before=hashlib.sha256(original).hexdigest(),
                          original_tests=list(dogfood.ORIGINAL_TESTS), added_test=dogfood.PROBE,
                          wrapper_sha256=native.digest(work / 'wrapper.rs'))
            if native.digest(source_path) != report['source_sha256_before']:
                raise AssertionError('Source changed while making private backup')
        else:
            report['workload_kind'] = 'generated-frontend-fixture'
        report['identity_before'] = native.identity(compiler)
        for rep in range(repetitions):
            repdir = root / f'repetition-{rep}'
            repdir.mkdir(mode=0o700)
            caches = {mode: repdir / mode for mode in MODES}
            workers = {}
            frozen_args = {}
            generations = {}
            for cache in caches.values():
                cache.mkdir(mode=0o700)
            def spawn(mode):
                generation = generations.get(mode, -1) + 1
                generations[mode] = generation
                logs = caches[mode]
                if mode == 'frontend-disk':
                    logs = repdir / f'disk-worker-{generation}'
                    logs.mkdir(mode=0o700)
                start = time.monotonic()
                worker = native.Resident(compiler, logs, work, False, timeout,
                                         frontend=mode != 'control')
                elapsed = time.monotonic() - start
                workers[mode] = worker
                report['sessions'].append(dict(mode=mode, repetition=rep, generation=generation,
                    pid=worker.process.pid, command=worker.command, log_directory=str(logs),
                    incremental_directory=str(caches[mode]), spawn_seconds=elapsed,
                    frontend=mode != 'control', values=False))
                return worker, elapsed
            try:
                for mode in MODES[1:]:
                    spawn(mode)
                for index, (name, source, expected) in enumerate(workload):
                    row = dict(repetition=rep, name=name, source=source,
                               source_sha256=hashlib.sha256(source.encode()).hexdigest(),
                               expected_stdout=expected, order=order(rep, index), modes={})
                    if source_path:
                        row['expectation'] = expected
                    report['observations'].append(row)
                    for mode in row['order']:
                        (work / ('remote_sync_queue.rs' if source_path else 'fixture.rs')).write_text(source)
                        binary = work / ('tests' if source_path else 'fixture')
                        binary.unlink(missing_ok=True)
                        argv = (dogfood.arguments(work, caches[mode]) if source_path
                                else native.arguments(work, caches[mode]))
                        result = row['modes'][mode] = {}
                        argv = list(map(str, argv))
                        if mode in frozen_args and argv != frozen_args[mode]:
                            raise AssertionError('Compile arguments changed across requests/restarts')
                        frozen_args[mode] = argv
                        previous = {}
                        if mode == 'fresh':
                            result['compile'] = native.run([compiler, *argv], timeout, cwd=work,
                                                            env=native.compiler_env(False, False))
                        else:
                            worker = workers[mode]
                            previous = dict(worker.frontend_counters or {})
                            # Warm first, then measure the requested state while still alive.
                            if index and mode == 'frontend-disk':
                                # Remove before close so failure cleanup never closes old streams twice.
                                del workers[mode]
                                result['transition'], result['idle_memory'] = terminate(worker, caches[mode])
                                start = time.monotonic()
                                worker, result['spawn_seconds'] = spawn(mode)
                                if worker.process.pid == result['transition']['old_pid']:
                                    raise AssertionError('Restart did not produce a new PID')
                                result['new_pid'] = worker.process.pid
                            elif index:
                                result['hot_owner'] = control(worker, 'hot')
                                tier = 'hot' if mode == 'control' else mode.split('-', 1)[1]
                                result['transition'] = (result['hot_owner'] if tier == 'hot'
                                                        else control(worker, tier))
                                tier_gate(tier, result['hot_owner']['response']['memory'],
                                          result['transition']['response']['memory'], mode != 'control')
                            if not (index and mode == 'frontend-disk'):
                                result['idle_memory'] = memory(worker.process.pid)
                                start = time.monotonic()
                            result['compile'] = worker.compile(argv)
                            result['wake_wall_seconds'] = time.monotonic() - start
                            result['awake_memory'] = memory(worker.process.pid)
                            if worker.counters['resident_hits']:
                                raise AssertionError('Scalar resident values must remain disabled')
                            if mode in ('frontend-hot', 'frontend-compressed'):
                                if name == 'unchanged':
                                    green_loads(previous, worker.frontend_counters)
                                elif name == 'body-edit' and source_path:
                                    green_loads(previous, worker.frontend_counters, typed=True)
                        if result['compile']['status'] == 0:
                            result['artifact_sha256'] = native.digest(binary)
                            if source_path:
                                result['nonce'] = uuid.uuid4().hex
                                env = native.compiler_env()
                                env[dogfood.NONCE_KEY] = result['nonce']
                                result['runtime'] = native.run([binary, '--nocapture', '--test-threads=1',
                                                               '--color=never'], timeout, cwd=work, env=env)
                            else:
                                result['runtime'] = native.run([binary], timeout)
                    validate(row)
                for mode in ('frontend-hot', 'frontend-compressed'):
                    validate_tier_loads(workers['control'].frontend_counters,
                                        workers[mode].frontend_counters, mode, bool(source_path))
                native.validate_frontend(workers['frontend-cold'].frontend_counters)
                native.validate_frontend(workers['frontend-disk'].frontend_counters)
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
        if source_path:
            report['source_sha256_after'] = native.digest(source_path)
            if report['source_sha256_after'] != report['source_sha256_before']:
                raise AssertionError('Original shipping source changed during capture')
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
        if source_path:
            try:
                report['source_sha256_after'] = native.digest(source_path)
                report['source_unchanged'] = report.get('source_sha256_before') == report['source_sha256_after']
                if not report['source_unchanged']:
                    report.update(verdict='failed', source_error='Original shipping source changed')
            except OSError as error:
                report.update(verdict='failed', source_unchanged=False, source_error=str(error))
        report['raw_streams'] = {str(path.relative_to(root)): native.encoded(path.read_bytes())
                                 for path in sorted(root.rglob('resident.*')) if path.is_file()}
        (root / 'receipt.json').write_text(json.dumps(report, indent=2) + '\n')


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--compiler', type=Path, required=True)
    parser.add_argument('--source', type=Path, help='Real shipping remote_sync_queue.rs, copied privately only')
    parser.add_argument('--output', type=Path, required=True)
    parser.add_argument('--hosted', action='store_true', help='Execute on guest with existing root-owned /run/pool/share')
    parser.add_argument('--hosted-marker', type=Path,
                        default=Path('/run/pool/share'))
    parser.add_argument('-n', '--repetitions', type=int, default=10)
    parser.add_argument('--items', type=int, default=64)
    parser.add_argument('--timeout', type=float, default=180)
    args = parser.parse_args()
    if not args.hosted or args.repetitions < 1 or args.items < 1 or args.timeout <= 0:
        parser.error('--hosted and positive repetitions, items and timeout are required')
    os.umask(0o077)
    args.output.mkdir(mode=0o700, parents=True, exist_ok=False)
    try:
        report = capture(args.compiler, args.output, args.repetitions, args.items, args.timeout,
                         hosted=True, hosted_marker=args.hosted_marker, source=args.source)
        print(json.dumps(dict(verdict=report['verdict'], summary=report['summary']), indent=2))
    finally:
        upload(args.output.resolve())


if __name__ == '__main__':
    main()
