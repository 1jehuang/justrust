#!/usr/bin/env python3
"""Attested native scalar arithmetic diagnostic, not a general or full gate.

Only producer invocation wall time is measured. Fresh C links and executions,
source generation, evidence persistence, startup and revision zero are excluded.
"""
import argparse
import json
import math
from pathlib import Path
import re

import stable_resident as shared
import stable_resident_typed as typed
import stable_resident_mir_leaf as leaf
import stable_resident_mir_arithmetic as arithmetic
import stable_resident_leaf_suite as base

require, sha, canonical = base.require, base.sha, base.canonical
host_identity, aggregate, check_identity = base.host_identity, base.aggregate, base.check_identity
attestation, order = base.attestation, base.order
ARMS = base.ARMS
SCHEMA = 'stable_native_arithmetic/v1'
CLAIM = ('Scalar arithmetic diagnostic with forced object, LLVM IR and MIR output only. '
         'NOT ordinary Cargo build performance, general reuse, realproject or fullgate. '
         'Descriptive producer invocation timings only.')


def fixture(items, revision):
    require(type(items) is int and 1 <= items <= 128, 'items outside 1..128')
    require(type(revision) is int and 0 <= revision <= 63, 'revision outside bounded64 lifetime')
    files, constants = {'fixture.rs': arithmetic.ROOT}, {}
    for module in ('a', 'b'):
        lines = []
        for index in range(items):
            updates = (revision + (module == 'b')) // 2
            value = (7 if module == 'a' else 9) + index + (updates if index == 0 else 0)
            name = f'leaf_{module}_{index:04d}'
            constants[name] = value
            args, expression = ('x: u32, y: u32', f'((x + y) ^ {value}) * 3') if module == 'a' else ('x: u32', f'(x - {value}) & 65535')
            lines.append(f'#[unsafe(no_mangle)]\npub extern "C" fn {name}({args}) -> u32 {{ {expression} }}\n')
        files[f'module_{module}.rs'] = ''.join(lines).encode()
    return files, constants


def expected_output(constants):
    return ''.join(f'{(((((x + y) & arithmetic.MASK) ^ value) * 3) & arithmetic.MASK) if name.startswith("leaf_a_") else (((x - value) & arithmetic.MASK) & 65535)}\n'
                   for name, value in constants.items() for x, y in arithmetic.INPUTS).encode()


def consumer_source(constants):
    declarations, calls = [], []
    for name in constants:
        is_a = name.startswith('leaf_a_')
        declarations.append(f'extern uint32_t {name}(uint32_t' + (', uint32_t' if is_a else '') + ');\n')
        for x, y in arithmetic.INPUTS:
            args = f'UINT32_C({x})' + (f', UINT32_C({y})' if is_a else '')
            calls.append(f'printf("%" PRIu32 "\\n", {name}({args}));\n')
    return ('#include <stdint.h>\n#include <inttypes.h>\n#include <stdio.h>\n'
            '_Static_assert(sizeof(uint32_t) == 4, "u32 ABI");\n' + ''.join(declarations)
            + 'int main(void) {\n' + ''.join(calls) + 'return 0;\n}\n').encode()


def workload(items, edits, repetitions):
    require(type(edits) is int and 2 <= edits <= 63, 'edits outside 2..63')
    require(type(repetitions) is int and repetitions >= 1, 'repetitions must be positive')
    revisions = []
    for revision in range(edits + 1):
        files, constants = fixture(items, revision)
        revisions.append(dict(revision=revision, sources={n: sha(d) for n, d in files.items()},
                              constants=constants, oracle_sha256=sha(expected_output(constants))))
    return dict(fixture='no-core-c-abi-scalar-arithmetic-v1', items=items, edits=edits,
                repetitions=repetitions, revisions=revisions, inputs=list(map(list, arithmetic.INPUTS)),
                order=[order(r) for r in range(repetitions)], lifetime=64,
                timing='producer-invocation-wall-only', warmup='revision0 excluded',
                incremental='isolated per repetition and arm',
                producer_outputs='obj+llvm-ir+mir (diagnostic-forced)')


def execution_identity():
    return dict(argv=arithmetic.arguments(Path('.')), cwd='private-workspace',
                environment_sha256={a: sha(canonical(typed.environment('leaf' if a == 'leaf' else 'control'))) for a in ARMS},
                cc=dict(path=str(arithmetic.CC), resolved=str(arithmetic.CC.resolve()), sha256=sha(arithmetic.CC.read_bytes())))


