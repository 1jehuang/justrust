#!/usr/bin/env python3
"""Opt-in managed decoder correctness acceptance. No typed reuse or speed claim.

Requires an explicitly frozen compiler and new evidence directory. This module
never builds or installs a compiler. Gates remain off unless mode is explicit.
"""
import argparse
import hashlib
import json
import os
from pathlib import Path
import selectors
import subprocess
import sys
import uuid

import stable_resident as shared
import stable_resident_ast as ast
import stable_resident_owned as owned

COUNTERS = ('hits', 'misses', 'rejected', 'publications', 'retained_entries',
            'retirement_requests')
SOURCE = '''mod helper;
fn main() {
    let nonce = std::env::args().nth(1).unwrap();
    let matched = std::matches!(Some(7u32), Some(7));
    println!("{}:{}:{}:{}", nonce, matched, std::env::consts::OS, helper::value());
}
'''
HELPER = 'pub fn value() -> u32 { 11 }\n'


def environment(mode):
    if mode not in ('0', '1'):
        raise ValueError('Decoder mode must be explicit 0 or 1')
    env = owned.environment()
    for gate in ('AST', 'LEXER', 'TYPED'):
        env['JUSTRUST_RUSTC_RESIDENT_' + gate] = '0'
    env['JUSTRUST_RUSTC_RESIDENT_DECODERS'] = mode
    return env


class DecoderResident(owned.OwnedResident):
    def __init__(self, compiler, root, mode, timeout=180, evidence=None):
        self.command = [str(compiler), '--jr-resident-owned']
        self.root, self.timeout = root, timeout
        evidence = root if evidence is None else evidence
        self.stdout = (evidence / 'resident.stdout').open('w+b', buffering=0)
        self.stderr = (evidence / 'resident.stderr').open('w+b', buffering=0)
        self.transcript = (evidence / 'resident.jsonl').open('wb')
        read_fd, write_fd = os.pipe()
        env = environment(mode)
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
        self.buffer, self.sequence, self.offsets = b'', 0, [0, 0]
        self.args = self.owned = None

    def retirement_eof(self):
        """Observe owner shutdown without sending stdin EOF to cause it."""
        if self.buffer:
            raise AssertionError('Unexpected trailing retirement response')
        if not self.selector.select(self.timeout):
            raise AssertionError('Retired owner did not close response FD')
        if os.read(self.response_fd, 65536) != b'':
            raise AssertionError('Retired owner emitted extra response bytes')
        self.process.wait(timeout=self.timeout)
        if self.process.returncode != 0:
            raise AssertionError('Retired owner shutdown failed')
        return dict(eof=True, returncode=self.process.returncode)


def validate_stats(response, previous, mode, kind):
    stats = response.get('decoder')
    if (not isinstance(stats, dict) or type(stats.get('retired')) is not bool or
            any(type(stats.get(k)) is not int or stats[k] < 0 for k in COUNTERS)):
        raise AssertionError('Missing or invalid decoder telemetry')
    delta = {k: stats[k] - (previous[k] if previous else 0) for k in COUNTERS if k != 'retained_entries'}
    if any(v < 0 for v in delta.values()):
        raise AssertionError('Cumulative decoder counter decreased')
    for field, keys in (('ast', ast.COUNTERS), ('lexer', ast.COUNTERS), ('live', ast.LIVE_COUNTERS)):
        other = response.get(field)
        if not isinstance(other, dict) or any(type(other.get(k)) is not int or other[k] != 0 for k in keys):
            raise AssertionError('Disabled ' + field + ' telemetry must be zero')
        if any(isinstance(v, (int, float)) and v != 0 for v in other.values()):
            raise AssertionError('Unexpected nonzero ' + field + ' telemetry')
    reason = response.get('owned', {}).get('retirement_reason')
    if response.get('executed') is not True:
        raise AssertionError('Native owned request was not executed')
    if mode == '0':
        if any(stats.values()):
            raise AssertionError('Disabled decoder telemetry must be zero')
    elif mode == '1':
        if kind == 'hit' and delta['hits'] <= 0:
            raise AssertionError('Unchanged metadata requires positive decoder hit delta')
        if kind == 'cold' and (delta['misses'] <= 0 or delta['publications'] <= 0):
            raise AssertionError('Cold owner requires decoder misses/publications')
        if kind != 'failure' and (stats['retained_entries'] <= 0 or stats['retired'] or delta['retirement_requests']):
            raise AssertionError('Successful owner must retain decoders without retirement')
        if kind == 'failure' and (stats['retired'] is not True or delta['retirement_requests'] <= 0):
            raise AssertionError('Failure must retire retained decoders and request owner retirement')
    else:
        raise ValueError('Unknown decoder mode')
    if kind == 'failure' and mode == '1':
        if reason is None:
            raise AssertionError('Failure must retire owner')
    elif reason is not None:
        raise AssertionError('Successful request unexpectedly retired owner')
    return dict(stats)


