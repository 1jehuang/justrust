#!/usr/bin/env python3
"""Nonincremental native owned-context correctness, not typed reuse or speed.

Uses an explicitly supplied frozen compiler without building or installing it.
Fresh and owned compile identical ordinary source files with fixed argv/cwd/env.
Raw streams and completion frames are retained, including on failed acceptance.
"""
import argparse
import hashlib
import json
import os
from pathlib import Path
import selectors
import subprocess
import sys
import time
import uuid

import stable_resident as shared


def environment():
    env = shared.compiler_env()
    # Do not inherit gates from another experiment.
    for key in list(env):
        if key.startswith('JUSTRUST_RUSTC_RESIDENT_'):
            del env[key]
    return env


def cases():
    root = '''mod helper;
fn root_value() -> bool { true }
fn main() {
    let nonce = std::env::args().nth(1).expect("runtime nonce");
    let helper_value: bool = helper::value();
    println!("{}:{}:{}", nonce, root_value(), helper_value);
}
'''
    helper = 'pub fn value() -> bool { true }\n'
    changed = helper.replace('{ true }', '{ false }')
    return [
        ('initial', root, helper, 'true:true'),
        ('unchanged', root, helper, 'true:true'),
        ('root-body-false', root.replace('{ true }', '{ false }'), helper, 'false:true'),
        ('submodule-body-false', root, changed, 'true:false'),
        ('signature-error', root, 'pub fn value() -> u32 { 0 }\n', None),
        ('same-pid-recovery', root, changed, 'true:false'),
        ('root-and-submodule-false', root.replace('{ true }', '{ false }'), changed, 'false:false'),
        ('restored', root, helper, 'true:true'),
    ]


def arguments(root):
    return [str(root / 'fixture.rs'), '--crate-name=stable_resident_owned_fixture',
            '--edition=2024', '-Zcodegen-backend=llvm', '-Copt-level=0',
            '-Cdebuginfo=0', '-o', str(root / 'fixture')]


def validate_owned(stats, previous=None):
    if not isinstance(stats, dict):
        raise AssertionError('Invalid owned telemetry')
    for key in ('revisions', 'contexts'):
        value = stats.get(key)
        if type(value) is not int or value < 0:
            raise AssertionError('Invalid owned counter: ' + key)
        if previous is not None and value < previous[key]:
            raise AssertionError('Owned counter decreased: ' + key)
    return dict(stats)


class OwnedResident:
    """Dedicated native mode, never a per-request subprocess fallback."""
    def __init__(self, compiler, root, timeout=180):
        self.command = [str(compiler), '--jr-resident-owned']
        self.timeout = timeout
        self.root = root
        self.stdout = (root / 'resident.stdout').open('w+b', buffering=0)
        self.stderr = (root / 'resident.stderr').open('w+b', buffering=0)
        self.transcript = (root / 'resident.jsonl').open('wb')
        read_fd, write_fd = os.pipe()
        env = environment()
        env['JUSTRUST_RUSTC_RESIDENT_FD'] = str(write_fd)
        try:
            self.process = subprocess.Popen(self.command, stdin=subprocess.PIPE,
                stdout=self.stdout, stderr=self.stderr, bufsize=0, cwd=root,
                env=env, pass_fds=(write_fd,))
        except BaseException:
            os.close(read_fd)
            self.stdout.close()
            self.stderr.close()
            self.transcript.close()
            raise
        finally:
            os.close(write_fd)
        self.response_fd = read_fd
        self.selector = selectors.DefaultSelector()
        self.selector.register(read_fd, selectors.EVENT_READ)
        self.buffer = b''
        self.sequence = 0
        self.offsets = [0, 0]
        self.args = None
        self.owned = None

    def compile(self, argv):
        argv = list(map(str, argv))
        if self.args is not None and argv != self.args:
            raise ValueError('Owned args must remain fixed')
        self.args = argv
        self.sequence += 1
        request = dict(id=self.sequence, args=argv)
        self.transcript.write(json.dumps({'request': request}).encode() + b'\n')
        self.transcript.flush()
        start = time.monotonic()
        self.process.stdin.write(json.dumps(request).encode() + b'\n')
        self.process.stdin.flush()
        while b'\n' not in self.buffer:
            remaining = self.timeout - (time.monotonic() - start)
            if remaining <= 0:
                raise TimeoutError('Owned response timed out')
            if not self.selector.select(min(remaining, .01)):
                continue
            chunk = os.read(self.response_fd, 65536)
            if not chunk:
                raise RuntimeError('Owned process closed response FD')
            self.buffer += chunk
            if len(self.buffer) > 64 * 1024 * 1024:
                raise ValueError('Owned response exceeded 64 MiB')
        line, self.buffer = self.buffer.split(b'\n', 1)
        self.transcript.write(json.dumps({'response_b64': shared.encoded(line)}).encode() + b'\n')
        self.transcript.flush()
        response = json.loads(line)
        if (type(response.get('id')) is not int or response['id'] != self.sequence or
                type(response.get('pid')) is not int or response['pid'] != self.process.pid):
            raise AssertionError('Owned response id/PID mismatch')
        if type(response.get('status')) is not int:
            raise AssertionError('Owned status must be integer')
        elapsed = response.get('elapsed_seconds')
        if type(elapsed) not in (float, int) or not 0 <= elapsed < float('inf'):
            raise AssertionError('Invalid worker elapsed_seconds')
        if 'owned' in response:
            self.owned = validate_owned(response['owned'], self.owned)
        elif self.owned is not None:
            raise AssertionError('Owned telemetry disappeared')
        for index, stream in enumerate((self.stdout, self.stderr)):
            size = os.fstat(stream.fileno()).st_size
            response[('stdout_b64', 'stderr_b64')[index]] = shared.encoded(
                os.pread(stream.fileno(), size - self.offsets[index], self.offsets[index]))
            self.offsets[index] = size
        response.update(request=request, worker_elapsed_seconds=elapsed,
                        elapsed_seconds=time.monotonic() - start)
        return response

    def close(self):
        # Same EOF/shutdown contract and retained streams as the existing harness.
        return shared.Resident.close(self)


