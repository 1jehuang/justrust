#!/usr/bin/env python3
"""Hosted-only real-project disk AST diagnostic, never a general speedup gate.

Uses a pristine public f97fcc2 fixture, private client/target/cache namespaces,
normal foreground justrust recording, and the unchanged ultrafast runtime oracle.
No compiler execution occurs on import or in the mock-only test suite.
"""
import argparse
import json
import os
from pathlib import Path
import re
import shutil
import statistics
import uuid

import native_disk_snapshot as snapshot
import ultrafast_hosted_compare as oracle

shared = oracle.shared
runtime = oracle.runtime
REASON = 'controlled native snapshot experiment on authenticated hosted guest'
PHASES = oracle.PHASES + ('full-library',)


def environment(root, compiler, mode, raw=False, cargo=None):
    """Whitelist process basics. Never copy operator credentials or Cargo config."""
    env = {key: os.environ[key] for key in ('PATH', 'LANG', 'LC_ALL', 'TZ')
           if key in os.environ}
    for name in ('home', 'cargo', 'target', 'disk'):
        (root / name).mkdir(mode=0o700, exist_ok=True)
    env.update(HOME=str(root / 'home'), CARGO_HOME=str(root / 'cargo'),
               CARGO_TARGET_DIR=str(root / 'target'),
               JUSTRUST_HOME=str(root / 'home/.justrust'),
               XDG_CONFIG_HOME=str(root / 'home/.config'),
               XDG_CACHE_HOME=str(root / 'home/.cache'),
               XDG_STATE_HOME=str(root / 'home/.local/state'),
               JUSTRUST_TOOLCHAIN='system', RUSTC=str(compiler),
               JUSTRUST_NO_UPDATE='1', JUSTRUST_AUTO_UPDATE='0',
               JUSTRUST_NO_TELEMETRY='1', JUSTRUST_REMOTE='off',
               JUSTRUST_PASSES='0',
               JUSTRUST_LOCAL_REASON=REASON, CARGO_INCREMENTAL='1',
               RUSTFLAGS='-Ztime-passes' if raw else '',
               JUSTRUST_RUSTC_DISK_AST_STATS=str(root / 'stats.jsonl'),
               JUSTRUST_RUSTC_DISK_AST_PHASES=str(root / 'phases.jsonl'))
    if cargo is not None:
        env['JUSTRUST_REAL_CARGO'] = str(cargo)
        env['PATH'] = str(cargo.parent) + os.pathsep + env.get('PATH', '')
    if mode == 'on':
        env['JUSTRUST_RUSTC_DISK_AST'] = str(root / 'disk')
    return env


def text_at(prefix):
    return '\n'.join(prefix.with_suffix('.' + stream).read_text(errors='replace')
                     for stream in ('stdout', 'stderr'))


def invoke(args, source, env, prefix, phase, marker):
    arguments = (['--lib', '--', '--test-threads=1', '--nocapture']
                 if phase == 'full-library' else shared.FILTER)
    command = [str(args.client), 'test', '--local=' + REASON, *arguments]
    row = runtime.run_process(command, source, env, prefix, args.timeout)
    row.update(phase=phase, rustflags=env['RUSTFLAGS'], expected_marker=marker,
               diagnostic_environment=dict(env),
               raw_streams={stream: dict(file=str(prefix.with_suffix('.' + stream)),
                   sha256=runtime.base.digest(prefix.with_suffix('.' + stream)))
                   for stream in ('stdout', 'stderr')},
               source_sha256=runtime.base.digest(source / 'src/summary.rs'))
    ids = set(re.findall(r'justrust (?:log|show) (\d{8}-\d{9}-\d+)\b', text_at(prefix)))
    row['run_ids'] = sorted(ids)
    row['identity_verified'] = False
    text = ''
    if len(ids) == 1:
        run_id = next(iter(ids))
        meta_path = Path(env['JUSTRUST_HOME']) / 'runs' / run_id / 'meta.json'
        try:
            if meta_path.stat().st_size > 1024 * 1024:
                raise ValueError('oversized foreground metadata')
            meta = json.loads(meta_path.read_text())
            preserved = prefix.with_suffix('.run-meta.json')
            shutil.copyfile(meta_path, preserved)
            row['run_metadata_file'] = str(preserved)
            row['run_metadata_sha256'] = runtime.base.digest(preserved)
            row['identity_verified'] = (
                meta.get('id') == run_id and meta.get('cwd') == str(source)
                and meta.get('args') == ['test', *arguments]
                and meta.get('real_rustc') == str(args.compiler))
            row['real_cargo'] = meta.get('real_cargo')
            process_path = meta_path.parent / 'processes.json'
            if process_path.is_file():
                preserved_processes = prefix.with_suffix('.processes.json')
                shutil.copyfile(process_path, preserved_processes)
                row['sampled_processes_file'] = str(preserved_processes)
                row['sampled_processes_sha256'] = runtime.base.digest(preserved_processes)
                row['sampled_processes'] = json.loads(preserved_processes.read_text())
                row['sampled_processes_policy'] = (
                    'Client samples may miss short processes and truncate command lines to 600 characters. '
                    'PID correlation is diagnostic, not exact source provenance.')
        except (OSError, ValueError, TypeError) as exc:
            row['identity_error'] = repr(exc)
        log_prefix = prefix.with_name(prefix.name + '-client-log')
        log = runtime.run_process([str(args.client), 'log', run_id], source, env,
                                  log_prefix, args.timeout)
        row['log_retrieval'] = log
        if row['identity_verified'] and runtime.base.succeeded(log):
            text = text_at(log_prefix)
    row['parsed'] = runtime.test_results(text)
    row['accepted'] = row['identity_verified'] and oracle.accepted(phase, row, text, marker)
    return row