def arguments(root):
    return [str(root / 'fixture.rs'), '--crate-name=decoder_fixture', '--edition=2024',
            '-Copt-level=0', '-Cdebuginfo=0', '-o', str(root / 'fixture')]


def runtime(binary, root, env, timeout):
    nonce = uuid.uuid4().hex
    return dict(nonce=nonce, execution=shared.run([binary, nonce], timeout, cwd=root, env=env))


def validate_pair(records, pid, expected):
    fresh, resident = (records[k] for k in ('fresh', 'resident'))
    if resident['compile'].get('pid') != pid:
        raise AssertionError('Resident PID changed')
    if any(r['compile'].get('timed_out') for r in records.values()):
        raise AssertionError('Compiler timed out')
    if shared.observable(fresh['compile']) != shared.observable(resident['compile']):
        raise AssertionError('Exact status/stdout/diagnostics differ')
    if expected is None:
        if fresh['compile']['status'] != 1 or any('runtime' in r for r in records.values()):
            raise AssertionError('Expected compile error without stale artifact execution')
        return
    outputs = []
    nonces = []
    import base64
    for record in (fresh, resident):
        if record['compile']['status'] != 0 or not record.get('artifact', {}).get('sha256'):
            raise AssertionError('Expected newly produced artifact')
        run = record['runtime']
        execution = run['execution']
        output = base64.b64decode(execution['stdout_b64']).decode()
        prefix = run['nonce'] + ':true:'
        if (execution['status'] != 0 or execution['stderr_b64'] or execution.get('timed_out') or
                not output.startswith(prefix) or not output.endswith(':' + str(expected) + '\n')):
            raise AssertionError('Artifact runtime mismatch')
        os_name = output[len(prefix):].rsplit(':', 1)[0]
        if not os_name or '\n' in os_name:
            raise AssertionError('Missing ordinary std OS constant')
        outputs.append(output[len(run['nonce']):])
        nonces.append(run['nonce'])
    if outputs[0] != outputs[1] or nonces[0] == nonces[1]:
        raise AssertionError('Fresh runtime oracle mismatch or reused nonce')