def runtime(binary, timeout, root):
    nonce = uuid.uuid4().hex
    return dict(nonce=nonce, execution=shared.run([binary, nonce], timeout,
                                               cwd=root, env=environment()))


def validate_pair(pair, expected, pid):
    fresh, owned = pair['fresh'], pair['owned']
    if owned['compile'].get('pid') != pid:
        raise AssertionError('Owned PID changed across revisions')
    if any(row['compile'].get('timed_out', False) for row in pair.values()):
        raise AssertionError('Compiler timed out')
    if shared.observable(fresh['compile']) != shared.observable(owned['compile']):
        raise AssertionError('Exact compiler status/stdout/diagnostics differ')
    for row in pair.values():
        if expected is None:
            if row['compile']['status'] != 1 or 'runtime' in row:
                raise AssertionError('Expected ordinary signature error, no runtime')
        else:
            if row['compile']['status'] != 0:
                raise AssertionError('Unexpected compile failure')
            execution = row['runtime']['execution']
            output = row['runtime']['nonce'] + ':' + expected + '\n'
            if execution.get('timed_out', False) or shared.observable(execution) != dict(
                    status=0, stdout_b64=shared.encoded(output.encode()), stderr_b64=''):
                raise AssertionError('Incorrect runtime output')
    if expected is not None and fresh['runtime']['nonce'] == owned['runtime']['nonce']:
        raise AssertionError('Runtime executions must use distinct nonces')


def capture(compiler, root, timeout=180):
    compiler, root = Path(compiler).resolve(), Path(root).resolve()
    report = dict(schema=1, scope='native-owned-context-correctness-nonincremental',
        verdict='incomplete', observations=[],
        claim='Same-PID source edit/error/recovery correctness only. Not evidence of '
              'live typed-cache reuse, retained query hits, speedup, hosted or end-to-end performance.')
    worker = None
    try:
        report['identity_before'] = shared.identity(compiler)
        argv = arguments(root)
        report['argv'] = argv
        report['cwd'] = str(root)
        fixed_env = environment()
        report['environment_sha256'] = hashlib.sha256(
            json.dumps(fixed_env, sort_keys=True).encode()).hexdigest()
        worker = OwnedResident(compiler, root, timeout)
        report['session'] = dict(pid=worker.process.pid, command=worker.command)
        workload = cases()
        if not 1 <= len(workload) <= 12:
            raise AssertionError('Owned workload exceeds revision bound')
        for name, source, helper, expected in workload:
            (root / 'fixture.rs').write_text(source)
            (root / 'helper.rs').write_text(helper)
            row = dict(name=name, source=source, helper=helper, expected=expected, modes={})
            report['observations'].append(row)
            for mode in ('fresh', 'owned'):
                if environment() != fixed_env:
                    raise AssertionError('Compiler environment changed during experiment')
                binary = root / 'fixture'
                binary.unlink(missing_ok=True)
                record = row['modes'][mode] = {}
                record['compile'] = (shared.run([compiler, *argv], timeout, cwd=root,
                    env=environment()) if mode == 'fresh' else worker.compile(argv))
                if record['compile']['status'] == 0:
                    record['artifact_sha256'] = shared.digest(binary)
                    record['runtime'] = runtime(binary, timeout, root)
            validate_pair(row['modes'], expected, worker.process.pid)
        report['session']['owned'] = worker.owned
        closing, worker = worker, None
        closing.close()
        report['identity_after'] = shared.identity(compiler)
        if report['identity_before'] != report['identity_after']:
            raise AssertionError('Compiler identity changed')
        report['verdict'] = 'passed'
        return report
    except BaseException as error:
        report.update(verdict='failed', error=f'{type(error).__name__}: {error}')
        raise
    finally:
        failing = sys.exc_info()[0] is not None
        shutdown_error = None
        if worker is not None:
            try:
                worker.close()
            except Exception as error:
                shutdown_error = error
                report.update(verdict='failed', shutdown_error=str(error))
        (root / 'receipt.json').write_text(json.dumps(report, indent=2) + '\n')
        if shutdown_error is not None and not failing:
            raise shutdown_error


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--compiler', type=Path, required=True,
                        help='Explicit frozen native compiler path')
    parser.add_argument('--output', type=Path, required=True, help='New private evidence directory')
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
                uploaded = subprocess.run(['justrust', 'records', 'put', 'bench',
                    args.output.resolve().name, str(receipt)], capture_output=True, text=True)
                print(uploaded.stdout, end='')
                if uploaded.returncode:
                    print('Record upload failed, private receipt retained: ' + uploaded.stderr)
            except OSError as error:
                print('Record upload failed, private receipt retained: ' + str(error))


if __name__ == '__main__':
    main()
