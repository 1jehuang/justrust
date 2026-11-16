#!/usr/bin/env python3
"""Limited shipping-module dogfood for native stable rustc, not the full client.

Never builds a compiler or modifies the supplied source. Raw streams include test
harness timing diagnostics, so runtime comparison uses validated named statuses,
not byte equivalence. No speedup or whole-compiler-residency claims are made.
"""
import argparse
import base64
import hashlib
import json
import os
from pathlib import Path
import re
import sys
import time
import uuid

import stable_resident as native

DEFAULT_SOURCE = Path(__file__).resolve().parents[1] / 'src' / 'remote_sync_queue.rs'
ORIGINAL_TESTS = (
    'remote_sync_queue::tests::queued_foreground_precedes_remaining_background_roots',
    'remote_sync_queue::tests::foreground_batch_contention_acceptance',
    'remote_sync_queue::tests::poisoned_foreground_acquisition_does_not_leave_waiter_registered',
)
PROBE = 'fresh_nonce_probe'
NONCE_KEY = 'JUSTRUST_DOGFOOD_RUNTIME_NONCE'
WRAPPER = '''#![allow(dead_code)]
#[path="remote_sync_queue.rs"]
mod remote_sync_queue;
#[test]
fn fresh_nonce_probe() {
    let marker = std::env::var("JUSTRUST_DOGFOOD_RUNTIME_NONCE").unwrap();
    assert!(!marker.is_empty());
    eprintln!("JR_DOGFOOD_NONCE={}", marker);
}
'''


def variants(source):
    body = 'Duration::from_millis(1)'
    assertion = 'assert_eq!(queue.foreground_waiters.load(Ordering::SeqCst), 0);'
    if source.count(body) != 1 or source.count(assertion) != 2:
        raise ValueError('Supplied shipping source does not match the reviewed edit anchors')
    if len(re.findall(r'#\[test\]', source)) != 3:
        raise ValueError('Expected exactly three original shipping-module tests')
    prefix, suffix = source.rsplit(assertion, 1)
    failing = prefix + assertion.replace(', 0);', ', 1);') + suffix
    type_anchor = 'Self {\n            inner: Mutex::new(value),'
    if source.count(type_anchor) != 1:
        raise ValueError('Expected unique type-error anchor')
    wrong_type = source.replace(type_anchor, 'Self {\n            inner: "deliberate type error",')
    return [('baseline', source, 'pass'), ('unchanged', source, 'pass'),
            ('body-edit', source.replace(body, 'Duration::from_millis(2)'), 'pass'),
            ('revert', source, 'pass'), ('assertion-failure', failing, 'test-fail'),
            ('recovery', source, 'pass'), ('type-error', wrong_type, 'compile-fail'),
            ('type-recovery', source, 'pass')]


def arguments(root, cache):
    return [str(root / 'wrapper.rs'), '--crate-name=stable_resident_dogfood', '--edition=2024',
            '--test', '-Zcodegen-backend=llvm', '-Copt-level=0', '-Cdebuginfo=0',
            '-Zincremental-verify-ich', '-C', f'incremental={cache}', '-o', str(root / 'tests')]


def statuses(record, nonce, expectation):
    if record.get('timed_out'):
        raise AssertionError('Test process timed out')
    stdout = base64.b64decode(record['stdout_b64']).decode('utf-8', errors='strict')
    stderr = base64.b64decode(record['stderr_b64']).decode('utf-8', errors='strict')
    pairs = re.findall(r'^test (\S+) \.\.\. (ok|FAILED|ignored)$', stdout, re.MULTILINE)
    names = [name for name, _ in pairs]
    expected = {name: 'ok' for name in (*ORIGINAL_TESTS, PROBE)}
    if expectation == 'test-fail':
        expected[ORIGINAL_TESTS[2]] = 'FAILED'
    if len(pairs) != 4 or len(set(names)) != 4 or dict(pairs) != expected:
        raise AssertionError(f'Unexpected full test names/statuses: {pairs!r}')
    expected_code = 101 if expectation == 'test-fail' else 0
    if record['status'] != expected_code:
        raise AssertionError(f'Unexpected test exit status: {record["status"]}')
    markers = re.findall(r'^JR_DOGFOOD_NONCE=(.*)$', stderr, re.MULTILINE)
    if markers != [nonce]:
        raise AssertionError('Runtime nonce absent, duplicated, or stale')
    passed, failed = (3, 1) if expectation == 'test-fail' else (4, 0)
    outcome = 'FAILED' if failed else 'ok'
    summary = f'test result: {outcome}. {passed} passed; {failed} failed; 0 ignored; 0 measured; 0 filtered out;'
    if summary not in stdout:
        raise AssertionError('Test harness summary disagrees with named statuses')
    return expected