def capture_mode(compiler, root, mode, timeout):
    root.mkdir(mode=0o700)
    env, argv = environment(mode), arguments(root)
    result = dict(mode=mode, verdict='incomplete', observations=[], argv=argv, cwd=str(root),
        environment_sha256=hashlib.sha256(json.dumps(env, sort_keys=True).encode()).hexdigest())
    worker = None
    try:
        previous = None
        old_pid = None
        workload = [('initial', SOURCE, HELPER, 11, 'cold'),
                    ('unchanged', SOURCE, HELPER, 11, 'hit'),
                    ('unrelated-local-edit', SOURCE, HELPER.replace('11', '23'), 23, 'hit'),
                    ('failure', SOURCE, 'pub fn value() -> u32 { false }\n', None, 'failure'),
                    ('new-process-recovery', SOURCE, HELPER, 11, 'cold')]
        for index, (name, source, helper, expected, kind) in enumerate(workload):
            if worker is None:
                session = root / ('owner-' + str(index))
                session.mkdir()
                worker = DecoderResident(compiler, root, mode, timeout, evidence=session)
                if old_pid == worker.process.pid:
                    raise AssertionError('Recovery requires new owner PID')
                previous = None
            row = dict(name=name, source=source, helper=helper, kind=kind, records={},
                       pid=worker.process.pid, command=worker.command)
            result['observations'].append(row)
            for filename, text in (('fixture.rs', source), ('helper.rs', helper)):
                (root / filename).write_text(text)
                (root / (str(index) + '-' + filename)).write_text(text)
            for transport in ('fresh', 'resident'):
                binary = root / 'fixture'
                binary.unlink(missing_ok=True)
                record = row['records'][transport] = {}
                record['compile'] = (shared.run([compiler, *argv], timeout, cwd=root, env=env)
                    if transport == 'fresh' else worker.compile(argv))
                if record['compile']['status'] == 0:
                    saved = root / f'{index}-{transport}.artifact'
                    saved.write_bytes(binary.read_bytes())
                    record['artifact'] = dict(path=str(saved), sha256=shared.digest(saved))
                    record['runtime'] = runtime(binary, root, env, timeout)
            validate_pair(row['records'], worker.process.pid, expected)
            previous = validate_stats(row['records']['resident']['compile'], previous, mode, kind)
            if kind == 'failure':
                if mode == '1':
                    row['retirement'] = worker.retirement_eof()
                else:
                    row['control_shutdown'] = 'client stdin EOF, not decoder retirement'
                old_pid = worker.process.pid
                worker.close()
                worker = None
        result['verdict'] = 'passed'
    except BaseException as error:
        result.update(verdict='failed', error=f'{type(error).__name__}: {error}')
    finally:
        if worker is not None:
            try:
                worker.close()
            except Exception as error:
                result.update(verdict='failed', shutdown_error=str(error))
        (root / 'receipt.json').write_text(json.dumps(result, indent=2) + '\n')
    return result


def capture(compiler, out, timeout=180):
    compiler, out = Path(compiler).resolve(), Path(out).resolve()
    if not 0 < timeout < float('inf'):
        raise ValueError('timeout must be finite and positive')
    out.mkdir(mode=0o700, parents=True, exist_ok=False)
    report = dict(schema=1, scope='managed-decoder-correctness-only', verdict='incomplete', modes=[],
        claim='Explicit opt-in decoder telemetry and correctness only. Default off. No external typed reuse or performance claim.')
    try:
        report['identity_before'] = shared.identity(compiler)
        for mode, label in (('0', 'disabled'), ('1', 'decoder')):
            report['modes'].append(capture_mode(compiler, out / label, mode, timeout))
        report['verdict'] = 'passed' if all(m['verdict'] == 'passed' for m in report['modes']) else 'failed'
    except BaseException as error:
        report.update(verdict='failed', error=f'{type(error).__name__}: {error}')
    finally:
        try:
            report['identity_after'] = shared.identity(compiler)
            if report.get('identity_before') != report['identity_after']:
                raise AssertionError('Frozen compiler identity changed')
        except Exception as error:
            report.update(verdict='failed', identity_error=str(error))
        (out / 'receipt.json').write_text(json.dumps(report, indent=2) + '\n')
    return report


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--compiler', type=Path, required=True)
    parser.add_argument('--output', type=Path, required=True)
    parser.add_argument('--timeout', type=float, default=180)
    args = parser.parse_args()
    if not 0 < args.timeout < float('inf'):
        parser.error('timeout must be finite and positive')
    os.umask(0o077)
    report = capture(args.compiler, args.output, args.timeout)
    print(json.dumps(dict(verdict=report['verdict'], receipt=str(args.output.resolve() / 'receipt.json'))))
    return 0 if report['verdict'] == 'passed' else 1


if __name__ == '__main__':
    sys.exit(main())
