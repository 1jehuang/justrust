#!/usr/bin/env python3
"""Focused actual Typeck retention acceptance, not a benchmark.

Uses the native typed transport without altering compiler query/identity gates.
Raw diagnostics, protocol frames, sources and LLVM/MIR artifacts are preserved.
A failed hit oracle does not skip the subsequent error/recovery correctness cases.
No hosted commands, builds, installation, timings or performance claims.
"""
import argparse
import hashlib
import json
import os
from pathlib import Path
import sys

import stable_resident_typed as typed

COMPILER = Path('/home/jeremy/.jcode/scratch/stable-resident-memo-20261116-1555-sysroot/bin/rustc')
LAUNCHER = '8cb4b7089e2e70cbd4832fdf53a24ef2e27db676dcc3632c2501134e6f3dd567'
DRIVER = '5b69dcb65e4e25971d5b57be0175d77c7b55a04511b4fefc9e6201ef120e9669'


def fixture(value=7, width=8, caller_width=None):
    # The local closure's inferred output, local inferred binding and explicit
    # caller return require real typeck_root results, not just an empty body.
    # No external crate or user-defined operator obscures the identity gate.
    source = typed.fixture(value, width, caller_width).replace(
        '#[unsafe(no_mangle)]', '#[lang = "copy"] pub trait Copy {}\n'
        '#[unsafe(no_mangle)]')
    return source.replace('{ %d }' % value,
        '{ let _infer = || { let primitive = %du%d; primitive }; '
        'let primitive = %du%d; primitive }' % (value, width, value, width))


def cases():
    return [(name, fixture(value, width, caller), expected, reusable)
            for (name, _, expected, reusable), (value, width, caller) in zip(
                typed.cases(), [(7, 8, 8), (7, 8, 8), (7, 8, 8),
                (9, 8, 8), (9, 8, 8), (11, 16, 16), (11, 16, 16),
                (13, 16, 8), (13, 16, 8), (15, 8, 8), (15, 8, 8)])]


def validate_live(response, previous, mode, reusable):
    # Shared validation still enforces zero changed/error reuse and all-zero
    # disabled telemetry. The stronger oracle never sums MIR and Typeck hits.
    stats = typed.validate_live(response, previous, mode, reusable)
    for key in ('mir_leaf_hits', 'mir_leaf_candidate_misses'):
        if key in stats and (type(stats[key]) is not int or stats[key] != 0):
            raise AssertionError('Unexpected separate leaf telemetry: ' + key)
    if stats.get('mir_leaf_last_rejection') is not None:
        raise AssertionError('Unexpected separate leaf rejection')
    if mode == '1' and reusable:
        delta = stats['typeck_hits'] - (previous['typeck_hits'] if previous else 0)
        if delta <= 0:
            raise AssertionError('Unchanged success did not increase actual Typeck hits '
                                 '(MIR hits are not Typeck acceptance)')
    return stats


def check_identity(identity):
    if identity.get('sha256') != LAUNCHER:
        raise AssertionError('Frozen launcher SHA256 mismatch')
    if [entry.get('sha256') for entry in identity.get('drivers', [])] != [DRIVER]:
        raise AssertionError('Frozen driver SHA256 mismatch')


