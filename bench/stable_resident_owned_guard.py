#!/usr/bin/env python3
"""Native --jr-resident-owned safety acceptance, never a performance benchmark.

Supply a fixed, already-built candidate. Only fixture crates are compiled. Mock
unit tests are not native acceptance evidence. No compiler is built or installed.
"""
import argparse
import base64
import json
import os
from pathlib import Path
import subprocess
import sys

import stable_resident_owned as owned
import stable_resident_guard as guard

native = owned.shared
require = guard.require


def reject_request(compiler, root, args, diagnostic, timeout):
    """Unsupported argv kills its own worker, with no completion frame."""
    worker = owned.OwnedResident(compiler, root, timeout)
    row = dict(pid=worker.process.pid, command=worker.command, args=args,
               diagnostic=diagnostic, behavior='fatal protocol rejection, no completion frame')
    try:
        try:
            row['unexpected_response'] = worker.compile(args)
        except RuntimeError as error:
            row['protocol_error'] = str(error)
    finally:
        try:
            worker.close()
        except RuntimeError as error:
            row['shutdown_error'] = str(error)
        row['exit_status'] = worker.process.returncode
        for stream in ('stdout', 'stderr'):
            row[stream + '_b64'] = native.encoded((root / ('resident.' + stream)).read_bytes())
        (root / 'lifecycle.json').write_text(json.dumps(row, indent=2) + '\n')
    require('unexpected_response' not in row and 'protocol_error' in row,
            'Unsupported owned request unexpectedly acknowledged', row)
    require(row['exit_status'] == 1, 'Unsupported owned request must exit 1', row)
    require(diagnostic.encode() in base64.b64decode(row['stderr_b64']),
            'Missing unsupported owned request diagnostic', row)
    return row