def consumer_argv(root):
    return [str(arithmetic.CC), '-std=c11', '-O0', '-Wall', '-Wextra', '-Werror',
            str(root / 'consumer.c'), str(root / 'fixture.o'), '-o', str(root / 'consumer')]


def save_file(path, destination):
    data = path.read_bytes()
    destination.write_bytes(data)
    return dict(path=str(destination), sha256=sha(data))


def run_consumer(workspace, evidence, revision, constants, timeout, env, row):
    source, binary = workspace / 'consumer.c', workspace / 'consumer'
    source.write_bytes(consumer_source(constants))
    row['consumer_source'] = save_file(source, evidence / f'{revision:02d}.consumer.c')
    binary.unlink(missing_ok=True)
    row['consumer_compile'] = shared.run(consumer_argv(workspace), timeout, cwd=workspace, env=env)
    if binary.exists():
        row['consumer_binary'] = save_file(binary, evidence / f'{revision:02d}.consumer.bin')
    require(row['consumer_compile']['status'] == 0 and not row['consumer_compile'].get('timed_out'), 'C consumer link failed')
    row['consumer_run'] = shared.run([str(binary)], timeout, cwd=workspace, env=env)
    arithmetic.validate_consumer(row['consumer_run'], expected_output(constants))
    row['oracle_sha256'] = sha(expected_output(constants))


def capture(compiler, output, items=128, edits=6, repetitions=3, timeout=180,
            *, expected_driver_sha256, expected_compiler_sha256):
    expected = attestation(expected_driver_sha256, expected_compiler_sha256)
    compiler = Path(compiler).resolve()
    require(compiler.is_file() and sha(compiler.read_bytes()) == expected['compiler_sha256'], 'unattested compiler before execution')
    drivers = [p for p in sorted((compiler.parent.parent / 'lib').glob('*rustc_driver*')) if p.is_file()]
    require([sha(p.read_bytes()) for p in drivers] == [expected['driver_sha256']], 'unattested driver before execution')
    require(type(timeout) in (int, float) and math.isfinite(timeout) and timeout > 0, 'invalid timeout')
    spec = workload(items, edits, repetitions)
    output = Path(output).resolve()
    output.mkdir(mode=0o700, parents=True, exist_ok=False)
    workspace = output / 'workspace'
    workspace.mkdir(mode=0o700)
    report = dict(schema=SCHEMA, claim=CLAIM, verdict='incomplete', workload=spec,
                  workload_sha256=sha(canonical(spec)), expected_identity=expected,
                  host=host_identity(), execution=execution_identity(), workspace=str(workspace),
                  actual_execution=dict(argv=arithmetic.arguments(Path('.')), cwd=str(workspace)),
                  rows=[], source_blobs={})
    argv = report['execution']['argv']
    try:
        report['identity_before'] = shared.identity(compiler)
        check_identity(report['identity_before'], expected)
        for repetition in range(repetitions):
            for arm in order(repetition):
                evidence = output / f'rep-{repetition:03d}-{arm}'
                evidence.mkdir(mode=0o700)
                bank, active = evidence / 'incremental', workspace / 'incremental'
                worker, previous = None, None
                try:
                    if arm != 'fresh':
                        worker = typed.TypedResident(compiler, workspace, arm, timeout)
                    for revision in range(edits + 1):
                        require(execution_identity() == report['execution'], 'execution environment changed')
                        files, constants = fixture(items, revision)
                        for name, data in files.items():
                            path = workspace / name
                            if not path.exists() or path.read_bytes() != data:
                                path.write_bytes(data)
                            report['source_blobs'][sha(data)] = shared.encoded(data)
                        row = dict(repetition=repetition, arm=arm, revision=revision, warmup=revision == 0,
                                   sources={n: sha(d) for n, d in files.items()}, validated=False, artifacts={})
                        report['rows'].append(row)
                        if bank.exists():
                            bank.rename(active)
                        for suffix in ('o', 'll', 'mir'):
                            (workspace / f'fixture.{suffix}').unlink(missing_ok=True)
                        try:
                            env = typed.environment('leaf' if arm == 'leaf' else 'control')
                            row['record'] = shared.run([compiler, *argv], timeout, cwd=workspace, env=env) if worker is None else worker.compile(argv)
                            record = row['record']
                            # Preserve all produced bytes before any semantic/counter gate.
                            for suffix in ('o', 'll', 'mir'):
                                path = workspace / f'fixture.{suffix}'
                                if path.exists():
                                    row['artifacts'][suffix] = save_file(path, evidence / f'{revision:02d}.{suffix}')
                            require(record['status'] == 0 and not record.get('timed_out'), 'producer failed')
                            require(set(row['artifacts']) == {'o', 'll', 'mir'}, 'missing producer artifacts')
                            run_consumer(workspace, evidence, revision, constants, timeout, env, row)
                            if worker is not None:
                                typed.validate_pair(record, record, constants, worker.process.pid)
                                old = previous
                                previous = leaf.validate_live(record, previous, arm, 'cold' if revision == 0 else 'cross')
                                if revision and arm == 'leaf':
                                    require(previous['mir_leaf_hits'] - old['mir_leaf_hits'] >= items, 'insufficient cross-edit hits')
                            row['validated'] = True
                        finally:
                            if active.exists():
                                active.rename(bank)
                finally:
                    try:
                        if worker is not None:
                            worker.close()
                    finally:
                        for stream in ('resident.stdout', 'resident.stderr', 'resident.jsonl'):
                            path = workspace / stream
                            if path.exists():
                                (evidence / stream).write_bytes(path.read_bytes())
                                path.unlink()
        report['identity_after'] = shared.identity(compiler)
        require(report['identity_after'] == report['identity_before'], 'compiler identity changed')
        require(host_identity() == report['host'], 'host changed')
        require(execution_identity() == report['execution'], 'execution changed')
        report['aggregates'] = aggregate(report['rows'])
        report['verdict'] = 'passed'
        validate(report)
        return report
    except BaseException as error:
        report.update(verdict='failed', error=f'{type(error).__name__}: {error}')
        report.pop('aggregates', None)
        raise
    finally:
        (output / 'receipt.json').write_text(json.dumps(report, indent=2) + '\n')


