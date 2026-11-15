#!/usr/bin/env python3
"""Matched native jr-engine live/restart edits and real two-project worker budgets.

Uses existing binaries only. This command runs real workloads when explicitly invoked.
Python unit tests are synthetic and never compile Rust. Requires a pristine pinned source
copy and the frozen toolchain accepted by resident_engine_compare.py.
"""
import argparse
import hashlib
import json
import math
import os
from pathlib import Path
import platform
import re
import shutil
import statistics
import time
import uuid

import resident_engine_compare as shared
import resident_state_matrix as matrix

runtime = shared.runtime
MODES = ('live-daemon', 'restart-disk-subset')
PHASES = ('edit', 'type-error', 'type-recovery', 'assert-failure', 'assert-recovery')
TIERS = {
    'live-daemon': 'Existing native daemon and worker heap retained between matched edits.',
    'restart-disk-subset': 'Native stop, then next command restarts daemon. On-disk store, '
        'function cache, target outputs and runtime files retained. Only the persisted subset '
        'is reusable, not full heap serialization. OS page cache is not flushed.',
    'bounded-worker-eviction': {'status': 'unsupported', 'reason':
        'Not exercised in restart protocol. Use --protocol budget for native memory-budget eviction.'},
    'full-heap-restore': {'status': 'unsupported', 'reason':
        'No supported native full-heap snapshot/restore interface exists in this harness.'},
    'stock/justrust-baseline': {'status': 'unsupported', 'reason':
        'Not included: this harness isolates matched native daemon residency modes.'},
}
POLICY = ('Descriptive matched filtered-test workload only. No fabricated speedup, '
          'general performance claim or promotion gate. Instrumentation affects timings. '
          'Memory is a post-command process snapshot, not peak or whole-machine memory. '
          'Restart total includes stop and workload, excludes status/memory probes. '
          'Live total is workload latency only. Private stores are independently warmed, '
          'not claimed to be byte-identical. No cache deletion or machine-cold claim.')


def digest_bytes(data):
    return hashlib.sha256(data).hexdigest()


def parse_status(text):
    daemon = re.findall(r'^daemon: pid (\d+),', text, re.M)
    workers = re.findall(r'^  worker: pid (\d+),', text, re.M)
    served = re.findall(r': (\d+) builds by the daemon,', text)
    details = [dict(pid=int(pid), project=project, log=log)
               for pid, project, log in re.findall(
                   r'^  worker: pid (\d+), (.*?), (?:holds \d+ MB, )?log (.+)$', text, re.M)]
    return dict(daemon=int(daemon[0]) if len(daemon) == 1 else None,
                workers=sorted(map(int, workers)), worker_details=details,
                ready_projects=re.findall(r'^    (.+): ready$', text, re.M),
                served=sum(map(int, served)), counters_available=bool(served))


def worker_log_snapshot(status, home):
    result = {}
    for worker in status.get('worker_details', []):
        path = Path(worker['log'])
        item = dict(worker, error=None)
        try:
            if path.is_symlink() or not path.resolve().is_relative_to(home.resolve()):
                raise ValueError('worker log is outside private engine home')
            stat = path.stat()
            if stat.st_size > matrix.MAX_RAW_BYTES:
                raise ValueError('worker log exceeds parser limit')
            data = path.read_bytes()
            item.update(size=len(data), sha256=digest_bytes(data), device=stat.st_dev,
                        inode=stat.st_ino)
        except (OSError, ValueError) as exc:
            item['error'] = repr(exc)
        result[str(path)] = item
    return result


def worker_log_evidence(before, after, home, prefix, restart=False, phase='edit'):
    """Admit only fresh worker edits, never a worker's unrelated historical work."""
    evidence = dict(before=before, raw=[], failures=[], classification='unproven')
    chunks = []
    for worker in after.get('worker_details', []):
        path = Path(worker['log'])
        old = before.get(str(path))
        try:
            if path.is_symlink() or not path.resolve().is_relative_to(home.resolve()):
                raise ValueError('worker log is outside private engine home')
            stat = path.stat()
            if stat.st_size > matrix.MAX_RAW_BYTES:
                raise ValueError('worker log exceeds parser limit')
            data = path.read_bytes()
            offset = 0
            if not restart:
                if not old or old.get('error'):
                    raise ValueError('no readable pre-command worker-log receipt')
                if (stat.st_dev, stat.st_ino, worker['pid']) != (old['device'], old['inode'], old['pid']):
                    raise ValueError('worker log/process identity changed in hot mode')
                offset = old['size']
                if digest_bytes(data[:offset]) != old['sha256']:
                    raise ValueError('worker log was truncated or its prefix changed')
            chunk = data[offset:]
            receipt = Path(str(prefix) + f'.worker-{worker["pid"]}.log')
            matrix.durable(receipt, chunk, exclusive=True)
            evidence['raw'].append(dict(path=str(receipt), sha256=digest_bytes(chunk),
                bytes=len(chunk), source=str(path), offset=offset, pid=worker['pid']))
            chunks.append(chunk.decode(errors='replace'))
        except (OSError, ValueError) as exc:
            evidence['failures'].append(repr(exc))
    text = '\n'.join(chunks)
    edited = bool(re.search(r'^edit-to-binary: ', text, re.M))
    refused = bool(re.search(r'^refused:|not built:|^Error:', text, re.M))
    if edited and (phase == 'type-error' or not refused):
        evidence['classification'] = 'native-worker-edit'
    else:
        evidence['failures'].append('no fresh accepted native worker edit evidence')
    return evidence