def validate_pair(modes, expectation):
    reference = native.observable(modes['fresh']['compile'])
    wanted_status = 1 if expectation == 'compile-fail' else 0
    if reference['status'] != wanted_status:
        raise AssertionError(f'Fresh compile status is not expected {wanted_status}: {reference}')
    for mode in native.MODES:
        row = modes[mode]
        if native.observable(row['compile']) != reference:
            raise AssertionError(f'Exact compiler diagnostics/status differ for {mode}')
        if row['compile'].get('timed_out'):
            raise AssertionError('Compiler process timed out')
        if expectation == 'compile-fail':
            if 'runtime' in row or 'artifact_sha256' in row:
                raise AssertionError('Type-error stage executed a stale test binary')
        else:
            # Independently validate the fresh oracle before comparing candidates.
            fresh = statuses(modes['fresh']['runtime'], modes['fresh']['nonce'], expectation)
            actual = statuses(row['runtime'], row['nonce'], expectation)
            if actual != fresh:
                raise AssertionError(f'Runtime named test statuses differ for {mode}')


def upload(root):
    receipt = root / 'receipt.json'
    if not receipt.exists():
        return
    try:
        row = native.run(['justrust', 'records', 'put', 'bench', root.name, str(receipt)], timeout=180)
        (root / 'upload.json').write_text(json.dumps(row, indent=2) + '\n')
        print(base64.b64decode(row['stdout_b64']).decode(errors='replace'), end='')
        if row['status']:
            print('Upload failed, private receipt retained: ' +
                  base64.b64decode(row['stderr_b64']).decode(errors='replace'), file=sys.stderr)
    except OSError as error:
        (root / 'upload.json').write_text(json.dumps({'error': str(error)}) + '\n')
        print('Upload unavailable, private receipt retained: ' + str(error), file=sys.stderr)


