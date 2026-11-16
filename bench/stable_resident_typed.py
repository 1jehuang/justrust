#!/usr/bin/env python3
"""Explicit frozen-compiler native typed retention acceptance, never a speed test.

Local no_core inputs deliberately avoid the external-crate identity rejection.
Retains original protocol frames, byte-exact diagnostic streams and LLVM IR.
The artifact oracle checks the exported constant-return function, not arbitrary
program equivalence. No compiler is built, installed or selected implicitly.
"""
import argparse
import hashlib
import json
import os
from pathlib import Path
import re
import selectors
import subprocess
import sys

import stable_resident as shared
import stable_resident_owned as owned

COUNTERS = ('typeck_hits', 'typeck_misses', 'mir_hits', 'mir_misses',
            'retained_entries', 'retained_estimated_bytes')
HITS = ('typeck_hits', 'mir_hits')


def environment(mode):
    if mode not in ('control', '1', 'leaf'):
        raise ValueError('Expected explicit typed control, 1 or leaf')
    env = owned.environment()
    env['JUSTRUST_RUSTC_RESIDENT_TYPED'] = mode
    return env


class TypedResident(owned.OwnedResident):
    """Owned transport with a dedicated explicit environment, not global mutation."""
    def __init__(self, compiler, root, mode, timeout=180):
        self.command = [str(compiler), '--jr-resident-owned']
        self.timeout = timeout
        self.root = root
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
        self.buffer = b''
        self.sequence = 0
        self.offsets = [0, 0]
        self.args = None
        self.owned = None


def fixture(value=7, width=8, caller_width=None):
    if caller_width is None:
        caller_width = width
    return '''#![feature(no_core, lang_items)]
#![no_core]
#![allow(internal_features, dead_code)]
#[lang = "pointee_sized"] pub trait PointeeSized {}
#[lang = "meta_sized"] pub trait MetaSized: PointeeSized {}
#[lang = "sized"] pub trait Sized: MetaSized {}
#[unsafe(no_mangle)]
pub fn typed_value() -> u%d { %d }
pub fn checked() -> u%d { typed_value() }
''' % (width, value, caller_width)


def cases():
    # Recovery differs from the last successful source. A failed revision must
    # neither publish its source nor authorize hits against a different source.
    return [
        ('initial', fixture(), (8, 7), False),
        ('unchanged', fixture(), (8, 7), True),
        ('unchanged-again', fixture(), (8, 7), True),
        ('body-edit', fixture(9), (8, 9), False),
        ('body-unchanged', fixture(9), (8, 9), True),
        ('signature-edit', fixture(11, 16), (16, 11), False),
        ('signature-unchanged', fixture(11, 16), (16, 11), True),
        ('signature-error', fixture(13, 16, 8), None, False),
        ('error-repeat', fixture(13, 16, 8), None, False),
        ('recovery', fixture(15), (8, 15), False),
        ('recovery-unchanged', fixture(15), (8, 15), True),
    ]


def arguments(root):
    return [str(root / 'fixture.rs'), '--crate-name=stable_resident_typed_fixture',
            '--crate-type=lib', '--edition=2024', '-Zcodegen-backend=llvm',
            '-Copt-level=0', '-Cdebuginfo=0', '-Cpanic=abort',
            '-Cincremental=' + str(root / 'incremental'),
            '--emit=llvm-ir=' + str(root / 'fixture.ll') + ',mir=' + str(root / 'fixture.mir')]


def validate_live(response, previous, mode, reusable):
    if mode not in ('control', '1'):
        raise ValueError('Expected explicit typed control or 1')
    if reusable and response['status'] != 0:
        raise AssertionError('Unchanged repeated compilation failed')
    if response.get('executed') is not True:
        raise AssertionError('Native request was not executed')
    if response['owned'].get('retirement_reason') is not None:
        raise AssertionError('Native worker retired before acceptance completed')
    stats = response.get('live')
    if not isinstance(stats, dict):
        raise AssertionError('Missing actual live typed telemetry')
    for key in COUNTERS:
        value = stats.get(key)
        if type(value) is not int or value < 0:
            raise AssertionError('Invalid live counter: ' + key)
        if previous is not None and key not in COUNTERS[-2:] and value < previous[key]:
            raise AssertionError('Live counter decreased: ' + key)
    if mode == 'control':
        if any(stats[key] for key in COUNTERS):
            raise AssertionError('Disabled control has nonzero live counters')
    else:
        delta = sum(stats[key] - (previous[key] if previous else 0) for key in HITS)
        if reusable and response['status'] == 0:
            if delta <= 0:
                raise AssertionError('Unchanged success did not increase actual typed hits')
            if stats['retained_entries'] == 0:
                raise AssertionError('Typed hits without retained entries')
        elif delta != 0:
            raise AssertionError('Changed/error source authorized typed reuse')
    return dict(stats)


def artifact_oracle(data, expected):
    # Deliberately narrow semantic oracle: the externally visible function must
    # contain exactly the expected constant return and no call instructions.
    text = data.decode('utf-8')
    function = re.search(r'^define\b[^\n]*@typed_value\([^\n]*\).*?^}',
                         text, re.MULTILINE | re.DOTALL)
    if function is None:
        raise AssertionError('Missing exported typed_value LLVM definition')
    body = function.group()
    returns = re.findall(r'\bret i(\d+) (\d+)\b', body)
    if returns != [(str(expected[0]), str(expected[1]))] or re.search(r'\bcall\b', body):
        raise AssertionError('Incorrect constant-return artifact semantics')
    return dict(kind='llvm-ir-exported-constant-return', width=expected[0], value=expected[1])