def memory_snapshot(pids, proc=Path('/proc')):
    """Unavailable procfs metrics remain null, never zero-filled."""
    result = []
    for pid in sorted(set(pids)):
        row = dict(pid=pid, rss_kib=None, pss_kib=None, start_ticks=None, errors=[])
        try:
            stat = (proc / str(pid) / 'stat').read_text()
            row['start_ticks'] = int(stat.rsplit(')', 1)[1].split()[19])
            for line in (proc / str(pid) / 'smaps_rollup').read_text().splitlines():
                fields = line.split()
                if fields and fields[0] in ('Rss:', 'Pss:'):
                    row['rss_kib' if fields[0] == 'Rss:' else 'pss_kib'] = int(fields[1])
        except (OSError, ValueError, IndexError) as exc:
            row['errors'].append(str(exc))
            try:
                for line in (proc / str(pid) / 'status').read_text().splitlines():
                    if line.startswith('VmRSS:'):
                        row['rss_kib'] = int(line.split()[1])
            except (OSError, ValueError, IndexError) as fallback:
                row['errors'].append(str(fallback))
        result.append(row)
    return result


def lifecycle_errors(mode, before, stopped, after):
    errors = []
    if not before['daemon'] or not before['workers']:
        errors.append('prepared daemon/worker unavailable before command')
    if not after['daemon'] or not after['workers']:
        errors.append('daemon/worker unavailable after command')
    if mode == 'live-daemon':
        if before['daemon'] != after['daemon'] or before['workers'] != after['workers']:
            errors.append('live daemon/worker identity changed')
    elif mode == 'restart-disk-subset':
        if stopped is None or stopped['daemon'] or stopped['workers']:
            errors.append('stop did not establish daemon absence')
        if before['daemon'] == after['daemon'] or set(before['workers']) & set(after['workers']):
            errors.append('restart did not establish distinct daemon/worker identities')
    else:
        errors.append('unsupported mode')
    return errors


def contents(original, phase, rep, nonce):
    if original.count(matrix.ANCHOR) != 1:
        raise ValueError('pinned runtime probe anchor must occur exactly once')
    data = original.replace(matrix.ANCHOR, matrix.ANCHOR + matrix.PROBE, 1)
    if phase == 'type-error':
        return runtime.edit_bytes(data, kind='type-error').replace(
            b'JR_RUNTIME_TYPE_ERROR', ('JR_MATRIX_TYPE_' + nonce).encode())
    if phase == 'assert-failure':
        data = runtime.edit_bytes(data, kind='test-failure').replace(
            b'JR_RUNTIME_FAILURE', ('JR_MATRIX_FAILURE_' + nonce).encode())
        return data.replace(matrix.PROBE, b'', 1).replace(
            matrix.ANCHOR, matrix.ANCHOR + matrix.PROBE, 1)
    # Same source bytes in both modes for successful matched edits/recoveries.
    return runtime.edit_bytes(data, rep)


def correctness(row, text, phase, rep, nonce, expected):
    failures = []
    if row.get('timed_out') or row.get('error') or row.get('raw_error'):
        failures.append('launch/timeout/raw receipt error')
    markers = re.findall(r'JR_MATRIX_RUNTIME_[^\s]*', text)
    if phase == 'type-error':
        if not (row.get('status') == 101 and 'E0308' in text and
                'JR_MATRIX_TYPE_' + nonce in text and not markers and row.get('parsed') is None):
            failures.append('fresh compile-error/no-runtime oracle failed')
    else:
        if markers != ['JR_MATRIX_RUNTIME_' + nonce]:
            failures.append('fresh runtime nonce missing, duplicated or stale')
        failing = phase == 'assert-failure'
        if not shared.valid_tests(row.get('parsed'), expected, failing=failing):
            failures.append('exact filtered test contract failed')
        if row.get('status') != (101 if failing else 0):
            failures.append('unexpected exit status')
        if failing:
            if 'JR_MATRIX_FAILURE_' + nonce not in text:
                failures.append('fresh assertion diagnostic missing')
        elif re.findall(r'JR_RUNTIME_EXECUTED_[^\s]*', text) != [f'JR_RUNTIME_EXECUTED_{rep + 1}']:
            failures.append('edited source runtime marker mismatch')
    return failures