def artifact_bytes(artifact):
    data = Path(artifact['path']).read_bytes()
    require(data and sha(data) == artifact['sha256'], 'artifact identity mismatch')
    return data


def validate(report, allow_compiler_change=False):
    require(type(allow_compiler_change) is bool, 'invalid compiler opt-in')
    require(report.get('schema') == SCHEMA and report.get('verdict') == 'passed' and not report.get('error'), 'not successful arithmetic suite')
    expected = report['expected_identity']
    require(set(expected) == {'compiler_sha256', 'driver_sha256'}, 'missing explicit attestation')
    check_identity(report['identity_before'], expected)
    require(report['identity_before'] == report['identity_after'], 'compiler changed')
    root = Path(report['workspace'])
    require(root.is_absolute(), 'nonabsolute workspace')
    execution = report['execution']
    require(execution['argv'] == arithmetic.arguments(Path('.')) and execution['cwd'] == 'private-workspace', 'execution template mismatch')
    require(report['actual_execution'] == dict(argv=execution['argv'], cwd=str(root)), 'actual execution mismatch')
    require(set(execution['environment_sha256']) == set(ARMS) and all(re.fullmatch('[0-9a-f]{64}', v) for v in execution['environment_sha256'].values()), 'invalid environment identity')
    require(execution['cc']['path'] == '/usr/bin/cc' and re.fullmatch('[0-9a-f]{64}', execution['cc']['sha256']), 'invalid C compiler identity')
    host = report['host']
    require(host.get('boot_id') and host.get('affinity') and host.get('execution') == 'local-native-owned', 'missing host boot/affinity/execution')
    spec = report['workload']
    require(spec == workload(spec['items'], spec['edits'], spec['repetitions']) and report['workload_sha256'] == sha(canonical(spec)), 'workload mismatch')
    rows = report['rows']
    require([(r['repetition'], r['arm'], r['revision']) for r in rows] == [(rep, arm, rev) for rep in range(spec['repetitions']) for arm in order(rep) for rev in range(spec['edits'] + 1)], 'schedule mismatch')
    previous, pids = {}, {}
    for row in rows:
        rep, arm, rev = row['repetition'], row['arm'], row['revision']
        files, constants = fixture(spec['items'], rev)
        require(row['warmup'] == (rev == 0) and row['validated'] is True, 'invalid sample')
        require(row['sources'] == {n: sha(d) for n, d in files.items()}, 'source identity mismatch')
        require(all(report['source_blobs'].get(sha(d)) == shared.encoded(d) for d in files.values()), 'source blob mismatch')
        require(set(row['artifacts']) == {'o', 'll', 'mir'}, 'missing artifacts')
        for artifact in row['artifacts'].values():
            artifact_bytes(artifact)
        require(artifact_bytes(row['consumer_source']) == consumer_source(constants), 'consumer source mismatch')
        artifact_bytes(row['consumer_binary'])
        require(row['consumer_compile']['command'] == consumer_argv(root) and row['consumer_compile']['status'] == 0 and not row['consumer_compile'].get('timed_out'), 'consumer compilation mismatch')
        require(row['consumer_run']['command'] == [str(root / 'consumer')], 'consumer execution mismatch')
        arithmetic.validate_consumer(row['consumer_run'], expected_output(constants))
        require(row['oracle_sha256'] == sha(expected_output(constants)), 'oracle mismatch')
        record = row['record']
        require(record['status'] == 0 and not record.get('timed_out'), 'failed producer')
        if arm == 'fresh':
            require(record['command'] == [report['identity_before']['path'], *execution['argv']], 'fresh invocation mismatch')
        else:
            key = rep, arm
            pid = record.get('pid')
            require(pid is not None and pid == pids.setdefault(key, pid), 'PID changed')
            require(record.get('request') == dict(id=rev + 1, args=execution['argv']) and record.get('id') == rev + 1 and record['owned']['revisions'] == rev + 1 <= 64, 'revision identity mismatch')
            typed.validate_pair(record, record, constants, pid)
            old = previous.get(key)
            previous[key] = leaf.validate_live(record, old, arm, 'cold' if rev == 0 else 'cross')
            if rev and arm == 'leaf':
                require(previous[key]['mir_leaf_hits'] - old['mir_leaf_hits'] >= spec['items'], 'insufficient cross-edit hits')
    for rep in range(spec['repetitions']):
        for rev in range(spec['edits'] + 1):
            paired = [r for r in rows if r['repetition'] == rep and r['revision'] == rev]
            require(all(shared.observable(r['record']) == shared.observable(paired[0]['record']) for r in paired), 'paired producer diagnostics differ')
            require(all(shared.observable(r['consumer_compile']) == shared.observable(paired[0]['consumer_compile']) for r in paired), 'paired consumer diagnostics differ')
    values = aggregate(rows)
    require(values == report['aggregates'], 'stale aggregate')
    return values