def capture_mode(compiler, root, mode, timeout):
    root.mkdir(mode=0o700)
    result = dict(mode=mode, observations=[], failures=[], verdict='incomplete')
    worker = None
    try:
        worker = typed.TypedResident(compiler, root, mode, timeout)
        result.update(pid=worker.process.pid, command=worker.command,
                      cwd=str(root), argv=typed.arguments(root),
                      environment_sha256=hashlib.sha256(json.dumps(
                          typed.environment(mode), sort_keys=True).encode()).hexdigest())
        previous = None
        for index, (name, source, expected, reusable) in enumerate(cases()):
            (root / 'fixture.rs').write_text(source)
            (root / ('%02d-source.rs' % index)).write_text(source)
            row = dict(name=name, source=source,
                       expected=list(expected) if expected is not None else None,
                       reusable=reusable)
            result['observations'].append(row)
            for kind in ('fresh', 'resident'):
                bank, active = root / (kind + '-incremental'), root / 'incremental'
                if bank.exists():
                    bank.rename(active)
                for suffix in ('ll', 'mir'):
                    (root / ('fixture.' + suffix)).unlink(missing_ok=True)
                try:
                    record = (typed.shared.run([compiler, *result['argv']], timeout,
                        cwd=root, env=typed.environment(mode)) if kind == 'fresh'
                        else worker.compile(result['argv']))
                    row[kind] = record
                    if record['status'] == 0:
                        for suffix in ('ll', 'mir'):
                            data = (root / ('fixture.' + suffix)).read_bytes()
                            saved = root / ('%02d-%s.%s' % (index, kind, suffix))
                            saved.write_bytes(data)
                            artifact = dict(path=str(saved), sha256=hashlib.sha256(data).hexdigest())
                            record[suffix + '_artifact'] = artifact
                            if suffix == 'll':
                                artifact['oracle'] = typed.artifact_oracle(data, expected)
                            elif not data:
                                raise AssertionError('Explicit MIR demand produced an empty artifact')
                finally:
                    if active.exists():
                        active.rename(bank)
            typed.validate_pair(row['fresh'], row['resident'], expected, worker.process.pid)
            current = row['resident'].get('live')
            if isinstance(current, dict) and all(type(current.get(key)) is int
                                                for key in typed.HITS):
                row['hit_deltas'] = {key: current[key] - (previous[key] if previous else 0)
                                     for key in typed.HITS}
            try:
                stats = validate_live(row['resident'], previous, mode, reusable)
            except AssertionError as error:
                # Keep the original response intact, continue correctness/error
                # recovery, but never turn missing Typeck evidence into a pass.
                row['acceptance_error'] = str(error)
                result['failures'].append(dict(case=name, error=str(error)))
                stats = row['resident'].get('live')
            previous = stats
        result['live'] = previous
        result['verdict'] = 'failed' if result['failures'] else 'passed'
        if mode == '1' and result['failures'] and previous and previous['typeck_hits'] == 0:
            result['typeck_failure_analysis'] = dict(
                observed='No actual Typeck hit on unchanged repeats. MIR hits do not satisfy acceptance.',
                gate='Native resident get is only consulted by '
                     'load_from_disk_or_invoke_provider_green, not ensure_ok green skips.',
                interpretation='The analysis typeck demand uses ensure_ok; green query dependencies '
                               'and retained optimized MIR can satisfy repeat compilation without '
                               'materializing typeck_root. This is a query-demand inference, not '
                               'proof of a failed retained Typeck lookup.',
                source_locations=['toolchain/rustc/compiler/rustc_hir_analysis/src/lib.rs:check_crate',
                    'toolchain/rustc/compiler/rustc_query_impl/src/execution.rs:'
                    'load_from_disk_or_invoke_provider_green',
                    'toolchain/rustc/compiler/rustc_query_impl/src/execution.rs:ensure_can_skip_execution'])
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
    return result


def capture(compiler, root, timeout=180):
    compiler, root = Path(compiler).resolve(), Path(root).resolve()
    # Exclusive creation preserves every original receipt, including failures.
    root.mkdir(mode=0o700, parents=True, exist_ok=False)
    report = dict(schema=1, scope='native-actual-typeck-same-source-correctness',
                  verdict='incomplete', modes=[], claim='Actual Typeck hit delta required. '
                  'MIR is not acceptance. Primitive LLVM output and exact diagnostics only. '
                  'No performance or general semantic-equivalence claim.')
    try:
        report['identity_before'] = typed.shared.identity(compiler)
        check_identity(report['identity_before'])
        for name, mode in (('control', 'control'), ('typeck', '1')):
            report['modes'].append(capture_mode(compiler, root / name, mode, timeout))
        report['verdict'] = ('passed' if all(mode['verdict'] == 'passed'
                            for mode in report['modes']) else 'failed')
    except BaseException as error:
        report.update(verdict='failed', error=f'{type(error).__name__}: {error}')
        raise
    finally:
        try:
            report['identity_after'] = typed.shared.identity(compiler)
            check_identity(report['identity_after'])
            if report.get('identity_before') != report['identity_after']:
                raise AssertionError('Frozen compiler identity changed')
        except Exception as error:
            report.update(verdict='failed', identity_error=str(error))
        (root / 'receipt.json').write_text(json.dumps(report, indent=2) + '\n')
    return report


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--compiler', type=Path, default=COMPILER)
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
