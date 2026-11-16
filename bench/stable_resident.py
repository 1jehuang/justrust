#!/usr/bin/env python3
"""Local stable-rustc resident acceptance, not hosted or end-to-end performance.

Uses an explicitly supplied compiler, never builds or installs one. Receipts retain
real commands, outputs, timings and Linux RSS observations, including failures.
The resident wire protocol uses a dedicated inherited pipe and must not be
emulated by spawning rustc per request. Unit-test fake servers are protocol tests,
not results. This measures initial scalar-safe query values, not retained GlobalCtxt.
"""
import argparse
import base64
import hashlib
import json
import os
from pathlib import Path
import selectors
import statistics
import subprocess
import sys
import tempfile
import time


def encoded(data):
    return base64.b64encode(data).decode('ascii')


def digest(path):
    h = hashlib.sha256()
    with Path(path).open('rb') as stream:
        for block in iter(lambda: stream.read(1024 * 1024), b''):
            h.update(block)
    return h.hexdigest()


def memory(pid):
    """Current and lifetime peak RSS, not a per-request allocation measurement."""
    result = {}
    try:
        for line in Path(f'/proc/{pid}/status').read_text().splitlines():
            key, _, value = line.partition(':')
            if key in ('VmRSS', 'VmHWM'):
                result[{'VmRSS': 'rss_kib', 'VmHWM': 'lifetime_peak_rss_kib'}[key]] = int(value.split()[0])
    except FileNotFoundError:
        pass
    return result


MODES = ('fresh', 'resident-values-off', 'resident-values-on')
COUNTERS = ('resident_hits', 'resident_misses', 'resident_entries')


def compiler_env(values=None):
    env = {key: value for key, value in os.environ.items()
           if key not in ('JUSTRUST_RUSTC_RESIDENT_FD', 'JUSTRUST_RUSTC_RESIDENT_VALUES',
                          'JUSTRUST_RUSTC_PHASES_FILE', 'JUSTRUST_RUSTC_EARLY_REPLY')}
    if values is not None:
        env['JUSTRUST_RUSTC_RESIDENT_VALUES'] = str(int(values))
    return env


def run(command, timeout=180, cwd=None, env=None):
    command = list(map(str, command))
    start = time.monotonic()
    with tempfile.TemporaryFile() as out, tempfile.TemporaryFile() as err:
        process = subprocess.Popen(command, stdout=out, stderr=err, cwd=cwd, env=env)
        timed_out = False
        try:
            while True:
                pid, status, usage = os.wait4(process.pid, os.WNOHANG)
                if pid:
                    process.returncode = os.waitstatus_to_exitcode(status)
                    break
                if time.monotonic() - start > timeout:
                    timed_out = True
                    process.kill()
                    _, status, usage = os.wait4(process.pid, 0)
                    process.returncode = os.waitstatus_to_exitcode(status)
                    break
                time.sleep(.002)
        finally:
            if process.returncode is None:
                process.kill()
                process.wait()
        out.seek(0)
        err.seek(0)
        return dict(command=command, status=process.returncode, timed_out=timed_out,
                    stdout_b64=encoded(out.read()), stderr_b64=encoded(err.read()),
                    elapsed_seconds=time.monotonic() - start, peak_rss_kib=usage.ru_maxrss)


def identity(compiler):
    version = run([compiler, '-vV'], env=compiler_env())
    sysroot = run([compiler, '--print', 'sysroot'], env=compiler_env())
    if version['status'] or sysroot['status']:
        raise AssertionError('Compiler identity query failed')
    root = Path(base64.b64decode(sysroot['stdout_b64']).decode().strip())
    drivers = [dict(path=str(p.resolve()), sha256=digest(p))
               for p in sorted((root / 'lib').glob('*rustc_driver*')) if p.is_file()]
    if not drivers or b'LLVM version:' not in base64.b64decode(version['stdout_b64']):
        raise AssertionError('Expected an LLVM rustc with driver identity')
    return dict(path=str(compiler), sha256=digest(compiler), drivers=drivers,
                version=observable(version), sysroot=observable(sysroot))


def observable(record):
    return {key: record[key] for key in ('status', 'stdout_b64', 'stderr_b64')}


