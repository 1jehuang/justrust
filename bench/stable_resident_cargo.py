#!/usr/bin/env python3
"""Ordinary Cargo acceptance for already-built native resident candidates.

No compiler builds, installation, transport simulation, or whole-client speed claim.
Full command streams and exact shipping source bytes survive failed acceptance.
Run mock tests first. Real execution requires explicit --compiler and --cargo.
"""
import argparse
import base64
import hashlib
import json
import math
import os
from pathlib import Path
import re
import stat
import sys
import uuid

import stable_resident as native
import stable_resident_dogfood as dogfood

SOURCE = Path(__file__).resolve().parents[1] / 'src/remote_sync_queue.rs'
FLAGS = '-Zunstable-options --jobs-backend=1 --jobs-linker=1'
PROBE = 'cargo_nonce_probe'


def private_directory(path):
    """Create, never reuse, a canonical user-owned private output directory."""
    path = Path(path).absolute()
    if path.parent.resolve() != path.parent:
        raise ValueError('Output parent must not traverse symlinks')
    path.mkdir(mode=0o700, exist_ok=False)
    meta = path.lstat()
    if not stat.S_ISDIR(meta.st_mode) or meta.st_uid != os.geteuid() or stat.S_IMODE(meta.st_mode) != 0o700:
        raise ValueError('Expected owned non-symlink 0700 directory')
    return path


def check_socket_path(output, repetitions):
    # Linux sockaddr_un.sun_path includes its trailing NUL. Cargo uses a
    # 32-hex digest plus '.sock', below this per-arm private directory.
    socket = Path(output).absolute() / f'{repetitions - 1:03d}-on/workers' / ('0' * 32 + '.sock')
    if len(os.fsencode(socket)) >= 108:
        raise ValueError('--output is too long for native Unix sockets; use a short private path')


def environment(compiler, fixture, mode, resident, trace):
    env = {k: v for k, v in os.environ.items()
           if not k.startswith(('JUSTRUST_', 'CARGO_', 'RUST'))
           and k not in ('MAKEFLAGS', 'MFLAGS')}
    env.update(RUSTC=str(compiler), RUSTFLAGS=FLAGS,
               CARGO_TARGET_DIR=str(fixture / ('target-' + mode)),
               CARGO_INCREMENTAL='1', CARGO_TERM_COLOR='never',
               JUSTRUST_RUSTC_RESIDENT_FRONTEND='1',
               JUSTRUST_CARGO_RESIDENT_TRACE=str(trace))
    if mode not in ('off', 'on'):
        raise ValueError('Unknown mode')
    # Keep direct compiler identity and optional query-launch admission matched.
    # Only the RPC execution switch changes between the two arms.
    env['JUSTRUST_CARGO_RESIDENT_DIR'] = str(resident)
    env['JUSTRUST_CARGO_RESIDENT'] = 'serial' if mode == 'on' else 'off'
    return env


def wrapper(nonce):
    return f'''#![allow(dead_code)]
#[path="remote_sync_queue.rs"]
mod remote_sync_queue;
fn edited_value() -> &'static str {{ "{nonce}" }}
#[test]
fn {PROBE}() {{
    let value = edited_value();
    assert_eq!(value, "{nonce}");
    let queue = remote_sync_queue::SyncQueue::new(value);
    assert_eq!(*queue.foreground_lock().unwrap(), "{nonce}");
    eprintln!("JR_CARGO_NONCE={{}}", value);
}}
'''.encode()


def stages(original, nonce):
    anchor = b'Duration::from_millis(1)'
    if original.count(anchor) != 1:
        raise ValueError('Shipping source edit anchor changed')
    edited = original.replace(anchor, b'Duration::from_millis(2)')
    root = wrapper(nonce + '-original')
    changed = wrapper(nonce + '-edited')
    recovered = wrapper(nonce + '-recovered')
    return [
        ('initial', original, root, nonce + '-original', 'pass'),
        ('cargo-fresh', original, root, nonce + '-original', 'fresh'),
        ('forced-same-source', original, root, nonce + '-original', 'pass'),
        ('body-edit', edited, changed, nonce + '-edited', 'pass'),
        ('compile-error', edited, changed + b'\ncompile_error!("JR_CARGO_DELIBERATE_ERROR");\n', None, 'error'),
        ('recovery', edited, changed, nonce + '-edited', 'pass'),
        ('restored-source', original, recovered, nonce + '-recovered', 'pass'),
    ]


def trace_events(path):
    if not path.exists():
        return []
    rows = [json.loads(line) for line in path.read_text().splitlines() if line.strip()]
    if any(not isinstance(row, dict) for row in rows):
        raise AssertionError('Trace must contain JSON objects')
    return rows