def summarize(rows, samples, unchanged, modes=MODES, phases=PHASES):
    result = {}
    for phase in phases:
        result[phase] = {}
        matched = unchanged and all(
            len([r for r in rows if r.get('phase') == phase and r.get('mode') == mode]) == samples
            and all(r.get('accepted') for r in rows if r.get('phase') == phase and r.get('mode') == mode)
            for mode in modes)
        for mode in modes:
            selected = [r for r in rows if r.get('phase') == phase and r.get('mode') == mode]
            admitted = matched
            result[phase][mode] = dict(observed=len(selected), expected=samples,
                exclusion_reason=None if admitted else 'identity gate, incomplete pair or failed matched-mode oracle',
                accepted=sum(bool(r.get('accepted')) for r in selected), excluded=not admitted,
                median_workload_seconds=statistics.median(r['elapsed_seconds'] for r in selected) if admitted else None,
                median_total_seconds=statistics.median(r['total_seconds'] for r in selected) if admitted else None,
                median_ready_for_next_edit_seconds=statistics.median(
                    r['ready_for_next_edit_seconds'] for r in selected)
                    if admitted and all('ready_for_next_edit_seconds' in r for r in selected) else None,
                failures=[dict(replicate=r['replicate'], reasons=r.get('failures', ['unvalidated']))
                          for r in selected if not r.get('accepted')])
    return result


BUDGET_MODES = ('uncapped', 'bounded')
BUDGET_PHASES = ('hot-edit', 'after-peer-edit', 'type-error', 'type-recovery',
                 'assert-failure', 'assert-recovery')


def process_identity(pid, proc=Path('/proc')):
    try:
        fields = (proc / str(pid) / 'stat').read_text().rsplit(')', 1)[1].split()
        return dict(pid=pid, start_ticks=int(fields[19]), state=fields[0])
    except FileNotFoundError:
        return None


def process_gone(identity):
    if not identity:
        return False
    now = process_identity(identity['pid'])
    return now is None or now['start_ticks'] != identity['start_ticks'] or now['state'] == 'Z'


def project_workers(status, source):
    return [w for w in status.get('worker_details', []) if Path(w['project']) == source]


def budget_transition_errors(mode, previous, status, source_a, source_b, identity):
    errors = []
    a, b = project_workers(status, source_a), project_workers(status, source_b)
    if status['daemon'] != previous['daemon']:
        errors.append('daemon changed during peer interleave')
    if not b:
        errors.append('peer B worker unavailable')
    if mode == 'bounded':
        if a or not process_gone(identity):
            errors.append('A worker eviction/process-exit not established')
    elif not a or a[0]['pid'] != identity['pid'] or process_gone(identity):
        errors.append('uncapped A+B retention not established')
    return errors


def budget_eviction_receipt(before, home, source, prefix):
    path = home / 'resident-v1/runtime/daemon.log'
    old = before.get(str(path))
    if not old or old.get('error'):
        raise ValueError('no readable pre-interleave daemon log receipt')
    data = path.read_bytes()
    if len(data) > matrix.MAX_RAW_BYTES or digest_bytes(data[:old['size']]) != old['sha256']:
        raise ValueError('daemon log changed prefix or exceeded limit')
    stat = path.stat()
    if (stat.st_dev, stat.st_ino) != (old['device'], old['inode']):
        raise ValueError('daemon log identity changed')
    chunk = data[old['size']:]
    receipt = Path(str(prefix) + '.eviction.log')
    matrix.durable(receipt, chunk, exclusive=True)
    if not re.search(r'worker \S+ of ' + re.escape(str(source)) +
                     r'/? stopped:.*BLAZE_MEMORY_MAX.*asked longest ago', chunk.decode(errors='replace')):
        raise ValueError('no fresh native budget eviction event for project A')
    return dict(path=str(receipt), bytes=len(chunk), sha256=digest_bytes(chunk),
                source=str(path), offset=old['size'])


