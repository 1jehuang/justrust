#!/usr/bin/env python3
"""Native resident guard acceptance, not a performance benchmark.

Requires an explicit already-built LLVM rustc candidate. Builds only tiny fixture
crates with that candidate, never the compiler. A proc macro writes a sentinel on
execution. Fresh compilation must execute it exactly once, resident compilation
must reject it before execution, and the same worker must then recover. Unit-test
mocks are not native acceptance evidence.
"""
import argparse
import base64
from contextlib import contextmanager
import json
import os
from pathlib import Path
import subprocess
import sys
import time

import stable_resident as native

MARKER_ENV = 'JUSTRUST_RESIDENT_GUARD_MARKER'
PROC_DIAGNOSTIC = 'native resident rustc does not support proc-macro crates'
MACRO = r'''extern crate proc_macro;
use proc_macro::TokenStream;
use std::io::Write;
#[proc_macro]
pub fn guarded(_: TokenStream) -> TokenStream {
    let marker = std::env::var("JUSTRUST_RESIDENT_GUARD_MARKER").expect("missing marker path");
    let mut file = std::fs::OpenOptions::new().create(true).append(true).open(marker).unwrap();
    file.write_all(b"executed\n").unwrap();
    "fn generated() -> u64 { 37 }".parse().unwrap()
}
'''
CONSUMER = 'guard_macro::guarded!();\nfn main() { println!("{}", generated()); }\n'
RECOVERY = 'fn main() { println!("41"); }\n'


@contextmanager
def marker_environment(path):
    """Resident inherits this one fixed value at launch. Restore caller environment."""
    previous = os.environ.get(MARKER_ENV)
    os.environ[MARKER_ENV] = str(path)
    try:
        yield
    finally:
        if previous is None:
            os.environ.pop(MARKER_ENV, None)
        else:
            os.environ[MARKER_ENV] = previous


def require(value, message, record=None):
    if value:
        return
    if record is not None:
        for stream in ('stdout_b64', 'stderr_b64'):
            if stream in record:
                message += '\n' + base64.b64decode(record[stream]).decode(errors='replace')
    raise AssertionError(message)


def marker_snapshot(path):
    return dict(path=str(path), exists=path.exists(),
                bytes_b64=native.encoded(path.read_bytes()) if path.exists() else None)


def expect_runtime(record, expected):
    require(native.observable(record) == dict(status=0, stdout_b64=native.encoded(expected), stderr_b64=''),
            'Incorrect fixture runtime output', record)


def reject_request(compiler, root, cwd, args, diagnostic, timeout):
    """Malformed/unsupported request is fatal to this worker, unlike a compile error."""
    worker = native.Resident(compiler, root, cwd, True, timeout)
    start = time.monotonic()
    row = dict(pid=worker.process.pid, command=worker.command, args=args,
               diagnostic=diagnostic, memory_before=native.memory(worker.process.pid))
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
        row['elapsed_seconds'] = time.monotonic() - start
        row['counters_available'] = 'unexpected_response' in row
        row['stdout_b64'] = native.encoded((root / 'resident.stdout').read_bytes())
        row['stderr_b64'] = native.encoded((root / 'resident.stderr').read_bytes())
        # Always retain rejected lifecycle evidence, even if its validation fails.
        (root / 'lifecycle.json').write_text(json.dumps(row, indent=2) + '\n')
    require('unexpected_response' not in row and 'protocol_error' in row,
            'Unsupported request unexpectedly acknowledged', row)
    require(row['exit_status'] == 1, 'Unsupported request did not exit with status 1', row)
    require(diagnostic.encode() in base64.b64decode(row['stderr_b64']),
            'Missing unsupported-request diagnostic', row)
    return row