def semantics(record, nonce, expectation):
    if record.get('timed_out'):
        raise AssertionError('Cargo timed out')
    stdout = base64.b64decode(record['stdout_b64']).decode(errors='strict')
    stderr = base64.b64decode(record['stderr_b64']).decode(errors='strict')
    diagnostics = []
    for line in stdout.splitlines():
        if not line.startswith('{'):
            continue
        message = json.loads(line)
        if message.get('reason') == 'compiler-message':
            diagnostics.append(message['message'])
    markers = re.findall(r'^JR_CARGO_NONCE=(.*)$', stderr, re.MULTILINE)
    if expectation == 'error':
        if record['status'] != 101 or markers or 'JR_CARGO_DELIBERATE_ERROR' not in stdout + stderr:
            raise AssertionError('Expected compile_error without stale test execution')
        if not any(d.get('level') == 'error' for d in diagnostics):
            raise AssertionError('Missing ordinary Cargo compiler error diagnostic')
        return dict(status=101, diagnostics=diagnostics, tests={})
    pairs = re.findall(r'^test (\S+) \.\.\. (ok|FAILED|ignored)$', stdout, re.MULTILINE)
    expected = {name: 'ok' for name in (*dogfood.ORIGINAL_TESTS, PROBE)}
    if record['status'] != 0 or len(pairs) != len(expected) or dict(pairs) != expected:
        raise AssertionError('Ordinary Cargo named tests/status mismatch: ' + repr(pairs))
    if markers != [nonce]:
        raise AssertionError('Changed body runtime nonce missing or stale')
    if 'test result: ok. 6 passed; 0 failed;' not in stdout:
        raise AssertionError('Unexpected test summary')
    return dict(status=0, diagnostics=diagnostics, tests=expected)


def validate_residency(rows, mode):
    """Require real compile responses, not Cargo's fresh artifact shortcut."""
    keys, previous = {}, {}
    hits = dict(graph_hits=0, archive_hits=0)
    required = {'initial', 'forced-same-source', 'body-edit', 'compile-error', 'recovery', 'restored-source'}
    seen = {}
    for row in rows:
        complete = [e for e in row['trace'] if e.get('event') == 'complete']
        if mode == 'off':
            if complete:
                raise AssertionError('Resident-off unexpectedly submitted native RPC')
            continue
        if row['name'] == 'cargo-fresh':
            if complete:
                raise AssertionError('Cargo fresh shortcut unexpectedly compiled')
            continue
        if not complete:
            raise AssertionError('No native Cargo compile response for ' + row['name'])
        row_keys = set()
        for event in complete:
            result = event.get('result', {})
            pid, key = result.get('pid'), event.get('key')
            if type(pid) is not int or pid <= 0 or not isinstance(key, str) or not key:
                raise AssertionError('Missing native worker PID/cache key')
            if type(result.get('id')) is not int or type(event.get('elapsed_ns')) is not int or event['elapsed_ns'] < 0:
                raise AssertionError('Missing native response ID or elapsed time')
            wanted = 1 if row['expectation'] == 'error' else 0
            if type(result.get('status')) is not int or result['status'] != wanted:
                raise AssertionError('Native response status disagrees with Cargo semantics')
            if key in keys and keys[key] != pid:
                raise AssertionError('Native worker PID changed across edit/error/recovery')
            keys[key] = pid
            row_keys.add(key)
            stats = native.validate_frontend(result.get('frontend'), previous.get(key))
            previous[key] = stats
            for counter in hits:
                hits[counter] = max(hits[counter], stats[counter])
        seen[row['name']] = row_keys
    if mode == 'on':
        common = set.intersection(*(seen[name] for name in required))
        if not common:
            raise AssertionError('No single worker key survived source edits and compiler error/recovery')
        if not all(hits.values()):
            raise AssertionError('No actual frontend graph/archive reuse observed')
    return dict(worker_pids=keys, observed_cumulative_hits=hits)