def cases(items):
    prefix = '''#![allow(dead_code)]
trait Number { fn number(&self) -> u64; }
struct Zero;
impl Number for Zero { fn number(&self) -> u64 { 0 } }
#[inline(never)]
fn dynamic(value: &dyn Number) -> u64 { value.number() }
const fn bias() -> u64 { 0 }
const BIAS: u64 = bias();
'''
    prefix += '\n'.join(f'#[track_caller]\nfn item_{i}(v: u64) -> u64 {{ v + {i} }}' for i in range(items)) + '\n'
    source = prefix + 'fn value() -> u64 { 7 }\nfn main() { println!("{}", value() as u64 + '
    source += ' + '.join(f'item_{i}(1)' for i in range(items)) + ' + dynamic(&Zero) + BIAS); }\n'
    expected = str(7 + items * (items + 1) // 2) + '\n'
    return [('warm', source, expected), ('unchanged', source, expected),
            ('body-edit', source.replace('u64 { 7 }', 'u64 { 8 }'), str(int(expected) + 1) + '\n'),
            ('signature-edit', source.replace('value() -> u64', 'value() -> u32'), expected),
            ('type-error', source.replace('u64 { 7 }', 'u64 { "wrong" }'), None),
            ('recovery', source, expected)]


def arguments(root, cache):
    # All modes share source/output paths, avoiding diagnostic path normalization.
    return [str(root / 'fixture.rs'), '--crate-name=stable_resident_fixture', '--edition=2024',
            '-Zcodegen-backend=llvm', '-Copt-level=0', '-Cdebuginfo=0',
            '-Zincremental-verify-ich', '-C', f'incremental={cache}', '-o', str(root / 'fixture')]


def validate_pair(pair, expected):
    def fail(message):
        outputs = []
        for mode, row in pair.items():
            compile_row = row['compile']
            outputs.append(f"{mode}: status {compile_row['status']}")
            for stream in ('stdout_b64', 'stderr_b64'):
                outputs.append(base64.b64decode(compile_row[stream]).decode(errors='replace'))
        raise AssertionError(message + '\n' + '\n'.join(outputs))

    if any(observable(pair['fresh']['compile']) != observable(pair[mode]['compile']) for mode in MODES[1:]):
        fail('Exact compiler status/stdout/diagnostics differ')
    for row in pair.values():
        if expected is None:
            if row['compile']['status'] != 1:
                fail('Expected ordinary type error, not ICE/crash')
        else:
            if row['compile']['status'] != 0:
                fail('Unexpected compile failure')
            if observable(row['runtime']) != dict(status=0, stdout_b64=encoded(expected.encode()), stderr_b64=''):
                raise AssertionError('Incorrect runtime output')


class Resident:
    """One process for an entire edit/recovery sequence, no restart or fallback."""
    def __init__(self, compiler, root, cwd, values, timeout=180):
        self.command = [str(compiler), '--jr-resident']
        self.timeout = timeout
        self.stdout = (root / 'resident.stdout').open('w+b', buffering=0)
        self.stderr = (root / 'resident.stderr').open('w+b', buffering=0)
        self.transcript = (root / 'resident.jsonl').open('wb')
        read_fd, write_fd = os.pipe()
        env = compiler_env(values)
        env['JUSTRUST_RUSTC_RESIDENT_FD'] = str(write_fd)
        try:
            self.process = subprocess.Popen(self.command, stdin=subprocess.PIPE, stdout=self.stdout,
                                            stderr=self.stderr, bufsize=0, cwd=cwd, env=env,
                                            pass_fds=(write_fd,))
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
        self.counters = dict.fromkeys(COUNTERS, 0)

    def compile(self, argv):
        argv = list(map(str, argv))
        if self.args is not None and argv != self.args:
            raise ValueError('Resident args must remain fixed for its lifetime')
        self.args = argv
        self.sequence += 1
        request = dict(id=self.sequence, args=argv)
        start = time.monotonic()
        before = memory(self.process.pid)
        payload = json.dumps(request).encode() + b'\n'
        self.transcript.write(json.dumps({'request': request}).encode() + b'\n')
        self.transcript.flush()
        self.process.stdin.write(payload)
        sampled_peak = before.get('rss_kib', 0)
        self.process.stdin.flush()
        while b'\n' not in self.buffer:
            remaining = self.timeout - (time.monotonic() - start)
            if remaining <= 0:
                raise TimeoutError('Resident response timed out')
            sampled_peak = max(sampled_peak, memory(self.process.pid).get('rss_kib', 0))
            if not self.selector.select(min(remaining, .01)):
                continue
            chunk = os.read(self.response_fd, 65536)
            if not chunk:
                raise RuntimeError('Resident exited or closed response FD')
            self.buffer += chunk
            if len(self.buffer) > 64 * 1024 * 1024:
                raise ValueError('Resident response exceeded 64 MiB')
        line, self.buffer = self.buffer.split(b'\n', 1)
        self.transcript.write(json.dumps({'response_b64': encoded(line)}).encode() + b'\n')
        self.transcript.flush()
        response = json.loads(line)
        if response.get('id') != self.sequence or response.get('pid') != self.process.pid:
            raise AssertionError('Resident response id/PID mismatch')
        if type(response.get('status')) is not int:
            raise AssertionError('Resident response has no integer status')
        for key in COUNTERS:
            value = response.get(key)
            if (type(value) is not int or value < 0 or
                    (key != 'resident_entries' and value < self.counters[key])):
                raise AssertionError('Resident counters missing or decreased')
            self.counters[key] = value
        worker_elapsed = response.pop('elapsed_seconds')
        if not isinstance(worker_elapsed, (float, int)) or worker_elapsed < 0:
            raise AssertionError('Invalid worker elapsed_seconds')
        for index, stream in enumerate((self.stdout, self.stderr)):
            size = os.fstat(stream.fileno()).st_size
            response[('stdout_b64', 'stderr_b64')[index]] = encoded(
                os.pread(stream.fileno(), size - self.offsets[index], self.offsets[index]))
            self.offsets[index] = size
        response.update(elapsed_seconds=time.monotonic() - start, request=request,
                        worker_elapsed_seconds=worker_elapsed,
                        sampled_peak_rss_kib=sampled_peak,
                        memory_before=before, memory_after=memory(self.process.pid))
        return response

    def close(self):
        self.process.stdin.close()
        try:
            self.process.wait(timeout=5)
        except subprocess.TimeoutExpired:
            self.process.kill()
            self.process.wait()
            raise RuntimeError('Resident failed to exit on EOF')
        finally:
            self.selector.close()
            os.close(self.response_fd)
            self.stdout.close()
            self.stderr.close()
            self.transcript.close()
        if self.process.returncode:
            raise RuntimeError(f'Resident shutdown status {self.process.returncode}')


def summarize(observations):
    result = {}
    for name in dict.fromkeys(row['name'] for row in observations):
        result[name] = {}
        for mode in MODES:
            rows = [row['modes'][mode]['compile'] for row in observations if row['name'] == name]
            result[name][mode] = dict(median_seconds=statistics.median(r['elapsed_seconds'] for r in rows))
            if mode == 'fresh':
                result[name][mode]['max_process_peak_rss_kib'] = max(r['peak_rss_kib'] for r in rows)
            else:
                peaks = [r['memory_after']['lifetime_peak_rss_kib'] for r in rows
                         if 'lifetime_peak_rss_kib' in r.get('memory_after', {})]
                result[name][mode]['max_lifetime_peak_rss_kib'] = max(peaks) if peaks else None
    return result


def capture(compiler, root, repetitions=3, items=32, timeout=180):
    if repetitions < 1 or items < 1:
        raise ValueError('Repetitions and fixture items must be positive')
    compiler, root = Path(compiler).resolve(), Path(root).resolve()
    report = dict(schema=1, scope='local-stable-rustc-resident-not-hosted-not-end-to-end',
                  verdict='incomplete', repetitions=repetitions, items=items, modes=list(MODES),
                  retention_scope='Initial scalar-safe query values only. Each request rebuilds '
                                  'GlobalCtxt. Not retained whole compiler execution memory.',
                  memory_note='Linux KiB. Resident VmHWM is lifetime cumulative, not per-request peak. '
                              'Sampled RSS polls at 10ms and can miss brief peaks.',
                  observations=[])
    try:
        report['identity_before'] = identity(compiler)
        for rep in range(repetitions):
            repdir = root / f'repetition-{rep}'
            repdir.mkdir(mode=0o700)
            caches = {mode: repdir / mode for mode in MODES}
            for cache in caches.values():
                cache.mkdir(mode=0o700)
            workers = {}
            sessions = {}
            try:
                for mode in MODES[1:]:
                    start = time.monotonic()
                    workers[mode] = Resident(compiler, caches[mode], root,
                                             mode == 'resident-values-on', timeout)
                    sessions[mode] = dict(repetition=rep, mode=mode, pid=workers[mode].process.pid,
                                          command=workers[mode].command,
                                          spawn_seconds=time.monotonic() - start)
                    report.setdefault('sessions', []).append(sessions[mode])
                for index, (name, source, expected) in enumerate(cases(items)):
                    rotation = (rep + index) % len(MODES)
                    row = dict(repetition=rep, name=name, source=source,
                               source_sha256=hashlib.sha256(source.encode()).hexdigest(),
                               expected_stdout=expected, modes={},
                               order=list(MODES[rotation:] + MODES[:rotation]))
                    report['observations'].append(row)
                    for mode in row['order']:
                        (root / 'fixture.rs').write_text(source)
                        binary = root / 'fixture'
                        binary.unlink(missing_ok=True)
                        argv = arguments(root, caches[mode])
                        record = row['modes'][mode] = {}
                        record['compile'] = (run([compiler, *argv], timeout, cwd=root, env=compiler_env())
                                             if mode == 'fresh' else workers[mode].compile(argv))
                        if record['compile']['status'] == 0:
                            record['artifact_sha256'] = digest(binary)
                            record['runtime'] = run([binary], timeout)
                    validate_pair(row['modes'], expected)
                if workers['resident-values-off'].counters['resident_hits'] != 0:
                    raise AssertionError('Values-off worker unexpectedly reports resident hits')
                if workers['resident-values-on'].counters['resident_hits'] <= 0:
                    raise AssertionError('Values-on worker reports no resident query hits')
            finally:
                failing = sys.exc_info()[0] is not None
                shutdown_errors = []
                for mode, worker in workers.items():
                    try:
                        worker.close()
                    except Exception as error:
                        sessions[mode]['shutdown_error'] = str(error)
                        shutdown_errors.append(error)
                if shutdown_errors and not failing:
                    raise shutdown_errors[0]
        report['identity_after'] = identity(compiler)
        if report['identity_before'] != report['identity_after']:
            raise AssertionError('Compiler identity changed during experiment')
        report['summary'] = summarize(report['observations'])
        report['verdict'] = 'passed'
        return report
    except BaseException as error:
        report['verdict'] = 'failed'
        report['error'] = f'{type(error).__name__}: {error}'
        raise
    finally:
        if 'identity_after' not in report:
            try:
                report['identity_after'] = identity(compiler)
                if report.get('identity_before') != report['identity_after']:
                    report['identity_error'] = 'Compiler identity changed during experiment'
            except Exception as error:
                report['identity_error'] = str(error)
        (root / 'receipt.json').write_text(json.dumps(report, indent=2) + '\n')


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--compiler', type=Path, required=True)
    parser.add_argument('--output', type=Path, required=True, help='New private evidence directory')
    parser.add_argument('-n', '--repetitions', type=int, default=3)
    parser.add_argument('--items', type=int, default=32)
    parser.add_argument('--timeout', type=float, default=180)
    args = parser.parse_args()
    if args.repetitions < 1 or args.items < 1 or args.timeout <= 0:
        parser.error('repetitions, items and timeout must be positive')
    os.umask(0o077)
    args.output.mkdir(mode=0o700, parents=True, exist_ok=False)
    print('Private raw evidence: ' + str(args.output.resolve()), flush=True)
    try:
        report = capture(args.compiler, args.output, args.repetitions, args.items, args.timeout)
        print(json.dumps(dict(verdict=report['verdict'], summary=report['summary']), indent=2))
    finally:
        receipt = args.output.resolve() / 'receipt.json'
        if receipt.exists():
            try:
                uploaded = subprocess.run(['justrust', 'records', 'put', 'bench', args.output.resolve().name,
                                           str(receipt)], capture_output=True, text=True)
                print(uploaded.stdout, end='')
                if uploaded.returncode:
                    print('Record upload failed, private receipt retained: ' + uploaded.stderr)
            except OSError as error:
                print('Record upload failed, private receipt retained: ' + str(error))


if __name__ == '__main__':
    main()