def telemetry(path, offset):
    """Retain unknown fields. Missing source attribution never becomes hit evidence."""
    data = path.read_bytes() if path.exists() else b''
    if len(data) < offset:
        raise ValueError('native append-only telemetry truncated')
    events = []
    for line in data[offset:].splitlines():
        event = json.loads(line)
        if not isinstance(event, dict) or type(event.get('pid')) is not int or event['pid'] <= 0:
            raise ValueError('native telemetry lacks positive compiler PID')
        events.append(event)
    return events, len(data)


def coverage(events, source):
    files = {}
    unattributed = []
    for event in events:
        filename = event.get('source')
        if not isinstance(filename, str) or not filename:
            unattributed.append(event)
            continue
        path = Path(filename)
        if not path.is_absolute():
            path = source / path
        key = str(path)
        item = files.setdefault(key, dict(pids=[], events=[], exists=path.is_file()))
        item['pids'] = sorted(set(item['pids'] + [event['pid']]))
        item['events'].append(event)
        if path.is_file():
            item['sha256'] = runtime.base.digest(path)
    return dict(files=files, unattributed_events=unattributed,
                edited_file=str(source / 'src/summary.rs'),
                policy='Only native-reported exact source paths attributed. No inferred module coverage.')


def summarize(rows, unchanged, samples):
    expected = samples * 2 * len(PHASES)
    valid = (unchanged and len(rows) == expected and
             all(row.get('accepted') for row in rows))
    full = [row['parsed'] for row in rows if row['phase'] == 'full-library']
    valid = valid and bool(full) and all(parsed == full[0] for parsed in full)
    hits = sum(event.get('event') == 'hit' for row in rows if row['mode'] == 'on'
               for event in row.get('disk_events', []))
    # A hit alone is insufficient: this harness does not establish independent PID
    # provenance for Cargo's descendants. Keep acceptance false even with hits.
    return dict(status='diagnostic-complete' if valid else 'failed/incomplete',
                runtime_identity_gates_passed=bool(valid), native_hit_events=hits,
                cache_speedup_accepted=False,
                cache_policy=('Zero disk hits: no cache speedup acceptance.' if not hits else
                              'Hits observed, diagnostic only. Independent compiler PID/source provenance required for acceptance.'),
                medians_seconds=({mode: {phase: statistics.median(
                    row['elapsed_seconds'] for row in rows
                    if row['mode'] == mode and row['phase'] == phase)
                    for phase in PHASES} for mode in ('off', 'on')} if valid else {}))


