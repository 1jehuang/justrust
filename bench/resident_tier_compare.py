#!/usr/bin/env python3
"""Matched native jr-engine edits: live workers versus restart with disk stores retained.

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
import statistics
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
        'Not exercised: requires multiple projects and a separate memory-budget protocol.'},
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
    return dict(daemon=int(daemon[0]) if len(daemon) == 1 else None,
                workers=sorted(map(int, workers)), served=sum(map(int, served)),
                counters_available=bool(served))


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


def summarize(rows, samples, unchanged):
    result = {}
    for phase in PHASES:
        result[phase] = {}
        matched = unchanged and all(
            len([r for r in rows if r.get('phase') == phase and r.get('mode') == mode]) == samples
            and all(r.get('accepted') for r in rows if r.get('phase') == phase and r.get('mode') == mode)
            for mode in MODES)
        for mode in MODES:
            selected = [r for r in rows if r.get('phase') == phase and r.get('mode') == mode]
            admitted = matched
            result[phase][mode] = dict(observed=len(selected), expected=samples,
                exclusion_reason=None if admitted else 'identity gate, incomplete pair or failed matched-mode oracle',
                accepted=sum(bool(r.get('accepted')) for r in selected), excluded=not admitted,
                median_workload_seconds=statistics.median(r['elapsed_seconds'] for r in selected) if admitted else None,
                median_total_seconds=statistics.median(r['total_seconds'] for r in selected) if admitted else None,
                failures=[dict(replicate=r['replicate'], reasons=r.get('failures', ['unvalidated']))
                          for r in selected if not r.get('accepted')])
    return result


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
                    if phase != 'type-error' and (not before['counters_available'] or
                            not after['counters_available'] or after['served'] <= before['served']):
                        errors.append('native daemon-served counter did not advance, fallback not admitted')
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


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    for name in ('engine', 'source', 'output', 'toolchain'):
        parser.add_argument('--' + name, required=True, type=shared.absolute)
    parser.add_argument('--proc-macro-server', type=shared.absolute)
    parser.add_argument('--samples', type=int, default=5)
    parser.add_argument('--timeout', type=float, default=360)
    args = parser.parse_args()
    raise SystemExit(execute(args))


if __name__ == '__main__':
    main()