def execute(args):
    if not 5 <= args.samples <= 30 or not math.isfinite(args.timeout) or not 0 < args.timeout <= 3600:
        raise ValueError('require 5..30 samples and finite timeout in (0,3600]')
    if (args.output.exists() or len(os.fsencode(args.output)) > 60 or
            args.output.is_relative_to(args.source) or args.source.is_relative_to(args.output)):
        raise ValueError('output must be fresh, disjoint and <=60 bytes')
    socket_example = args.output / 's29/m0/engine/engine-home/resident-v1/runtime/workers' / ('0' * 16) / 'sock'
    if len(os.fsencode(socket_example)) > 107:
        raise ValueError('output path too long for private Unix worker sockets, use e.g. /tmp/rt1')
    shared.pinned_source(args.source)
    with args.engine.open('rb') as stream:
        if stream.read(4) != b'\x7fELF' or not os.access(args.engine, os.X_OK):
            raise ValueError('existing native ELF jr-engine required')
    if args.proc_macro_server:
        with args.proc_macro_server.open('rb') as stream:
            if stream.read(4) != b'\x7fELF' or not os.access(args.proc_macro_server, os.X_OK):
                raise ValueError('existing native ELF proc macro server required')
    tools = {'engine': args.engine}
    if args.proc_macro_server:
        tools['proc-macro-server'] = args.proc_macro_server
    identity = lambda: {k: dict(path=str(p), sha256=runtime.base.digest(p)) for k, p in tools.items()}
    metadata = dict(schema=1, tiers=TIERS, policy=POLICY, pin=shared.PIN,
        source=str(args.source), samples=args.samples, timeout=args.timeout,
        host=dict(platform=platform.platform(), machine=platform.machine(), cpu_count=os.cpu_count()),
        tools_before=identity(), toolchain_before=shared.toolchain_identity(args.toolchain),
        source_before=shared.source_hashes(args.source), environments={}, status='incomplete',
        order='cyclic rotation by replicate, identical phase sequence per mode',
        proc_macro_policy='same explicit server' if args.proc_macro_server else 'both server variables 0',
        planned_workload_calls=args.samples * len(MODES) * (1 + len(PHASES)),
        planned_measured_calls=args.samples * len(MODES) * len(PHASES),
        startup_controls={'RA_JIT_MEMORY_MAX': 'off', 'RA_JIT_SPARE': '0',
                          'RA_JIT_IDLE_SECS': '3600'})
    args.output.mkdir(parents=True, mode=0o700)
    source = args.source / 'src/summary.rs'
    original = source.read_bytes()
    backup = args.output / 'source.original'
    matrix.durable(backup, original, exclusive=True)
    rows, receipts, envs = [], [], []
    complete = False

    def save():
        for name, value in (('metadata.json', metadata), ('records.json', rows), ('receipts.json', receipts)):
            (args.output / name).write_text(json.dumps(value, indent=2) + '\n')

    def invoke(env, label, command):
        prefix = args.output / label
        row = runtime.run_process(command, args.source, env, prefix, args.timeout)
        raw, text, error = matrix.captured(prefix)
        row.update(raw=raw, raw_error=error, label=label)
        receipts.append(row)
        save()
        return row, text

    def control(env, label, operation):
        row, text = invoke(env, label, [str(args.engine), operation])
        if not runtime.base.succeeded(row) or row['raw_error']:
            raise ValueError('native lifecycle control failed: ' + label)
        return parse_status(text), row

    try:
        save()
        for rep in range(args.samples):
            session = args.output / f's{rep}'
            session.mkdir(mode=0o700)
            sessions = {}
            for mode in runtime.order(list(MODES), rep):
                root = session / f'm{MODES.index(mode)}'
                root.mkdir(mode=0o700)
                env = shared.environment(root, 'engine', args.toolchain, args.proc_macro_server)
                env.update(metadata['startup_controls'])
                envs.append(env)
                sessions[mode] = env
                metadata['environments'][f'{rep}:{mode}'] = dict(
                    matrix.receipt_environment(env), **metadata['startup_controls'])
                row, text = invoke(env, f'{rep}-{mode}-version', [str(args.engine), '--version'])
                if not runtime.base.succeeded(row) or not re.search(r'^jr-engine\s+\S+', text, re.M):
                    raise ValueError('native version gate failed')
                nonce = uuid.uuid4().hex
                matrix.durable(source, contents(original, 'edit', 100000 + rep, nonce))
                row, text = invoke(dict(env, JR_MATRIX_NONCE=nonce), f'{rep}-{mode}-prepare',
                                   shared.command(args.engine, 'engine'))
                row['parsed'] = runtime.test_results(text)
                errors = correctness(row, text, 'edit', 100000 + rep, nonce, None)
                if errors:
                    raise ValueError('private cold prepare failed, no seeding: ' + repr(errors))
                sessions[mode] = (env, row['parsed'])
            for phase in PHASES:
                # Both modes see precisely the same edited source and nonce per matched pair.
                nonce = uuid.uuid4().hex
                data = contents(original, phase, rep, nonce)
                for mode in runtime.order(list(MODES), rep):
                    env, expected = sessions[mode]
                    label = f'{rep}-{mode}-{phase}'
                    matrix.durable(source, data)
                    before_inputs = shared.source_hashes(args.source)
                    before, _ = control(env, label + '-before', 'status')
                    memory_before = memory_snapshot([before['daemon']] + before['workers']) if before['daemon'] else []
                    log_before = worker_log_snapshot(before, Path(env['JR_ENGINE_HOME']))
                    stopped, stop_row = None, None
                    if mode == 'restart-disk-subset':
                        _, stop_row = control(env, label + '-stop', 'stop')
                        stopped, _ = control(env, label + '-stopped', 'status')
                    row, text = invoke(dict(env, JR_MATRIX_NONCE=nonce), label,
                                       shared.command(args.engine, 'engine'))
                    row.update(mode=mode, phase=phase, replicate=rep, nonce=nonce,
                        source_sha256=digest_bytes(data), input_manifest=before_inputs,
                        parsed=runtime.test_results(text),
                        before=before, stopped=stopped, memory_before=memory_before,
                        stop_seconds=stop_row['elapsed_seconds'] if stop_row else 0)
                    rows.append(row)
                    after, _ = control(env, label + '-after', 'status')
                    row.update(after=after, total_seconds=row['elapsed_seconds'] + row['stop_seconds'],
                        memory_after=memory_snapshot([after['daemon']] + after['workers']) if after['daemon'] else [])
                    errors = correctness(row, text, phase, rep, nonce, expected)
                    errors += lifecycle_errors(mode, before, stopped, after)
                    evidence = worker_log_evidence(log_before, after, Path(env['JR_ENGINE_HOME']),
                        args.output / label, restart=mode == 'restart-disk-subset', phase=phase)
                    row['native_evidence'] = evidence
                    errors += evidence['failures']
                    if before_inputs != shared.source_hashes(args.source):
                        errors.append('source changed during invocation')
                    row.update(failures=errors, accepted=not errors)
                    save()
            # No process or private cache is shared across repetitions.
            for mode in MODES:
                control(sessions[mode][0], f'{rep}-{mode}-cleanup', 'stop')
    except BaseException as exc:
        metadata['error'] = repr(exc)
        raise
    finally:
        cleanup_errors = []
        for index, env in enumerate(envs):
            try:
                control(env, f'final-{index}-stop', 'stop')
            except Exception as exc:
                cleanup_errors.append(repr(exc))
        metadata['cleanup_errors'] = cleanup_errors
        unchanged = False
        try:
            if backup.read_bytes() != original:
                raise ValueError('durable original backup changed')
            matrix.durable(source, backup.read_bytes())
            metadata['source_after'] = shared.source_hashes(args.source)
            metadata['tools_after'] = identity()
            metadata['toolchain_after'] = shared.toolchain_identity(args.toolchain)
            metadata['raw_receipts_unchanged'] = all(runtime.base.digest(Path(r['path'])) == r['sha256']
                for receipt in receipts for r in receipt['raw'].values())
            metadata['raw_receipts_unchanged'] = metadata['raw_receipts_unchanged'] and all(
                runtime.base.digest(Path(item['path'])) == item['sha256']
                for row in rows for item in row.get('native_evidence', {}).get('raw', []))
            unchanged = all(metadata[k + '_before'] == metadata[k + '_after']
                            for k in ('source', 'tools', 'toolchain'))
            metadata['toolchain_links_valid'] = all(
                (Path(env['BLAZE_HOME']) / 'toolchains' / args.toolchain.name).is_symlink()
                and (Path(env['BLAZE_HOME']) / 'toolchains' / args.toolchain.name).resolve() == args.toolchain
                for env in envs)
            unchanged = unchanged and metadata['raw_receipts_unchanged'] and metadata['toolchain_links_valid'] and not cleanup_errors and 'error' not in metadata
        except Exception as exc:
            unchanged = False
            metadata['restoration_or_identity_error'] = repr(exc)
        states = summarize(rows, args.samples, unchanged)
        complete = unchanged and all(not s['excluded'] for p in states.values() for s in p.values())
        metadata.update(status='complete' if complete else 'failed/incomplete', identities_unchanged=unchanged)
        save()
        (args.output / 'summary.json').write_text(json.dumps(dict(states=states, tiers=TIERS,
            policy=POLICY, status=metadata['status']), indent=2) + '\n')
    return 0 if complete else 1