def capture(compiler, root, timeout=180):
    compiler, root = Path(compiler).resolve(), Path(root).resolve()
    report = dict(schema=1, scope='native-owned-mode-safety-guards-nonincremental',
                  verdict='incomplete', observations={},
                  claim='Safety acceptance only, not typed reuse or performance evidence.')
    try:
        report['identity_before'] = native.identity(compiler)
        source, binary = root / 'consumer.rs', root / 'consumer'
        macro, dylib = root / 'guard_macro.rs', root / 'libguard_macro.so'
        macro.write_text(guard.MACRO)
        source.write_text(guard.CONSUMER)
        report['sources'] = dict(macro=guard.MACRO, consumer=guard.CONSUMER,
                                 recovery=guard.RECOVERY)
        build = report['observations']['macro_build'] = native.run(
            [compiler, macro, '--crate-name=guard_macro', '--crate-type=proc-macro',
             '--edition=2024', '-Zcodegen-backend=llvm', '-Cdebuginfo=0', '-o', dylib],
            timeout, cwd=root, env=owned.environment())
        require(build['status'] == 0 and not build.get('timed_out'), 'Macro build failed', build)
        report['macro_dylib_sha256'] = native.digest(dylib)
        args = [str(source), '--crate-name=owned_guard_consumer', '--edition=2024',
                '-Zcodegen-backend=llvm', '-Cdebuginfo=0', '--extern',
                f'guard_macro={dylib}', '-o', str(binary)]
        report.update(argv=args, cwd=str(root))
        fresh_dir, resident_dir = root / 'fresh', root / 'resident'
        fresh_dir.mkdir(mode=0o700)
        resident_dir.mkdir(mode=0o700)
        fresh_marker, marker = fresh_dir / 'macro-executions', resident_dir / 'macro-executions'
        require(not fresh_marker.exists() and not marker.exists(), 'Marker paths must be fresh')
        env = owned.environment()
        env[guard.MARKER_ENV] = str(fresh_marker)
        fresh = report['observations']['fresh'] = {}
        fresh['compile'] = native.run([compiler, *args], timeout, cwd=root, env=env)
        fresh['marker'] = guard.marker_snapshot(fresh_marker)
        require(fresh['compile']['status'] == 0 and not fresh['compile'].get('timed_out'),
                'Fresh positive control failed', fresh['compile'])
        require(fresh_marker.exists() and fresh_marker.read_bytes() == b'executed\n',
                'Fresh positive control must execute macro exactly once')
        fresh['artifact_sha256'] = native.digest(binary)
        fresh['runtime'] = native.run([binary], timeout, cwd=root, env=env)
        guard.expect_runtime(fresh['runtime'], b'37\n')
        binary.unlink()
        row = report['observations']['owned'] = {}
        # OwnedResident uses its root as cwd. Absolute frozen argv permits separate
        # stream directories, and both revisions use this exact same worker/cwd.
        with guard.marker_environment(marker):
            worker = owned.OwnedResident(compiler, resident_dir, timeout)
        row.update(pid=worker.process.pid, command=worker.command, cwd=str(resident_dir))
        try:
            rejected = row['rejected_compile'] = worker.compile(args)
            row['marker_after_rejection'] = guard.marker_snapshot(marker)
            require(rejected['status'] == 1, 'Expected ordinary owned macro error', rejected)
            require(guard.PROC_DIAGNOSTIC.encode() in base64.b64decode(rejected['stderr_b64']),
                    'Missing owned macro guard diagnostic', rejected)
            require(not marker.exists(), 'Owned worker executed macro before rejection')
            require(not binary.exists(), 'Rejected request produced executable')
            source.write_text(guard.RECOVERY)
            recovered = row['recovered_compile'] = worker.compile(args)
            require(recovered['pid'] == rejected['pid'] == worker.process.pid,
                    'Owned recovery changed PID')
            require(recovered['status'] == 0, 'Owned recovery failed', recovered)
            row['artifact_sha256'] = native.digest(binary)
            row['runtime'] = native.run([binary], timeout, cwd=resident_dir, env=owned.environment())
            guard.expect_runtime(row['runtime'], b'41\n')
            row['marker_after_recovery'] = guard.marker_snapshot(marker)
            require(not marker.exists(), 'Recovery executed macro')
        finally:
            failing = sys.exc_info()[0] is not None
            try:
                worker.close()
            except Exception as error:
                row['shutdown_error'] = str(error)
                if not failing:
                    raise
            finally:
                row['exit_status'] = worker.process.returncode
                row['marker_after_exit'] = guard.marker_snapshot(marker)
        require(not marker.exists(), 'Macro executed by worker shutdown')
        report['request_guards'] = {}
        for name, extra, diagnostic in (
                ('backend', ['-Zcodegen-backend=definitely-not-a-backend'], 'only the owned LLVM backend'),
                ('target', ['--target=definitely-not-a-target'], 'only the host target'),
                ('incremental', ['-Cincremental=' + str(root / 'incremental-cache')],
                 'resident owned batch incremental persistence requires an active-revision source-file view')):
            directory = root / ('reject-' + name)
            directory.mkdir(mode=0o700)
            report['request_guards'][name] = reject_request(
                compiler, directory, [str(source), *extra], diagnostic, timeout)
        report['identity_after'] = native.identity(compiler)
        require(report['identity_before'] == report['identity_after'], 'Compiler identity changed')
        report['verdict'] = 'passed'
        return report
    except BaseException as error:
        report.update(verdict='failed', error=f'{type(error).__name__}: {error}')
        raise
    finally:
        if 'identity_after' not in report:
            try:
                report['identity_after'] = native.identity(compiler)
            except Exception as error:
                report['identity_error'] = str(error)
        (root / 'receipt.json').write_text(json.dumps(report, indent=2) + '\n')


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--compiler', required=True, type=Path)
    parser.add_argument('--output', required=True, type=Path, help='New private evidence directory')
    parser.add_argument('--timeout', type=float, default=180)
    args = parser.parse_args()
    if not 0 < args.timeout < float('inf'):
        parser.error('timeout must be finite and positive')
    os.umask(0o077)
    args.output.mkdir(mode=0o700, parents=True, exist_ok=False)
    print('Private raw evidence: ' + str(args.output.resolve()), flush=True)
    try:
        report = capture(args.compiler, args.output, args.timeout)
        print(json.dumps(dict(verdict=report['verdict'], claim=report['claim']), indent=2))
    finally:
        receipt = args.output.resolve() / 'receipt.json'
        if receipt.exists():
            try:
                result = subprocess.run(['justrust', 'records', 'put', 'bench',
                    args.output.resolve().name, str(receipt)], capture_output=True, text=True)
                print(result.stdout, end='')
                if result.returncode:
                    print('Record upload failed, private receipt retained: ' + result.stderr)
            except OSError as error:
                print('Record upload failed, private receipt retained: ' + str(error))


if __name__ == '__main__':
    main()