def capture_mode(compiler, cargo, fixture, root, mode, original, nonce, timeout, runner=native.run):
    root.mkdir(mode=0o700)
    resident = private_directory(root / 'workers')
    trace = root / 'trace.jsonl'
    env = environment(compiler, fixture, mode, resident, trace)
    report = dict(mode=mode, verdict='incomplete', observations=[], failures=[],
                  environment_sha256=hashlib.sha256(json.dumps(env, sort_keys=True).encode()).hexdigest(),
                  fixed_environment={k: env[k] for k in ('RUSTC', 'RUSTFLAGS', 'JUSTRUST_RUSTC_RESIDENT_FRONTEND')})
    try:
        for index, (name, source, lib, nonce_value, expectation) in enumerate(stages(original, nonce)):
            if expectation != 'fresh':
                (fixture / 'src/remote_sync_queue.rs').write_bytes(source)
                (fixture / 'src/lib.rs').write_bytes(lib)
            before = len(trace_events(trace))
            row = dict(name=name, expectation=expectation, nonce=nonce_value,
                       source_sha256=hashlib.sha256(source).hexdigest(),
                       wrapper_sha256=hashlib.sha256(lib).hexdigest())
            report['observations'].append(row)
            (root / f'{index:02d}-source.rs').write_bytes(source)
            (root / f'{index:02d}-lib.rs').write_bytes(lib)
            row['cargo'] = runner([cargo, 'test', '--offline', '--lib', '--message-format=json',
                                   '--', '--nocapture', '--test-threads=1'], timeout, cwd=fixture, env=env)
            row['trace'] = trace_events(trace)[before:]
            try:
                row['semantics'] = semantics(row['cargo'], nonce_value, expectation)
            except AssertionError as error:
                report['failures'].append(dict(case=name, error=str(error)))
        try:
            report['residency'] = validate_residency(report['observations'], mode)
        except AssertionError as error:
            report['failures'].append(dict(case='residency', error=str(error)))
        fallback_nonce = nonce + '-wrapper-fallback'
        (fixture / 'src/remote_sync_queue.rs').write_bytes(original)
        (fixture / 'src/lib.rs').write_bytes(wrapper(fallback_nonce))
        passthrough = root / 'rustc-wrapper'
        passthrough.write_text('#!/bin/sh\nexec "$@"\n')
        passthrough.chmod(0o700)
        fallback_env = dict(env, RUSTC_WRAPPER=str(passthrough))
        before = len(trace_events(trace))
        fallback = report['wrapper_fallback'] = dict(nonce=fallback_nonce)
        fallback['cargo'] = runner([cargo, 'test', '--offline', '--lib', '--message-format=json',
                                    '--', '--nocapture', '--test-threads=1'], timeout,
                                   cwd=fixture, env=fallback_env)
        fallback['trace'] = trace_events(trace)[before:]
        try:
            fallback['semantics'] = semantics(fallback['cargo'], fallback_nonce, 'pass')
            if any(e.get('event') == 'complete' for e in fallback['trace']):
                raise AssertionError('Unsupported RUSTC_WRAPPER did not fall back')
        except AssertionError as error:
            report['failures'].append(dict(case='wrapper-fallback', error=str(error)))
        report['verdict'] = 'failed' if report['failures'] else 'passed'
    except BaseException as error:
        report.update(verdict='failed', error=f'{type(error).__name__}: {error}')
        raise
    finally:
        (root / 'receipt.json').write_text(json.dumps(report, indent=2) + '\n')
    return report


def justrust_runner(client, cargo, runner=native.run):
    """Use the real client on an already-authenticated hosted guest.

    Explicit component paths are not an installed/published toolchain. Keep
    the off/on environment identical except the harness's resident switch.
    """
    def invoke(command, timeout, cwd, env):
        if Path(command[0]) != Path(cargo):
            raise ValueError('Expected the frozen Cargo candidate')
        environment = dict(env, JUSTRUST_TOOLCHAIN='system',
                           JUSTRUST_REAL_CARGO=str(cargo),
                           JUSTRUST_SLOTS='0', JUSTRUST_TEST_PARALLEL='0')
        return runner([client, '--local=resident acceptance already on authenticated hosted guest',
                       'cargo', *command[1:]], timeout, cwd=cwd, env=environment)
    return invoke


