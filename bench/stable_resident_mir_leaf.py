#!/usr/bin/env python3
"""Explicit frozen-compiler, bounded primitive MIR leaf cross-edit acceptance.

No build, implicit compiler selection, speed claim or general MIR/Typeck reuse
claim. Raw protocol, diagnostics, source bytes, LLVM/MIR and failure receipts
are retained. Native source loading reads the root and both real file modules.
"""
import argparse
import hashlib
import json
import os
from pathlib import Path
import re
import subprocess
import sys

import stable_resident as shared
import stable_resident_typed as typed

COUNTERS = (*typed.COUNTERS, 'mir_leaf_hits')
ROOT = '''#![feature(no_core, lang_items)]
#![no_core]
#![allow(internal_features, dead_code)]
#[lang = "pointee_sized"] pub trait PointeeSized {}
#[lang = "meta_sized"] pub trait MetaSized: PointeeSized {}
#[lang = "sized"] pub trait Sized: MetaSized {}
mod module_a;
mod module_b;
'''
CLAIM = ('Bounded primitive exported constant-return MIR leaf cross-edit correctness '
         'only. Positive actual mir_leaf_hits is required. No general MIR/Typeck '
         'reuse, performance, hosted or arbitrary artifact equivalence claim.')


def module(name, value, width=8, literal_width=None):
    literal = str(value) + ('u%d' % literal_width if literal_width else '')
    return '#[unsafe(no_mangle)]\npub fn leaf_%s() -> u%d { %s }\n' % (name, width, literal)


def sources(a=7, b=9, error=False):
    return {'fixture.rs': ROOT.encode(), 'module_a.rs': module('a', a).encode(),
            'module_b.rs': module('b', b, 16 if error else 8,
                                  8 if error else None).encode()}


def cases():
    return [
        ('initial', sources(), (7, 9), 'cold'),
        ('unchanged', sources(), (7, 9), 'same'),
        ('edit-b', sources(b=11), (7, 11), 'cross'),
        ('after-b-unchanged', sources(b=11), (7, 11), 'same'),
        ('edit-a', sources(a=13, b=11), (13, 11), 'cross'),
        ('signature-error', sources(a=13, b=11, error=True), None, 'error'),
        ('error-repeat', sources(a=13, b=11, error=True), None, 'error'),
        ('recovery', sources(a=13, b=15), (13, 15), 'recovery'),
        ('recovery-unchanged', sources(a=13, b=15), (13, 15), 'same'),
    ]


def arguments(root):
    return [str(root / 'fixture.rs'), '--crate-name=stable_resident_mir_leaf_fixture',
            '--crate-type=lib', '--edition=2024', '-Zcodegen-backend=llvm',
            '-Copt-level=0', '-Cdebuginfo=0', '-Cpanic=abort',
            '-Cincremental=' + str(root / 'incremental'),
            '--emit=llvm-ir=' + str(root / 'fixture.ll') + ',mir=' + str(root / 'fixture.mir')]


def artifact_oracle(data, expected):
    text = data.decode('utf-8')
    result = {}
    for name, value in zip(('leaf_a', 'leaf_b'), expected):
        definitions = re.findall(r'^define\b[^\n]*@' + name + r'\([^\n]*\).*?^}',
                                 text, re.MULTILINE | re.DOTALL)
        if len(definitions) != 1:
            raise AssertionError('Missing or duplicate exported ' + name)
        body = definitions[0]
        if (re.findall(r'\bret i(\d+) (\d+)\b', body) != [('8', str(value))]
                or re.search(r'\b(call|invoke|callbr)\b', body)):
            raise AssertionError('Incorrect constant-return artifact: ' + name)
        result[name] = dict(width=8, value=value)
    return result