def validate_pair(fresh, resident, expected, pid):
    if resident.get('pid') != pid:
        raise AssertionError('Native PID changed')
    if fresh.get('timed_out') or resident.get('timed_out'):
        raise AssertionError('Compiler timed out')
    if shared.observable(fresh) != shared.observable(resident):
        raise AssertionError('Exact fresh/native compiler status/stdout/diagnostics differ')
    if fresh['status'] != (1 if expected is None else 0):
        raise AssertionError('Unexpected compiler status')


def capture_mode(compiler, root, mode, timeout):
    root.mkdir(mode=0o700)
    result = dict(mode=mode, observations=[], environment_sha256=hashlib.sha256(
        json.dumps(environment(mode), sort_keys=True).encode()).hexdigest())
    worker = None
    try:
        worker = TypedResident(compiler, root, mode, timeout)
        result.update(pid=worker.process.pid, command=worker.command,
                      cwd=str(root), argv=arguments(root))
        previous = None
        for index, (name, source, expected, reusable) in enumerate(cases()):
            (root / 'fixture.rs').write_text(source)
            row = dict(name=name, source=source, expected=list(expected) if expected else None,
                       reusable=reusable)
            result['observations'].append(row)
            for kind in ('fresh', 'resident'):
                # Same frozen argv/path, separate persistent incremental banks.
                bank = root / (kind + '-incremental')
                active = root / 'incremental'
                if bank.exists():
                    bank.rename(active)
                artifact = root / 'fixture.ll'
                mir_artifact = root / 'fixture.mir'
                artifact.unlink(missing_ok=True)
                mir_artifact.unlink(missing_ok=True)
                try:
                    record = (shared.run([compiler, *result['argv']], timeout,
                        cwd=root, env=environment(mode)) if kind == 'fresh'
                        else worker.compile(result['argv']))
                    row[kind] = record
                    if record['status'] == 0:
                        data = artifact.read_bytes()
                        saved = root / ('%02d-%s.ll' % (index, kind))
                        saved.write_bytes(data)
                        record['artifact'] = dict(path=str(saved),
                            sha256=hashlib.sha256(data).hexdigest())
                        record['artifact']['oracle'] = artifact_oracle(data, expected)
                        mir_data = mir_artifact.read_bytes()
                        saved_mir = root / ('%02d-%s.mir' % (index, kind))
                        saved_mir.write_bytes(mir_data)
                        if not mir_data:
                            raise AssertionError('Explicit MIR demand produced an empty artifact')
                        record['mir_artifact'] = dict(path=str(saved_mir),
                            sha256=hashlib.sha256(mir_data).hexdigest())
                finally:
                    if active.exists():
                        active.rename(bank)
            validate_pair(row['fresh'], row['resident'], expected, worker.process.pid)
            previous = validate_live(row['resident'], previous, mode, reusable)
        result['live'] = previous
        closing, worker = worker, None
        closing.close()
        result['verdict'] = 'passed'
        return result
    except BaseException as error:
        result.update(verdict='failed', error=f'{type(error).__name__}: {error}')
        raise
    finally:
        failing = sys.exc_info()[0] is not None
        try:
            if worker is not None:
                worker.close()
        except Exception as error:
            result.update(verdict='failed', shutdown_error=str(error))
            if not failing:
                raise
        finally:
            (root / 'receipt.json').write_text(json.dumps(result, indent=2) + '\n')


def capture(compiler, root, timeout=180):
    compiler, root = Path(compiler).resolve(), Path(root).resolve()
    report = dict(schema=1, scope='native-owned-live-typed-same-source-correctness',
        verdict='incomplete', modes=[], claim='Actual retained Typeck/MIR hit acceptance '
        'for controlled local same-source compilation only. No speed, hosted, general '
        'cross-edit reuse or arbitrary artifact equivalence claim.')
    try:
        report['mode_receipts'] = [str(root / name / 'receipt.json')
                                   for name in ('control', 'typed')]
        report['identity_before'] = shared.identity(compiler)
        for name, mode in (('control', 'control'), ('typed', '1')):
            report['modes'].append(capture_mode(compiler, root / name, mode, timeout))
        report['identity_after'] = shared.identity(compiler)
        if report['identity_before'] != report['identity_after']:
            raise AssertionError('Frozen compiler identity changed')
        report['verdict'] = 'passed'
        return report
    except BaseException as error:
        report.update(verdict='failed', error=f'{type(error).__name__}: {error}')
        raise
    finally:
        (root / 'receipt.json').write_text(json.dumps(report, indent=2) + '\n')


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--compiler', required=True, type=Path)
    parser.add_argument('--output', required=True, type=Path)
    parser.add_argument('--timeout', type=float, default=180)
    args = parser.parse_args()
    if not 0 < args.timeout < float('inf'):
        parser.error('timeout must be finite and positive')
    os.umask(0o077)
    args.output.mkdir(mode=0o700, parents=True, exist_ok=False)
    print('Private original evidence: ' + str(args.output.resolve()), flush=True)
    try:
        report = capture(args.compiler, args.output, args.timeout)
        print(json.dumps(dict(verdict=report['verdict'], claim=report['claim']), indent=2))
    finally:
        receipt = args.output.resolve() / 'receipt.json'
        if receipt.exists():
            try:
                upload = subprocess.run(['justrust', 'records', 'put', 'bench',
                    args.output.resolve().name, str(receipt)], capture_output=True, text=True)
                print(upload.stdout, end='')
                if upload.returncode:
                    print('Record upload failed, raw evidence retained: ' + upload.stderr)
            except OSError as error:
                print('Record upload failed, raw evidence retained: ' + str(error))


if __name__ == '__main__':
    main()