def execute_budget(args):
    """Actual worker eviction, not daemon restart or a simulated cache tier."""
    if not 5 <= args.samples <= 30 or not math.isfinite(args.timeout) or not 0 < args.timeout <= 3600:
        raise ValueError('require 5..30 samples and finite timeout in (0,3600]')
    if not math.isfinite(args.settle_timeout) or not 0 < args.settle_timeout <= 600:
        raise ValueError('settle timeout must be finite in (0,600]')
    if not re.fullmatch(r'[1-9][0-9]*[kKmMgG]?', args.memory_max):
        raise ValueError('memory max must be positive native size, e.g. 1M or 64M')
    if (args.output.exists() or args.output.is_relative_to(args.source)
            or args.source.is_relative_to(args.output)):
        raise ValueError('output must be fresh and disjoint from pristine source')
    socket_path = args.output / 'm0/engine/engine-home/resident-v1/runtime/workers' / ('0' * 16) / 'sock'
    if len(os.fsencode(socket_path)) > 107:
        raise ValueError('use a short output path such as /tmp/bt1 for Unix sockets')
    shared.pinned_source(args.source)
    tools = {'engine': args.engine}
    if args.proc_macro_server:
        tools['proc-macro-server'] = args.proc_macro_server
    for path in tools.values():
        with path.open('rb') as stream:
            if stream.read(4) != b'\x7fELF' or not os.access(path, os.X_OK):
                raise ValueError('existing native ELF executables required')
    identity = lambda: {k: dict(path=str(p), sha256=runtime.base.digest(p)) for k, p in tools.items()}
    original = (args.source / 'src/summary.rs').read_bytes()
    metadata = dict(schema=2, protocol='budget', samples=args.samples, memory_max=args.memory_max,
        timeout=args.timeout, settle_timeout=args.settle_timeout,
        pin=shared.PIN, source_before=shared.source_hashes(args.source), tools_before=identity(),
        toolchain_before=shared.toolchain_identity(args.toolchain), environments={}, status='incomplete',
        host=dict(platform=platform.platform(), cpu_count=os.cpu_count()),
        policy='Two-project native worker budget. A hot edit and A after-peer edit are separate states. '
               'Bounded misses may run cargo while a replacement worker loads. No blended hit/miss '
               'weight, full heap serialization, machine-cold claim or speedup claim. '
               'All priming and peer commands are outside measured response latency. '
               'ready_for_next_edit includes post-result readiness waits and inspection probes; '
               'Cap is native soft worker budget, not total process or machine RSS limit. '
               'The newest or busy worker may exceed it. Baseline measured separately by coordinator.',
        tiers={'uncapped': 'RA_JIT_MEMORY_MAX=off, A+B workers retained',
               'bounded': 'Native worker LRU eviction with retained disk stores, not daemon restart',
               'full-heap-restore': {'status': 'unsupported'}},
        startup_controls={'RA_JIT_SPARE': '0', 'RA_JIT_IDLE_SECS': '3600'},
        order='rotate modes by replicate, matched source edit markers and phase ordering')
    args.output.mkdir(parents=True, mode=0o700)
    rows, receipts, sessions, copies = [], [], {}, []
    complete = False
    progress = {'time': 0.0, 'rows': -1}

    def save():
        for name, data in (('metadata.json', metadata), ('records.json', rows), ('receipts.json', receipts)):
            (args.output / name).write_text(json.dumps(data, indent=2) + '\n')
        now = time.monotonic()
        if now - progress['time'] >= 15 or progress['rows'] != len(rows):
            print('JCODE_PROGRESS ' + json.dumps(dict(rows=len(rows),
                accepted=sum(bool(r.get('accepted')) for r in rows),
                latest=receipts[-1]['label'] if receipts else 'setup')), flush=True)
            progress.update(time=now, rows=len(rows))

    def invoke(mode, source, label, operation='test', nonce=None):
        env = sessions[mode]['env']
        env = dict(env, JR_MATRIX_NONCE=nonce) if nonce else env
        command = shared.command(args.engine, 'engine') if operation == 'test' else [str(args.engine), operation]
        prefix = args.output / label
        row = runtime.run_process(command, source, env, prefix, args.timeout)
        result_returned = time.monotonic()
        raw, text, error = matrix.captured(prefix)
        row.update(raw=raw, raw_error=error, cwd=str(source), label=label, nonce=nonce,
                   result_returned_monotonic=result_returned)
        receipts.append(row)
        save()
        return row, text

    def status(mode, source, label):
        row, text = invoke(mode, source, label, 'status')
        if not runtime.base.succeeded(row) or row['raw_error']:
            raise ValueError('status probe failed: ' + label)
        return parse_status(text)

    def wait_for(mode, source, label, predicate):
        start = time.monotonic()
        attempt = 0
        while True:
            current = status(mode, source, f'{label}-{attempt}')
            if predicate(current):
                return current, dict(elapsed_seconds=time.monotonic() - start, probes=attempt + 1,
                                     status=current)
            if time.monotonic() - start >= args.settle_timeout:
                raise ValueError('readiness/eviction timeout: ' + label)
            time.sleep(0.2)
            attempt += 1

    def ready(mode, source, label):
        return wait_for(mode, source, label, lambda s: bool(project_workers(s, source))
                        and str(source) in s['ready_projects'])

    def focus(current, source):
        workers = project_workers(current, source)
        return dict(current, workers=[w['pid'] for w in workers], worker_details=workers)

    def prepare(mode, source, label, edit_rep, nonce):
        matrix.durable(source / 'src/summary.rs', contents(original, 'edit', edit_rep, nonce))
        row, text = invoke(mode, source, label, nonce=nonce)
        row['parsed'] = runtime.test_results(text)
        errors = correctness(row, text, 'edit', edit_rep, nonce, sessions[mode].get('expected'))
        if errors:
            raise ValueError('private prepare/peer correctness failed: ' + repr(errors))
        sessions[mode].setdefault('expected', row['parsed'])
        return ready(mode, source, label + '-ready')

    try:
        save()
        for index, mode in enumerate(BUDGET_MODES):
            root = args.output / f'm{index}'
            root.mkdir(mode=0o700)
            a, b = root / 'a', root / 'b'
            for path in (a, b):
                shutil.copytree(args.source, path)
                copies.append(path)
            env = shared.environment(root, 'engine', args.toolchain, args.proc_macro_server)
            env.update(metadata['startup_controls'], RA_JIT_MEMORY_MAX='off' if mode == 'uncapped' else args.memory_max)
            sessions[mode] = dict(env=env, a=a, b=b)
            metadata['environments'][mode] = dict(matrix.receipt_environment(env),
                **metadata['startup_controls'], RA_JIT_MEMORY_MAX=env['RA_JIT_MEMORY_MAX'])
            row, text = invoke(mode, a, mode + '-version', '--version')
            if not runtime.base.succeeded(row) or not re.search(r'^jr-engine\s+\S+', text, re.M):
                raise ValueError('native version gate failed')
            prepare(mode, b, mode + '-initial-b', 300000, uuid.uuid4().hex)
        for rep in range(args.samples):
            matched_nonces = {phase: uuid.uuid4().hex for phase in BUDGET_PHASES}
            for mode in runtime.order(list(BUDGET_MODES), rep):
                session = sessions[mode]
                a, b, env = session['a'], session['b'], session['env']
                label = f'{rep}-{mode}'
                current, prime = prepare(mode, a, label + '-prime-a', 100000 + rep, uuid.uuid4().hex)
                transition, old_identity = None, None
                for phase in BUDGET_PHASES:
                    if phase == 'after-peer-edit':
                        a_workers = project_workers(current, a)
                        old_identity = process_identity(a_workers[0]['pid']) if a_workers else None
                        if old_identity is None:
                            raise ValueError('cannot establish A worker PID/starttick identity')
                        before_peer = current
                        daemon_log = Path(env['JR_ENGINE_HOME']) / 'resident-v1/runtime/daemon.log'
                        daemon_before = worker_log_snapshot(dict(worker_details=[dict(
                            pid=current['daemon'], project=str(a), log=str(daemon_log))]), Path(env['JR_ENGINE_HOME']))
                        _, peer = prepare(mode, b, label + '-peer-b', 200000 + rep, uuid.uuid4().hex)
                        current, transition = wait_for(mode, a, label + '-peer-settle', lambda s:
                            not budget_transition_errors(mode, before_peer, s, a, b, old_identity))
                        transition.update(old_a_identity=old_identity, before=before_peer,
                                          memory=memory_snapshot([current['daemon']] + current['workers']))
                        if mode == 'bounded':
                            transition['eviction_log'] = budget_eviction_receipt(daemon_before,
                                Path(env['JR_ENGINE_HOME']), a, args.output / label)
                    nonce = matched_nonces[phase]
                    edit_rep = rep + 1000 if phase == 'after-peer-edit' else rep
                    kind = phase if phase in PHASES else 'edit'
                    data = contents(original, kind, edit_rep, nonce)
                    matrix.durable(a / 'src/summary.rs', data)
                    before = status(mode, a, label + '-' + phase + '-before')
                    previous_logs = worker_log_snapshot(focus(before, a), Path(env['JR_ENGINE_HOME']))
                    manifest = shared.source_hashes(a)
                    memory_before = memory_snapshot([before['daemon']] + before['workers']) if before['daemon'] else []
                    row, text = invoke(mode, a, label + '-' + phase, nonce=nonce)
                    row.update(mode=mode, phase=phase, replicate=rep, parsed=runtime.test_results(text),
                        source_sha256=digest_bytes(data), input_manifest=manifest,
                        before=before, transition=transition if phase == 'after-peer-edit' else None,
                        total_seconds=row['elapsed_seconds'], memory_before=memory_before,
                        prime_receipt=prime)
                    rows.append(row)
                    after = status(mode, a, label + '-' + phase + '-after')
                    errors = correctness(row, text, kind, edit_rep, nonce, session['expected'])
                    miss = mode == 'bounded' and phase == 'after-peer-edit'
                    if miss:
                        after, reload_wait = ready(mode, a, label + '-miss-ready')
                        row['reload_wait'] = reload_wait
                        evidence = worker_log_evidence(previous_logs, focus(after, a), Path(env['JR_ENGINE_HOME']),
                            args.output / (label + '-' + phase), restart=True, phase=kind)
                        if evidence['classification'] != 'native-worker-edit':
                            reload_said = bool(re.search(r"blaze:.*(?:cargo|loading|loaded|worker)", text))
                            if reload_said and project_workers(after, a):
                                evidence['failures'] = [e for e in evidence['failures']
                                    if e != 'no fresh accepted native worker edit evidence']
                                evidence['classification'] = 'cargo-reload-with-native-worker-ready'
                        if (project_workers(before, a) or not process_gone(old_identity)
                                or not project_workers(after, a)
                                or project_workers(after, a)[0]['pid'] == old_identity['pid']):
                            errors.append('bounded miss did not establish distinct replacement A worker')
                    else:
                        evidence = worker_log_evidence(previous_logs, focus(after, a), Path(env['JR_ENGINE_HOME']),
                            args.output / (label + '-' + phase), phase=kind)
                        errors += lifecycle_errors('live-daemon', focus(before, a), None, focus(after, a))
                    if before['daemon'] != after['daemon']:
                        errors.append('daemon changed, worker-only protocol violated')
                    errors += evidence['failures']
                    if shared.source_hashes(a) != manifest:
                        errors.append('source changed during invocation')
                    row.update(after=after, native_evidence=evidence,
                        memory_after=memory_snapshot([after['daemon']] + after['workers']) if after['daemon'] else [],
                        accepted=not errors, failures=errors)
                    current = after
                    if kind != 'type-error':
                        current, row['ready_wait'] = ready(mode, a, label + '-' + phase + '-ready')
                    # Include readiness probes/inspection after the returned result so
                    # asynchronous reloading is not presented as a completed restore.
                    row['ready_for_next_edit_seconds'] = row['elapsed_seconds'] + max(
                        0, time.monotonic() - row['result_returned_monotonic'])
                    save()
    except BaseException as exc:
        metadata['error'] = repr(exc)
        raise
    finally:
        unchanged = False
        cleanup_errors = []
        for mode, session in sessions.items():
            try:
                row, _ = invoke(mode, session['a'], mode + '-final-stop', 'stop')
                if not runtime.base.succeeded(row):
                    raise ValueError('native stop failed')
            except Exception as exc:
                cleanup_errors.append(repr(exc))
        try:
            for path in copies:
                matrix.durable(path / 'src/summary.rs', original)
            metadata['source_after'] = shared.source_hashes(args.source)
            metadata['copy_identities_restored'] = all(shared.source_hashes(p) == metadata['source_before'] for p in copies)
            metadata['tools_after'] = identity()
            metadata['toolchain_after'] = shared.toolchain_identity(args.toolchain)
            metadata['raw_receipts_unchanged'] = all(runtime.base.digest(Path(r['path'])) == r['sha256']
                for receipt in receipts for r in receipt['raw'].values()) and all(
                    runtime.base.digest(Path(r['path'])) == r['sha256']
                    for row in rows for r in row.get('native_evidence', {}).get('raw', []))
            metadata['raw_receipts_unchanged'] = metadata['raw_receipts_unchanged'] and all(
                runtime.base.digest(Path(row['transition']['eviction_log']['path'])) == row['transition']['eviction_log']['sha256']
                for row in rows if row.get('transition') and 'eviction_log' in row['transition'])
            metadata['toolchain_links_valid'] = all(
                (Path(s['env']['BLAZE_HOME']) / 'toolchains' / args.toolchain.name).is_symlink()
                and (Path(s['env']['BLAZE_HOME']) / 'toolchains' / args.toolchain.name).resolve() == args.toolchain
                for s in sessions.values())
            unchanged = (all(metadata[k + '_before'] == metadata[k + '_after'] for k in ('source', 'tools', 'toolchain'))
                and metadata['copy_identities_restored'] and metadata['raw_receipts_unchanged']
                and metadata['toolchain_links_valid'] and not cleanup_errors and 'error' not in metadata)
        except Exception as exc:
            metadata['identity_error'] = repr(exc)
        metadata['cleanup_errors'] = cleanup_errors
        states = summarize(rows, args.samples, unchanged, modes=BUDGET_MODES, phases=BUDGET_PHASES)
        complete = unchanged and all(not state['excluded'] for phase in states.values() for state in phase.values())
        metadata.update(status='complete' if complete else 'failed/incomplete', identities_unchanged=unchanged)
        save()
        (args.output / 'summary.json').write_text(json.dumps(dict(status=metadata['status'], states=states,
            tiers=metadata['tiers'], policy=metadata['policy']), indent=2) + '\n')
    return 0 if complete else 1


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    for name in ('engine', 'source', 'output', 'toolchain'):
        parser.add_argument('--' + name, required=True, type=shared.absolute)
    parser.add_argument('--proc-macro-server', type=shared.absolute)
    parser.add_argument('--protocol', choices=('restart', 'budget'), default='restart')
    parser.add_argument('--memory-max', default='1M', help='budget protocol native soft worker cap, e.g. 1M or 64M')
    parser.add_argument('--settle-timeout', type=float, default=120,
                        help='budget-only readiness/eviction timeout, outside measured latency')
    parser.add_argument('--samples', type=int, default=5)
    parser.add_argument('--timeout', type=float, default=360)
    args = parser.parse_args()
    raise SystemExit(execute_budget(args) if args.protocol == 'budget' else execute(args))


if __name__ == '__main__':
    main()