def validate_live(response, previous, mode, kind):
    if mode not in ('control', 'leaf'):
        raise ValueError('Expected explicit control or leaf')
    if response.get('executed') is not True:
        raise AssertionError('Native request was not executed')
    if response['owned'].get('retirement_reason') is not None:
        raise AssertionError('Native worker retired')
    stats = response.get('live')
    if not isinstance(stats, dict):
        raise AssertionError('Missing live telemetry')
    for key in COUNTERS:
        value = stats.get(key)
        if type(value) is not int or value < 0:
            raise AssertionError('Invalid live counter: ' + key)
        if previous is not None and key not in typed.COUNTERS[-2:] and value < previous[key]:
            raise AssertionError('Live counter decreased: ' + key)
    delta = {key: stats[key] - (previous[key] if previous else 0)
             for key in (*typed.HITS, 'mir_leaf_hits')}
    if mode == 'control':
        if any(stats[key] for key in COUNTERS):
            raise AssertionError('Disabled control has nonzero live counters')
    elif kind == 'cross':
        if response['status'] != 0 or delta['mir_leaf_hits'] <= 0:
            raise AssertionError('Cross-edit requires positive actual live.mir_leaf_hits')
        if delta['typeck_hits'] != 0:
            raise AssertionError('Cross-edit unexpectedly authorized Typeck reuse')
        if stats['retained_entries'] == 0:
            raise AssertionError('Leaf hits without retained entries')
    elif kind == 'same':
        if response['status'] != 0 or sum(delta.values()) <= 0:
            raise AssertionError('Unchanged success requires actual live hits')
        if stats['retained_entries'] == 0:
            raise AssertionError('Hits without retained entries')
    elif kind == 'recovery':
        # Failed staging never publishes a revision. Previously successful,
        # unchanged leaves can still be valid, so recovery may reuse those
        # leaves without authorizing general whole-revision typed reuse.
        if response['status'] != 0:
            raise AssertionError('Recovery compilation failed')
        if any(delta[key] for key in typed.HITS):
            raise AssertionError('Recovery unexpectedly authorized whole-revision typed reuse')
        if delta['mir_leaf_hits'] and stats['retained_entries'] == 0:
            raise AssertionError('Recovery leaf hits without retained entries')
    elif kind in ('cold', 'error'):
        if any(delta.values()):
            raise AssertionError('Cold/error unexpectedly authorized reuse')
    else:
        raise ValueError('Unknown revision kind')
    return dict(stats)


def validate_transition(old, new, kind):
    changed = [name for name in new if old is None or old[name] != new[name]]
    if kind == 'cross':
        if len(changed) != 1 or changed[0] not in ('module_a.rs', 'module_b.rs'):
            raise AssertionError('Cross-edit must change exactly one file module')
        for name in new:
            if old[name].count(b'pub fn ') != new[name].count(b'pub fn '):
                raise AssertionError('Cross-edit changed definition count')
    if kind == 'same' and changed:
        raise AssertionError('Unchanged revision changed source')
    return changed