def compare(a, b, floor=10.0, allow_compiler_change=False):
    require(type(floor) in (int, float) and math.isfinite(floor) and floor >= 0, 'invalid floor')
    require(type(allow_compiler_change) is bool, 'invalid compiler opt-in')
    values = [validate(r) for r in (a, b)]
    keys = ('workload', 'workload_sha256', 'host', 'execution')
    if allow_compiler_change:
        require(a['identity_before']['drivers'][0]['sha256'] != b['identity_before']['drivers'][0]['sha256'], 'optimization comparison requires distinct attested drivers')
    else:
        keys += ('identity_before',)
    for key in keys:
        require(a[key] == b[key], 'comparison refuses different ' + key)
    print(CLAIM)
    for label, report in (('before', a), ('after', b)):
        print(label, json.dumps(report['expected_identity'], sort_keys=True))
    for arm in ARMS:
        old, new = (v[arm]['median_seconds'] for v in values)
        delta = (new / old - 1) * 100
        print(f'{arm}: {old:.6f}s -> {new:.6f}s ({delta:+.2f}%), floor {floor:g}% exceeded: {abs(delta) > floor}')


def run(args):
    existed = Path(args.output).exists()
    try:
        report = capture(args.compiler, args.output, args.items, args.edits, args.repetitions, args.timeout,
                         expected_driver_sha256=args.expected_driver_sha256,
                         expected_compiler_sha256=args.expected_compiler_sha256)
        print(json.dumps(dict(receipt=str(args.output / 'receipt.json'), aggregates=report['aggregates'], claim=CLAIM), indent=2))
    finally:
        if not existed:
            base.upload_result(args.output)


def add_parser(sub):
    parser = sub.add_parser('stable-native-arithmetic', help=__doc__)
    parser.add_argument('--compiler', type=Path, required=True)
    parser.add_argument('--output', type=Path, required=True)
    parser.add_argument('--expected-driver-sha256', required=True)
    parser.add_argument('--expected-compiler-sha256', required=True)
    parser.add_argument('--items', type=int, default=128)
    parser.add_argument('--edits', type=int, default=6)
    parser.add_argument('-n', '--repetitions', type=int, default=3)
    parser.add_argument('--timeout', type=float, default=180)
    parser.set_defaults(fn=run)
    return parser


if __name__ == '__main__':
    parser = argparse.ArgumentParser(description=__doc__)
    add_parser(parser.add_subparsers(dest='command', required=True))
    args = parser.parse_args()
    args.fn(args)