def capture(compiler, source_path, root, repetitions=3, timeout=180):
    if repetitions < 1 or timeout <= 0:
        raise ValueError('Repetitions and timeout must be positive')
    compiler, source_path, root = (Path(path).resolve() for path in (compiler, source_path, root))
    report = dict(schema=1, verdict='incomplete', repetitions=repetitions,
                  scope='shipping-remote_sync_queue-module-only-not-full-client',
                  claims='Correctness dogfood only, no speed claims. Each compiler request creates a fresh GlobalCtxt.',
                  modes=list(native.MODES), source_path=str(source_path), observations=[], sessions=[],
                  original_tests=list(ORIGINAL_TESTS), added_test=PROBE,
                  memory_note='Linux KiB. Resident peak RSS is lifetime cumulative. Polling may miss brief peaks.')
    try:
        report['source_sha256_before'] = native.digest(source_path)
        original = source_path.read_bytes()
        if hashlib.sha256(original).hexdigest() != report['source_sha256_before']:
            raise AssertionError('Source changed while capturing private copy')
        matrix = variants(original.decode('utf-8'))
        report['identity_before'] = native.identity(compiler)
        work = root / 'private-workload'
        work.mkdir(mode=0o700)
        (work / 'wrapper.rs').write_text(WRAPPER)
        report['wrapper_sha256'] = native.digest(work / 'wrapper.rs')
        for rep in range(repetitions):
            repdir = root / f'repetition-{rep}'
            repdir.mkdir(mode=0o700)
            caches = {mode: repdir / mode for mode in native.MODES}
            for cache in caches.values():
                cache.mkdir(mode=0o700)
            workers, sessions = {}, {}
            try:
                for mode in native.MODES[1:]:
                    start = time.monotonic()
                    worker = native.Resident(compiler, caches[mode], work,
                                             mode == 'resident-values-on', timeout)
                    workers[mode] = worker
                    session = dict(repetition=rep, mode=mode, pid=worker.process.pid,
                                   command=worker.command, spawn_seconds=time.monotonic() - start,
                                   raw_stream_directory=str(caches[mode]))
                    sessions[mode] = session
                    report['sessions'].append(session)
                for index, (name, source, expectation) in enumerate(matrix):
                    rotation = (rep + index) % len(native.MODES)
                    row = dict(repetition=rep, name=name, expectation=expectation,
                               source_sha256=hashlib.sha256(source.encode()).hexdigest(),
                               modes={}, order=list(native.MODES[rotation:] + native.MODES[:rotation]))
                    report['observations'].append(row)
                    for mode in row['order']:
                        (work / 'remote_sync_queue.rs').write_bytes(source.encode())
                        binary = work / 'tests'
                        binary.unlink(missing_ok=True)
                        argv = arguments(work, caches[mode])
                        result = row['modes'][mode] = {}
                        result['compile'] = (native.run([compiler, *argv], timeout, cwd=work,
                                                        env=native.compiler_env()) if mode == 'fresh'
                                             else workers[mode].compile(argv))
                        if result['compile']['status'] == 0:
                            result['artifact_sha256'] = native.digest(binary)
                            nonce = uuid.uuid4().hex
                            result['nonce'] = nonce
                            env = native.compiler_env()
                            env[NONCE_KEY] = nonce
                            result['runtime'] = native.run([binary, '--nocapture', '--test-threads=1',
                                                            '--color=never'], timeout, cwd=work, env=env)
                            result['named_statuses'] = statuses(result['runtime'], nonce, expectation)
                    validate_pair(row['modes'], expectation)
                for mode, worker in workers.items():
                    sessions[mode]['final_counters'] = dict(worker.counters)
                if workers['resident-values-off'].counters['resident_hits'] != 0:
                    raise AssertionError('Values-off worker reported RAM hits')
                hits = workers['resident-values-on'].counters['resident_hits']
                sessions['resident-values-on']['live_hits_observed'] = hits > 0
            finally:
                failing = sys.exc_info()[0] is not None
                errors = []
                for mode, worker in workers.items():
                    sessions[mode]['final_counters'] = dict(worker.counters)
                    try:
                        worker.close()
                    except Exception as error:
                        sessions[mode]['shutdown_error'] = str(error)
                        errors.append(error)
                if errors and not failing:
                    raise errors[0]
        report['identity_after'] = native.identity(compiler)
        if report['identity_before'] != report['identity_after']:
            raise AssertionError('Compiler/driver identity changed')
        report['source_sha256_after'] = native.digest(source_path)
        if report['source_sha256_before'] != report['source_sha256_after']:
            raise AssertionError('Repository source changed during capture')
        report['live_hits_observed'] = any(session.get('live_hits_observed') for session in report['sessions'])
        report['retention_verdict'] = ('live-query-value-hits-observed' if report['live_hits_observed']
                                       else 'no-live-hits-observed-no-retention-claim')
        report['summary'] = native.summarize(report['observations'])
        report['verdict'] = 'passed'
        return report
    except BaseException as error:
        report['verdict'] = 'failed'
        report['error'] = f'{type(error).__name__}: {error}'
        raise
    finally:
        try:
            report['source_sha256_after'] = native.digest(source_path)
            report['source_unchanged'] = report.get('source_sha256_before') == report['source_sha256_after']
            if not report['source_unchanged']:
                report['verdict'] = 'failed'
                report['source_error'] = 'Repository source hash changed'
        except Exception as error:
            report['source_error'] = str(error)
        if 'identity_after' not in report:
            try:
                report['identity_after'] = native.identity(compiler)
            except Exception as error:
                report['identity_error'] = str(error)
        (root / 'receipt.json').write_text(json.dumps(report, indent=2) + '\n')


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--compiler', type=Path, required=True)
    parser.add_argument('--source', type=Path, default=DEFAULT_SOURCE)
    parser.add_argument('--output', type=Path, required=True, help='New private evidence directory')
    parser.add_argument('-n', '--repetitions', type=int, default=3)
    parser.add_argument('--timeout', type=float, default=180)
    args = parser.parse_args()
    if args.repetitions < 1 or args.timeout <= 0:
        parser.error('repetitions and timeout must be positive')
    os.umask(0o077)
    args.output.mkdir(mode=0o700, parents=True, exist_ok=False)
    print('Private evidence: ' + str(args.output.resolve()), flush=True)
    try:
        report = capture(args.compiler, args.source, args.output, args.repetitions, args.timeout)
        print(json.dumps({key: report[key] for key in ('verdict', 'retention_verdict', 'source_unchanged')}, indent=2))
    finally:
        upload(args.output.resolve())


if __name__ == '__main__':
    main()