def capture_mode(compiler, root, evidence, mode, timeout):
    evidence.mkdir(mode=0o700)
    fixed_env = typed.environment(mode)
    argv = arguments(root)
    result = dict(mode=mode, observations=[], argv=argv, cwd=str(root),
                  environment_sha256=hashlib.sha256(json.dumps(fixed_env, sort_keys=True).encode()).hexdigest())
    backups = {name: (root / name).read_bytes() for name in sources()}
    for name, data in backups.items():
        (evidence / (name + '.rawbackup')).write_bytes(data)
    worker = None
    streams = ('resident.stdout', 'resident.stderr', 'resident.jsonl')
    for name in streams:
        (root / name).unlink(missing_ok=True)
    try:
        worker = typed.TypedResident(compiler, root, mode, timeout)
        result.update(pid=worker.process.pid, command=worker.command)
        previous = None
        old_sources = None
        for index, (name, files, expected, kind) in enumerate(cases()):
            changed = validate_transition(old_sources, files, kind)
            for filename, data in files.items():
                # Do not rewrite unchanged modules. Native source objects remain
                # backed by the original unchanged bytes/path across the edit.
                if (root / filename).read_bytes() != data:
                    (root / filename).write_bytes(data)
            row = dict(name=name, kind=kind, changed_files=changed,
                       sources={n: dict(sha256=hashlib.sha256(d).hexdigest(),
                                        bytes_b64=shared.encoded(d)) for n, d in files.items()},
                       expected=expected)
            result['observations'].append(row)
            for engine in ('fresh', 'resident'):
                if typed.environment(mode) != fixed_env:
                    raise AssertionError('Compiler environment changed within worker')
                bank = root / (mode + '-' + engine + '-incremental')
                active = root / 'incremental'
                if bank.exists():
                    bank.rename(active)
                for suffix in ('ll', 'mir'):
                    (root / ('fixture.' + suffix)).unlink(missing_ok=True)
                try:
                    record = (shared.run([compiler, *argv], timeout, cwd=root, env=fixed_env)
                              if engine == 'fresh' else worker.compile(argv))
                    row[engine] = record
                    if record['status'] == 0:
                        if expected is None:
                            raise AssertionError('Signature mismatch unexpectedly compiled')
                        artifacts = {}
                        for suffix in ('ll', 'mir'):
                            data = (root / ('fixture.' + suffix)).read_bytes()
                            artifacts[suffix] = data
                            saved = evidence / ('%02d-%s.%s' % (index, engine, suffix))
                            saved.write_bytes(data)
                            record[suffix] = dict(path=str(saved), sha256=hashlib.sha256(data).hexdigest())
                        record['ll']['oracle'] = artifact_oracle(artifacts['ll'], expected)
                        if not artifacts['mir']:
                            raise AssertionError('Explicit MIR demand produced empty artifact')
                finally:
                    if active.exists():
                        active.rename(bank)
            typed.validate_pair(row['fresh'], row['resident'], expected, worker.process.pid)
            previous = validate_live(row['resident'], previous, mode, kind)
            old_sources = files
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
            for name in streams:
                stream = root / name
                if stream.exists():
                    (evidence / name).write_bytes(stream.read_bytes())
            for name in backups:
                (root / name).write_bytes((evidence / (name + '.rawbackup')).read_bytes())
            result['sources_restored'] = all((root / n).read_bytes() == d for n, d in backups.items())
            (evidence / 'receipt.json').write_text(json.dumps(result, indent=2) + '\n')


def capture(compiler, root, timeout=180):
    compiler, root = Path(compiler).resolve(), Path(root).resolve()
    report = dict(schema=1, scope='native-primitive-mir-leaf-cross-edit',
                  verdict='incomplete', claim=CLAIM, modes=[],
                  mode_receipts=[str(root / mode / 'receipt.json') for mode in ('control', 'leaf')])
    try:
        report['identity_before'] = shared.identity(compiler)
        workspace = root / 'workspace'
        workspace.mkdir(mode=0o700)
        for name, data in sources().items():
            (workspace / name).write_bytes(data)
        for mode in ('control', 'leaf'):
            report['modes'].append(capture_mode(compiler, workspace, root / mode, mode, timeout))
        for control, leaf in zip(*(m['observations'] for m in report['modes'])):
            for engine in ('fresh', 'resident'):
                if shared.observable(control[engine]) != shared.observable(leaf[engine]):
                    raise AssertionError('Exact control/leaf diagnostics differ')
        report['identity_after'] = shared.identity(compiler)
        if report['identity_before'] != report['identity_after']:
            raise AssertionError('Frozen compiler identity changed')
        report['verdict'] = 'passed'
        return report
    except BaseException as error:
        report.update(verdict='failed', error=f'{type(error).__name__}: {error}')
        raise
    finally:
        if 'identity_before' in report and 'identity_after' not in report:
            try:
                report['identity_after'] = shared.identity(compiler)
            except Exception as error:
                report['identity_after_error'] = f'{type(error).__name__}: {error}'
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