def capture(compiler, cargo, output, repetitions=1, timeout=180, justrust=None):
    if repetitions < 1 or not math.isfinite(timeout) or timeout <= 0:
        raise ValueError('Positive repetitions and finite positive timeout required')
    check_socket_path(output, repetitions)
    compiler, cargo = Path(compiler).resolve(strict=True), Path(cargo).resolve(strict=True)
    client = Path(justrust).resolve(strict=True) if justrust is not None else None
    runner = justrust_runner(client, cargo) if client else native.run
    root = private_directory(output)
    original = SOURCE.read_bytes()
    (root / 'original-remote_sync_queue.rs').write_bytes(original)
    fixture = private_directory(root / 'fixture')
    (fixture / 'src').mkdir(mode=0o700)
    (fixture / 'Cargo.toml').write_text('[package]\nname="resident_cargo_fixture"\nversion="0.0.0"\nedition="2024"\n[workspace]\n')
    report = dict(schema=1, scope='ordinary-cargo-std-only-edited-build-acceptance', verdict='incomplete',
                  claim='No standalone protocol result, whole-client speedup, or 10x claim.',
                  limitations=['No dependency fallback acceptance yet', 'Transport cancellation belongs to native worker tests'],
                  original_source=dict(path=str(SOURCE), sha256=hashlib.sha256(original).hexdigest(),
                                       bytes=len(original), bytes_b64=native.encoded(original),
                                       preserved='original-remote_sync_queue.rs'),
                  repetitions=[], failures=[])
    try:
        if client:
            report['scope'] = 'justrust-client-native-cargo-std-only-edited-build-acceptance'
            report['client_before'] = dict(path=str(client), sha256=native.digest(client),
                                           version=native.run([client, '--version']))
            if report['client_before']['version']['status']:
                raise AssertionError('justrust candidate identity query failed')
            report['limitations'].append('Explicit component candidates, not a published/default toolchain; shim phase telemetry unavailable')
        report['compiler_before'] = native.identity(compiler)
        report['cargo_before'] = dict(path=str(cargo), sha256=native.digest(cargo), version=native.run([cargo, '-V']))
        if report['cargo_before']['version']['status'] or report['cargo_before']['version'].get('timed_out'):
            raise AssertionError('Cargo candidate identity query failed')
        for repetition in range(repetitions):
            nonce = uuid.uuid4().hex
            pair = {}
            report['repetitions'].append(pair)
            for mode in ('off', 'on'):
                pair[mode] = capture_mode(compiler, cargo, fixture, root / f'{repetition:03d}-{mode}',
                                          mode, original, nonce, timeout, runner=runner)
            if any(pair[m]['verdict'] != 'passed' for m in pair):
                report['failures'].append(f'Repetition {repetition} failed acceptance')
            if pair['off'].get('wrapper_fallback', {}).get('semantics') != pair['on'].get('wrapper_fallback', {}).get('semantics'):
                report['failures'].append(f'Wrapper fallback semantics differ: {repetition}')
            for off, on in zip(pair['off']['observations'], pair['on']['observations']):
                if off.get('semantics') != on.get('semantics'):
                    report['failures'].append(f'Matching compile/test semantics differ: {repetition}/{off["name"]}')
        report['verdict'] = 'failed' if report['failures'] else 'passed'
    except BaseException as error:
        report.update(verdict='failed', error=f'{type(error).__name__}: {error}')
        raise
    finally:
        try:
            if client:
                report['client_after_sha256'] = native.digest(client)
                if report['client_after_sha256'] != report['client_before']['sha256']:
                    raise AssertionError('justrust candidate identity changed')
            report['compiler_after'] = native.identity(compiler)
            report['cargo_after_sha256'] = native.digest(cargo)
            if report.get('compiler_before') != report['compiler_after'] or report.get('cargo_before', {}).get('sha256') != report['cargo_after_sha256']:
                raise AssertionError('Candidate identity changed')
            if SOURCE.read_bytes() != original:
                raise AssertionError('Original shipping source changed during capture')
        except Exception as error:
            report.update(verdict='failed', identity_error=str(error))
        (root / 'receipt.json').write_text(json.dumps(report, indent=2) + '\n')
    return report


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--compiler', type=Path, required=True)
    parser.add_argument('--cargo', type=Path, required=True)
    parser.add_argument('--justrust', type=Path, help='Explicit immutable client candidate, already running on an authenticated hosted guest')
    parser.add_argument('--output', type=Path, required=True)
    parser.add_argument('-n', type=int, default=1)
    parser.add_argument('--timeout', type=float, default=180)
    parser.add_argument('--no-upload', action='store_true', help='Keep receipts private locally without records-store upload')
    args = parser.parse_args()
    if args.n < 1 or not math.isfinite(args.timeout) or args.timeout <= 0:
        parser.error('Positive -n and finite positive --timeout required')
    if args.output.exists() or args.output.is_symlink():
        parser.error('--output must be a new private directory')
    try:
        check_socket_path(args.output, args.n)
    except ValueError as error:
        parser.error(str(error))
    os.umask(0o077)
    try:
        report = capture(args.compiler, args.cargo, args.output, args.n, args.timeout, justrust=args.justrust)
    finally:
        if not args.no_upload:
            dogfood.upload(args.output.absolute())
    print(json.dumps(dict(verdict=report['verdict'], receipt=str(args.output.absolute() / 'receipt.json'))))
    return int(report['verdict'] != 'passed')


if __name__ == '__main__':
    sys.exit(main())