def capture(compiler, root, timeout=180):
    compiler, root = Path(compiler).resolve(), Path(root).resolve()
    report = dict(schema=1, scope='native-stable-rustc-guard-acceptance-not-performance',
                  verdict='incomplete', observations={},
                  retention_scope='Initial scalar-safe query values, not retained GlobalCtxt',
                  marker_rule='Unique paths in new private evidence directories, never delete marker data')
    try:
        report['identity_before'] = native.identity(compiler)
        fresh = root / 'fresh'
        resident_dir = root / 'resident'
        for directory in (fresh, resident_dir):
            directory.mkdir(mode=0o700)
        source = root / 'consumer.rs'
        binary = root / 'consumer'
        macro_source = root / 'guard_macro.rs'
        dylib = root / 'libguard_macro.so'
        macro_source.write_text(MACRO)
        source.write_text(CONSUMER)
        report['sources'] = {name: dict(text=path.read_text(), sha256=native.digest(path))
                             for name, path in (('macro', macro_source), ('consumer', source))}
        build = report['observations']['macro_build'] = native.run(
            [compiler, macro_source, '--crate-name=guard_macro', '--crate-type=proc-macro',
             '--edition=2024', '-Zcodegen-backend=llvm', '-Cdebuginfo=0', '-o', dylib],
            timeout, cwd=root, env=native.compiler_env())
        require(build['status'] == 0, 'Proc-macro fixture build failed', build)
        report['macro_dylib_sha256'] = native.digest(dylib)
        # No incremental cache: the fresh positive control must really execute the macro.
        args = [str(source), '--crate-name=resident_guard_consumer', '--edition=2024',
                '-Zcodegen-backend=llvm', '-Cdebuginfo=0', '--extern',
                f'guard_macro={dylib}', '-o', str(binary)]
        report['consumer_args'] = args
        fresh_marker = fresh / 'macro-executions'
        resident_marker = resident_dir / 'macro-executions'
        require(not fresh_marker.exists() and not resident_marker.exists(), 'Marker path is not fresh')
        env = native.compiler_env()
        env[MARKER_ENV] = str(fresh_marker)
        fresh_row = report['observations']['fresh'] = {}
        fresh_row['compile'] = native.run([compiler, *args], timeout, cwd=root, env=env)
        fresh_row['marker'] = marker_snapshot(fresh_marker)
        require(fresh_row['compile']['status'] == 0, 'Fresh consumer compilation failed', fresh_row['compile'])
        require(fresh_marker.exists() and fresh_marker.read_bytes() == b'executed\n',
                'Fresh positive control did not execute macro exactly once')
        fresh_row['artifact_sha256'] = native.digest(binary)
        fresh_row['runtime'] = native.run([binary], timeout)
        expect_runtime(fresh_row['runtime'], b'37\n')
        binary.unlink()  # Owned generated artifact inside this run's new private directory.
        resident_row = report['observations']['resident'] = {}
        with marker_environment(resident_marker):
            worker = native.Resident(compiler, resident_dir, root, True, timeout)
        resident_row.update(pid=worker.process.pid, command=worker.command)
        try:
            rejected = resident_row['rejected_compile'] = worker.compile(args)
            resident_row['marker_after_rejection'] = marker_snapshot(resident_marker)
            require(rejected['status'] == 1, 'Resident did not return an ordinary compile error', rejected)
            require(PROC_DIAGNOSTIC.encode() in base64.b64decode(rejected['stderr_b64']),
                    'Resident proc-macro rejection diagnostic missing', rejected)
            require(not resident_marker.exists(), 'Resident executed proc macro before rejecting it')
            require(not binary.exists(), 'Rejected compile produced an executable')
            source.write_text(RECOVERY)
            resident_row['recovery_source'] = dict(text=RECOVERY, sha256=native.digest(source))
            recovered = resident_row['recovered_compile'] = worker.compile(args)
            require(recovered['pid'] == rejected['pid'] == worker.process.pid,
                    'Resident recovery changed process identity')
            require(recovered['status'] == 0, 'Resident failed recovery after guard error', recovered)
            resident_row['artifact_sha256'] = native.digest(binary)
            resident_row['runtime'] = native.run([binary], timeout)
            expect_runtime(resident_row['runtime'], b'41\n')
            resident_row['marker_after_recovery'] = marker_snapshot(resident_marker)
            require(not resident_marker.exists(), 'Macro executed during clean-source recovery')
        finally:
            failing = sys.exc_info()[0] is not None
            try:
                worker.close()
            except Exception as error:
                resident_row['shutdown_error'] = str(error)
                if not failing:
                    raise
            finally:
                resident_row['exit_status'] = worker.process.returncode
                resident_row['marker_after_exit'] = marker_snapshot(resident_marker)
        require(not resident_marker.exists(), 'Resident macro marker appeared by worker shutdown')
        report['request_guards'] = {}
        for name, extra, diagnostic in (
                ('backend', ['-Zcodegen-backend=definitely-not-a-backend'], 'only the owned LLVM backend'),
                ('target', ['--target=definitely-not-a-target'], 'only the host target')):
            directory = root / f'reject-{name}'
            directory.mkdir(mode=0o700)
            report['request_guards'][name] = reject_request(
                compiler, directory, root, [str(source), *extra], diagnostic, timeout)
        report['identity_after'] = native.identity(compiler)
        require(report['identity_before'] == report['identity_after'], 'Compiler identity changed during acceptance')
        report['verdict'] = 'passed'
        return report
    except BaseException as error:
        report['verdict'] = 'failed'
        report['error'] = f'{type(error).__name__}: {error}'
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
    if args.timeout <= 0:
        parser.error('timeout must be positive')
    os.umask(0o077)
    args.output.mkdir(parents=True, mode=0o700, exist_ok=False)
    print('Private raw evidence: ' + str(args.output.resolve()), flush=True)
    try:
        report = capture(args.compiler, args.output, args.timeout)
        print(json.dumps(dict(verdict=report['verdict'], scope=report['scope']), indent=2))
    finally:
        receipt = args.output.resolve() / 'receipt.json'
        if receipt.exists():
            try:
                result = subprocess.run(['justrust', 'records', 'put', 'bench', args.output.resolve().name,
                                         str(receipt)], capture_output=True, text=True)
                print(result.stdout, end='')
                if result.returncode:
                    print('Record upload failed, private receipt retained: ' + result.stderr)
            except OSError as error:
                print('Record upload failed, private receipt retained: ' + str(error))


if __name__ == '__main__':
    main()
