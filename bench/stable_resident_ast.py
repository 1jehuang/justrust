#!/usr/bin/env python3
"""Opt-in root parse-AST correctness acceptance, never a speed benchmark.

API capture(compiler, out, timeout=180). CLI requires --compiler and --output
NEW_DIR. Invoke only after the coordinator supplies a frozen compiler and ACK.
No compiler builds, installation, forced MIR/LLVM demand or incremental state.
Raw receipts, full streams, sources and compiler identities survive failure.
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
import stable_resident_owned as owned

COUNTERS = ('hits', 'misses', 'rejected', 'retained_entries', 'retained_estimated_bytes')
LIVE_COUNTERS = ('typeck_hits', 'typeck_misses', 'mir_hits', 'mir_misses',
                 'retained_entries', 'retained_estimated_bytes')
ROOT = 'pub mod helper; pub fn value(x:u32)->u32 { (x ^ 7) & 255 }\n'
HELPER = 'pub fn value(x:u32)->u32 { (x ^ 11) & 255 }\n'
EDGES = (0, 1, 7, 11, 255, 256, 65535, 2147483648, 4294967295)


def environment(mode):
    if mode not in ('0', '1'):
        raise ValueError('AST mode must be explicit 0 or 1')
    env = owned.environment()
    env['JUSTRUST_RUSTC_RESIDENT_AST'] = mode
    env['JUSTRUST_RUSTC_RESIDENT_LEXER'] = '0'
    env['JUSTRUST_RUSTC_RESIDENT_TYPED'] = '0'
    return env


class AstResident(owned.OwnedResident):
    """Reuse owned framing and full-stream capture with an isolated launch env."""
    def __init__(self, compiler, root, mode, timeout=180):
        self.command = [str(compiler), '--jr-resident-owned']
        self.root, self.timeout = root, timeout
        self.stdout = (root / 'resident.stdout').open('w+b', buffering=0)
        self.stderr = (root / 'resident.stderr').open('w+b', buffering=0)
        self.transcript = (root / 'resident.jsonl').open('wb')
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


def cases():
    changed = ROOT.replace('^ 7', '^ 19')
    helper = HELPER.replace('^ 11', '^ 23')
    bad = changed.replace('& 255 }', '& 255 ) }')
    typed_bad = changed.replace('->u32', '->bool')
    return [
        ('initial', ROOT, HELPER, 7, 11, 'cold'),
        ('unchanged', ROOT, HELPER, 7, 11, 'hit'),
        ('unrelated-helper-edit', ROOT, helper, 7, 23, 'hit'),
        ('root-body-edit', changed, helper, 19, 23, 'miss'),
        ('repeat-new-root', changed, helper, 19, 23, 'hit'),
        ('root-delimiter-error', bad, helper, None, None, 'reject'),
        ('recovery', changed, helper, 19, 23, 'recover'),
        ('root-type-error', typed_bad, helper, None, None, 'miss'),
        ('repeat-type-error', typed_bad, helper, None, None, 'hit'),
        ('root-macro-fallback', 'macro_rules! identity { ($x:expr) => { $x }; }\n' +
         changed.replace('(x ^ 19) & 255', 'identity!((x ^ 19) & 255)'), helper, 19, 23, 'reject'),
        ('root-item-attribute-fallback', changed.replace('pub fn', '#[inline] pub fn'), helper, 19, 23, 'reject'),
        ('root-attribute-fallback', '#![allow(dead_code)]\n' + changed, helper, 19, 23, 'reject'),
        ('final-recovery', changed, helper, 19, 23, 'recover'),
    ]


def arguments(root):
    return [str(root / 'fixture.rs'), '--crate-name=ast_fixture', '--crate-type=rlib',
            '--edition=2024', '-Copt-level=0', '-Cdebuginfo=0', '--emit=link',
            '-o', str(root / 'libast_fixture.rlib')]


def consumer_source():
    inputs = ','.join(str(x) + 'u32' for x in EDGES)
    return ('extern crate ast_fixture;\nfn main() {\n'
            'let nonce = std::env::args().nth(1).unwrap();\n'
            'for x in [' + inputs + '] { println!("{}:{}:{}:{}", nonce, x, '
            'ast_fixture::value(x), ast_fixture::helper::value(x)); }\n}\n')


def execute(compiler, root, mode, timeout, a, b):
    binary = root / 'consumer'
    binary.unlink(missing_ok=True)
    nonce = uuid.uuid4().hex
    # Always a fresh consumer compilation against this revision's ordinary rlib.
    link = shared.run([compiler, str(root / 'consumer.rs'), '--edition=2024',
        '--extern', 'ast_fixture=' + str(root / 'libast_fixture.rlib'),
        '-o', str(binary)], timeout, cwd=root, env=environment(mode))
    record = dict(nonce=nonce, compile=link)
    if link['status'] == 0:
        record['execution'] = shared.run([binary, nonce], timeout, cwd=root, env=environment(mode))
    return record


def validate_runtime(record, a, b):
    if shared.observable(record['compile']) != dict(status=0, stdout_b64='', stderr_b64=''):
        raise AssertionError('Fresh Rust consumer compile failed')
    expected = ''.join(f"{record['nonce']}:{x}:{(x ^ a) & 255}:{(x ^ b) & 255}\n" for x in EDGES)
    if shared.observable(record['execution']) != dict(status=0,
            stdout_b64=shared.encoded(expected.encode()), stderr_b64=''):
        raise AssertionError('Fresh Rust consumer runtime mismatch')
    if record['execution'].get('timed_out'):
        raise AssertionError('Fresh Rust consumer timed out')


def validate_stats(response, previous, mode, kind):
    if response.get('executed') is not True or response.get('owned', {}).get('retirement_reason') is not None:
        raise AssertionError('Owned request not executed or worker retired')
    for field, keys in (('lexer', COUNTERS), ('live', LIVE_COUNTERS)):
        stats = response.get(field)
        if not isinstance(stats, dict) or any(type(stats.get(k)) is not int or stats[k] != 0 for k in keys):
            raise AssertionError('Disabled ' + field + ' telemetry must be present and all zero')
        if field == 'live' and stats.get('mir_leaf_last_rejection') is not None:
            raise AssertionError('Disabled leaf query rejection must be null')
        for key, value in stats.items():
            if isinstance(value, (int, float)) and value != 0:
                raise AssertionError('Other retention telemetry nonzero: ' + key)
    stats = response.get('ast')
    if not isinstance(stats, dict) or any(type(stats.get(k)) is not int or stats[k] < 0 for k in COUNTERS):
        raise AssertionError('Missing or invalid AST telemetry')
    delta = {k: stats[k] - (previous[k] if previous else 0) for k in COUNTERS[:3]}
    if any(v < 0 for v in delta.values()):
        raise AssertionError('Cumulative AST counter decreased')
    if mode == '0':
        if any(stats[k] for k in COUNTERS):
            raise AssertionError('Disabled AST telemetry must be all zero')
    elif mode == '1':
        expected = 1 if kind == 'hit' else 0
        if kind == 'recover':
            # Restoring bytes after a changed root can allocate a new SourceFile.
            # Exact-Arc cache identity therefore permits a fresh miss or one hit,
            # never rejection, double accounting, or skipping the root parse.
            if delta['hits'] not in (0, 1) or delta['misses'] != 1 - delta['hits'] or delta['rejected']:
                raise AssertionError('Recovery requires exactly one root lookup and no rejection')
        elif delta['hits'] != expected:
            raise AssertionError('Exact root AST hit delta required: ' + str(expected))
        if kind in ('cold', 'miss') and delta['misses'] != 1:
            raise AssertionError('New root parse requires exactly one AST miss')
        if kind == 'hit' and (delta['misses'] or delta['rejected']):
            raise AssertionError('Root-only AST hit cannot count helper misses or rejections')
        if kind == 'reject':
            if delta['rejected'] < 1:
                raise AssertionError('Unsupported/failed parse must be rejected')
            if previous and any(stats[k] > previous[k] for k in COUNTERS[3:]):
                raise AssertionError('Rejected AST admitted')
        elif stats['retained_entries'] < 1 or stats['retained_estimated_bytes'] < 1:
            raise AssertionError('Supported AST must be retained, including parse-success type errors')
    else:
        raise ValueError('Unknown AST mode')
    return dict(stats)


def capture_mode(compiler, root, mode, timeout):
    root.mkdir(mode=0o700)
    result = dict(mode=mode, verdict='incomplete', observations=[], failures=[])
    worker = None
    try:
        argv = arguments(root)
        env = environment(mode)
        result.update(argv=argv, cwd=str(root),
                      environment_sha256=hashlib.sha256(json.dumps(env, sort_keys=True).encode()).hexdigest())
        (root / 'consumer.rs').write_text(consumer_source())
        worker = AstResident(compiler, root, mode, timeout)
        result.update(pid=worker.process.pid, command=worker.command)
        previous = None
        nonces = set()
        for index, (name, source, helper, a, b, kind) in enumerate(cases()):
            row = dict(name=name, kind=kind, source=source, helper=helper,
                       source_sha256=hashlib.sha256(source.encode()).hexdigest(),
                       helper_sha256=hashlib.sha256(helper.encode()).hexdigest(), records={})
            result['observations'].append(row)
            for filename, data in (('fixture.rs', source), ('helper.rs', helper)):
                (root / filename).write_text(data)
                (root / (f'{index:02d}-' + filename)).write_text(data)
            for transport in ('fresh', 'resident'):
                artifact = root / 'libast_fixture.rlib'
                artifact.unlink(missing_ok=True)
                record = row['records'][transport] = {}
                record['compile'] = (shared.run([compiler, *argv], timeout, cwd=root, env=env)
                                     if transport == 'fresh' else worker.compile(argv))
                if record['compile']['status'] == 0:
                    saved = root / f'{index:02d}-{transport}.rlib'
                    saved.write_bytes(artifact.read_bytes())
                    record['artifact'] = dict(path=str(saved), sha256=shared.digest(saved))
                    record['runtime'] = execute(compiler, root, mode, timeout, a, b)
            try:
                fresh, resident = (row['records'][x] for x in ('fresh', 'resident'))
                if resident['compile'].get('pid') != worker.process.pid:
                    raise AssertionError('Resident PID changed')
                if any(r['compile'].get('timed_out') for r in (fresh, resident)):
                    raise AssertionError('Compiler timed out')
                if shared.observable(fresh['compile']) != shared.observable(resident['compile']):
                    raise AssertionError('Exact status/stdout/diagnostics differ')
                for record in (fresh, resident):
                    if a is None:
                        if record['compile']['status'] != 1 or 'runtime' in record:
                            raise AssertionError('Expected compile error without stale runtime')
                    else:
                        if record['compile']['status'] != 0:
                            raise AssertionError('Expected successful ordinary rlib')
                        validate_runtime(record['runtime'], a, b)
                        nonce = record['runtime']['nonce']
                        if nonce in nonces:
                            raise AssertionError('Each fresh runtime must use a new nonce')
                        nonces.add(nonce)
                validate_stats(resident['compile'], previous, mode, kind)
            except AssertionError as error:
                row['acceptance_error'] = str(error)
                result['failures'].append(dict(case=name, error=str(error)))
            current = row['records']['resident']['compile'].get('ast')
            if isinstance(current, dict) and all(type(current.get(k)) is int for k in COUNTERS):
                previous = dict(current)
        result['verdict'] = 'failed' if result['failures'] else 'passed'
    except BaseException as error:
        result.update(verdict='failed', error=f'{type(error).__name__}: {error}')
        raise
    finally:
        try:
            if worker is not None:
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
    report = dict(schema=1, scope='root-parse-ast-only', verdict='incomplete', modes=[],
                  claim='Correctness and explicit AST telemetry only. No speedup claim.')
    try:
        report['identity_before'] = shared.identity(compiler)
        for mode in ('0', '1'):
            report['modes'].append(capture_mode(compiler, out / ('disabled' if mode == '0' else 'ast'), mode, timeout))
        report['verdict'] = 'passed' if all(m['verdict'] == 'passed' for m in report['modes']) else 'failed'
    except BaseException as error:
        report.update(verdict='failed', error=f'{type(error).__name__}: {error}')
        raise
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