def execute(args):
    host = snapshot.host_guard(args.execute_host_workloads)
    if not 1 <= args.samples <= 10 or not 0 < args.timeout <= 1800:
        raise ValueError('bounded samples 1..10 and timeout (0,1800] required')
    if (args.output.exists() or args.output.is_relative_to(args.source)
            or args.source.is_relative_to(args.output)):
        raise ValueError('output must be fresh and disjoint from source')
    shared.pinned_source(args.source)
    for tool in (args.compiler, args.client):
        with tool.open('rb') as stream:
            if stream.read(4) != b'\x7fELF' or not os.access(tool, os.X_OK):
                raise ValueError('compiler and client must be absolute executable ELF files')
    identities = {str(p): runtime.base.digest(p) for p in (args.compiler, args.client)}
    compiler_before = snapshot.shared.identity(args.compiler)
    before = shared.source_hashes(args.source)
    original = (args.source / 'src/summary.rs').read_bytes()
    args.output.mkdir(mode=0o700, parents=True)
    rows, attribution = [], []
    receipt = dict(pin=shared.PIN, host=host, identities=identities, source_before=before,
                   scope=__doc__, raw_rustc_phases=args.raw_rustc_phases,
                   coverage_limit='Root-only native parsed AST cache is not whole-project AST reuse.',
                   compiler_before=compiler_before, incremental=True,
                   cargo=str(getattr(args, 'cargo', None)),
                   rows=rows, attribution_rows=attribution)

    def save():
        snapshot.write_receipt(args.output / 'receipt.json', receipt)

    save()
    try:
        for raw in ([False, True] if args.raw_rustc_phases else [False]):
            for sample in range(1 if raw else args.samples):
                nonce = (uuid.uuid4().int & ((1 << 63) - 1)) + 1
                contents = oracle.inputs(original, nonce)
                arms = {}
                for mode in ('off', 'on'):
                    root = args.output / ('raw' if raw else 'timed') / str(sample) / mode
                    root.mkdir(mode=0o700, parents=True)
                    source = root / 'source'
                    shutil.copytree(args.source, source, ignore=shutil.ignore_patterns('.git'))
                    if shared.source_hashes(source) != before:
                        raise ValueError('source changed during private copy')
                    arms[mode] = (root, source, environment(root, args.compiler, mode, raw, getattr(args, 'cargo', None)), [0, 0])
                try:
                    for phase in PHASES:
                        for mode in (('off', 'on') if sample % 2 == 0 else ('on', 'off')):
                            root, source, env, offsets = arms[mode]
                            (source / 'src/summary.rs').write_bytes(contents[phase])
                            (root / (phase + '-summary.rs')).write_bytes(contents[phase])
                            marker = f'JR_RUNTIME_EXECUTED_{oracle.phase_nonce(nonce, phase)}'
                            prefix = root / phase
                            row = invoke(args, source, env, prefix, phase, marker)
                            row.update(mode=mode, sample=sample, untimed_attribution=raw)
                            (attribution if raw else rows).append(row)
                            for index, (name, filename) in enumerate((('disk_events', 'stats.jsonl'),
                                                                      ('phase_events', 'phases.jsonl'))):
                                row[name], offsets[index] = telemetry(root / filename, offsets[index])
                            row['coverage'] = coverage(row['disk_events'], source)
                            pids = {event['pid'] for event in row['disk_events'] + row['phase_events']}
                            row['native_pid_samples'] = [process for process in row.get('sampled_processes', [])
                                                         if process.get('pid') in pids]
                            row['unobserved_native_pids'] = sorted(pids - {
                                process['pid'] for process in row['native_pid_samples']})
                            save()
                            if not row['accepted']:
                                raise ValueError(f'runtime/identity gate failed: {sample} {mode} {phase}')
                finally:
                    for root, source, env, offsets in arms.values():
                        (source / 'src/summary.rs').write_bytes(original)
                        stop_prefix = root / 'daemon-stop'
                        receipt.setdefault('cleanup', []).append(runtime.run_process(
                            [str(args.client), 'daemon', 'stop'], source, env, stop_prefix, args.timeout))
    except BaseException as exc:
        receipt['error'] = repr(exc)
        if not isinstance(exc, Exception):
            raise
    finally:
        try:
            receipt['compiler_after'] = snapshot.shared.identity(args.compiler)
            receipt['unchanged'] = (compiler_before == receipt['compiler_after'] and before == shared.source_hashes(args.source) and
                identities == {str(p): runtime.base.digest(p) for p in (args.compiler, args.client)})
        except (OSError, ValueError) as exc:
            receipt.update(unchanged=False, identity_error=repr(exc))
        gate = receipt['unchanged'] and 'error' not in receipt
        gate = gate and all(runtime.base.succeeded(r) for r in receipt.get('cleanup', []))
        summary = summarize(rows, gate, args.samples)
        receipt['summary'] = summary
        save()
        snapshot.write_receipt(args.output / 'summary.json', summary)
    return 0 if summary['runtime_identity_gates_passed'] else 1


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    for name in ('compiler', 'client', 'source', 'output'):
        parser.add_argument('--' + name, required=True, type=shared.absolute)
    parser.add_argument('--cargo', type=shared.absolute, help='Concrete Cargo binary, not a rustup shim')
    parser.add_argument('--execute-host-workloads', action='store_true')
    parser.add_argument('--samples', type=int, default=3)
    parser.add_argument('--timeout', type=float, default=360)
    parser.add_argument('--raw-rustc-phases', action='store_true',
                        help='Separate untimed fresh namespace attribution pass with exact RUSTFLAGS=-Ztime-passes')
    raise SystemExit(execute(parser.parse_args()))


if __name__ == '__main__':
    main()
