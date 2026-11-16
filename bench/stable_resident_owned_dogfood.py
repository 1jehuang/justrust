#!/usr/bin/env python3
"""Direct native-owned shipping-module dogfood, nonincremental correctness only.

Uses private source copies and an explicitly supplied frozen compiler. Does not
build/install compilers, claim typed-query reuse, or measure a speedup.
"""
import argparse
import hashlib
import json
import os
from pathlib import Path
import sys
import uuid

import stable_resident_owned as owned
import stable_resident_dogfood as shared

native = owned.shared
MODES = ('fresh', 'owned')


def arguments(root):
    return [str(root / 'wrapper.rs'), '--crate-name=stable_resident_owned_dogfood',
            '--edition=2024', '--test', '-Zcodegen-backend=llvm', '-Copt-level=0',
            '-Cdebuginfo=0', '-o', str(root / 'tests')]


def validate_pair(modes, expectation, pid):
    if modes['owned']['compile'].get('pid') != pid:
        raise AssertionError('Owned worker PID changed')
    reference = native.observable(modes['fresh']['compile'])
    if reference['status'] != (1 if expectation == 'compile-fail' else 0):
        raise AssertionError('Fresh compile status differs from expectation')
    for mode in MODES:
        row = modes[mode]
        if row['compile'].get('timed_out'):
            raise AssertionError('Compiler timed out')
        if native.observable(row['compile']) != reference:
            raise AssertionError('Exact compiler status/diagnostics differ')
        if expectation == 'compile-fail':
            if 'runtime' in row or 'artifact_sha256' in row:
                raise AssertionError('Type-error stage ran stale executable')
        else:
            shared.statuses(row['runtime'], row['nonce'], expectation)
    if expectation != 'compile-fail' and modes['fresh']['nonce'] == modes['owned']['nonce']:
        raise AssertionError('Fresh runtime nonce must differ from owned nonce')


def capture(compiler, source_path, root, repetitions=3, timeout=180):
    if repetitions < 1 or not 0 < timeout < float('inf'):
        raise ValueError('Repetitions and finite timeout must be positive')
    compiler, source_path, root = map(lambda p: Path(p).resolve(), (compiler, source_path, root))
    report = dict(schema=1, verdict='incomplete', modes=list(MODES), observations=[], sessions=[],
                  scope='native-owned-shipping-remote_sync_queue-module-only-nonincremental',
                  claim='Correctness dogfood only. No typed-query-hit, speedup, full-client or production claim.',
                  source_path=str(source_path), repetitions=repetitions,
                  original_tests=list(shared.ORIGINAL_TESTS), added_test=shared.PROBE)
    try:
        original = source_path.read_bytes()
        report['source_sha256_before'] = hashlib.sha256(original).hexdigest()
        report['identity_before'] = native.identity(compiler)
        # Include signature edit/recovery as source-only revisions, never argv edits.
        matrix = shared.variants(original.decode(), frontend=True)
        for rep in range(repetitions):
            work = root / f'repetition-{rep}'
            work.mkdir(mode=0o700)
            (work / 'wrapper.rs').write_text(shared.WRAPPER)
            report['wrapper_sha256'] = native.digest(work / 'wrapper.rs')
            argv = arguments(work)
            worker = owned.OwnedResident(compiler, work, timeout)
            session = dict(repetition=rep, pid=worker.process.pid, command=worker.command,
                           argv=argv, cwd=str(work))
            report['sessions'].append(session)
            try:
                for name, source, expectation in matrix:
                    row = dict(repetition=rep, name=name, expectation=expectation,
                               source_sha256=hashlib.sha256(source.encode()).hexdigest(), modes={})
                    report['observations'].append(row)
                    for mode in MODES:
                        (work / 'remote_sync_queue.rs').write_text(source)
                        binary = work / 'tests'
                        binary.unlink(missing_ok=True)
                        result = row['modes'][mode] = {}
                        result['compile'] = (native.run([compiler, *argv], timeout, cwd=work,
                            env=owned.environment()) if mode == 'fresh' else worker.compile(argv))
                        if result['compile']['status'] == 0:
                            result['artifact_sha256'] = native.digest(binary)
                            result['nonce'] = uuid.uuid4().hex
                            env = owned.environment()
                            env[shared.NONCE_KEY] = result['nonce']
                            result['runtime'] = native.run([binary, '--nocapture', '--test-threads=1',
                                '--color=never'], timeout, cwd=work, env=env)
                            result['named_statuses'] = shared.statuses(
                                result['runtime'], result['nonce'], expectation)
                    validate_pair(row['modes'], expectation, worker.process.pid)
                session['owned'] = worker.owned
            finally:
                failing = sys.exc_info()[0] is not None
                session['owned'] = worker.owned
                try:
                    worker.close()
                except Exception as error:
                    session['shutdown_error'] = str(error)
                    if not failing:
                        raise
                finally:
                    session['exit_status'] = worker.process.returncode
        report['identity_after'] = native.identity(compiler)
        if report['identity_before'] != report['identity_after']:
            raise AssertionError('Compiler identity changed')
        report['source_sha256_after'] = native.digest(source_path)
        if report['source_sha256_before'] != report['source_sha256_after']:
            raise AssertionError('Original shipping source changed')
        report['source_unchanged'] = True
        report['verdict'] = 'passed'
        return report
    except BaseException as error:
        report.update(verdict='failed', error=f'{type(error).__name__}: {error}')
        raise
    finally:
        try:
            report['source_sha256_after'] = native.digest(source_path)
            report['source_unchanged'] = report.get('source_sha256_before') == report['source_sha256_after']
            if not report['source_unchanged']:
                report.update(verdict='failed', source_error='Original source hash changed')
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
    parser.add_argument('--compiler', required=True, type=Path)
    parser.add_argument('--source', type=Path, default=shared.DEFAULT_SOURCE)
    parser.add_argument('--output', required=True, type=Path, help='New private evidence directory')
    parser.add_argument('-n', '--repetitions', type=int, default=3)
    parser.add_argument('--timeout', type=float, default=180)
    args = parser.parse_args()
    if args.repetitions < 1 or not 0 < args.timeout < float('inf'):
        parser.error('Repetitions and finite timeout must be positive')
    os.umask(0o077)
    args.output.mkdir(mode=0o700, parents=True, exist_ok=False)
    print('Private original evidence: ' + str(args.output.resolve()), flush=True)
    try:
        report = capture(args.compiler, args.source, args.output, args.repetitions, args.timeout)
        print(json.dumps({key: report[key] for key in ('verdict', 'claim', 'source_unchanged')}, indent=2))
    finally:
        shared.upload(args.output.resolve())


if __name__ == '__main__':
    main()
