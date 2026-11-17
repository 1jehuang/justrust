#!/usr/bin/env python3
"""Attested ordinary rlib root AST diagnostic. No compiler builds or speed claims.

Only producer invocation wall time is measured. Consumer compilation, execution,
source generation, persistence, startup and two warmup requests are excluded.
Optional --with-lexer composes lexer and AST retention in the enabled arm only.
AST-only remains the default. Receipts from different modes cannot be compared.
"""
import argparse
import base64
import sys
import json
import math
import os
import selectors
import subprocess
from pathlib import Path
import re
import uuid

import stable_resident as shared
import stable_resident_ast as ast
import stable_resident_leaf_suite as base

require, sha, canonical = base.require, base.sha, base.canonical
host_identity, check_identity, attestation = base.host_identity, base.check_identity, base.attestation
SCHEMA = 'stable_native_ast/v1'
ARMS = ('fresh', 'control', 'ast')
CLAIM = ('Ordinary Rust rlib root AST diagnostic only, NOT general Cargo, fullgate or '
         'realproject performance. Descriptive producer invocation timings only.')
INPUTS = tuple((x, y) for x in ast.EDGES for y in (0, 7, 4294967295))


def order(repetition):
    return ['ast' if arm == 'leaf' else arm for arm in base.order(repetition)]


def aggregate(rows):
    translated = [dict(row, arm='leaf' if row['arm'] == 'ast' else row['arm']) for row in rows]
    return {('ast' if arm == 'leaf' else arm): value
            for arm, value in base.aggregate(translated).items()}


def environment(arm, with_lexer=False):
    require(type(with_lexer) is bool, 'invalid lexer composition mode')
    require(arm in ARMS, 'invalid arm')
    env = ast.environment('1' if arm == 'ast' else '0')
    if with_lexer and arm == 'ast':
        env['JUSTRUST_RUSTC_RESIDENT_LEXER'] = '1'
    return env


class AstResident(ast.AstResident):
    def __init__(self, compiler, root, arm, timeout, with_lexer=False):
        env = environment(arm, with_lexer)
        self.environment_sha256 = sha(canonical(env))
        self.command = [str(compiler), '--jr-resident-owned']
        self.root, self.timeout = root, timeout
        self.stdout = (root / 'resident.stdout').open('w+b', buffering=0)
        self.stderr = (root / 'resident.stderr').open('w+b', buffering=0)
        self.transcript = (root / 'resident.jsonl').open('wb')
        read_fd, write_fd = os.pipe()
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


def arguments(root):
    return [str(root / 'fixture.rs'), '--crate-name=ast_fixture',
            '--crate-type=rlib', '--edition=2024', '-Copt-level=0',
            '-Cdebuginfo=0', '--emit=link', '-o', str(root / 'libfixture.rlib')]


def fixture(items, revision):
    require(type(items) is int and 1 <= items <= 128, 'items outside 1..128')
    require(type(revision) is int and 0 <= revision <= 32, 'revision outside 0..32')
    constants = {f'value_{index:04d}': 7 + index for index in range(items)}
    root = 'pub mod helper;\n' + ''.join(
        f'pub fn {name}(x:u32, y:u32)->u32 {{ ((x ^ y) ^ {value}) & 255 }}\n'
        for name, value in constants.items())
    constants['helper::value'] = 11 + revision
    helper = f'pub fn value(x:u32, y:u32)->u32 {{ ((x ^ y) ^ {11 + revision}) & 255 }}\n'
    return {'fixture.rs': root.encode('ascii'), 'helper.rs': helper.encode('ascii')}, constants


def expected_output(constants, nonce):
    return (nonce + '\n' + ''.join(f'{((x ^ y) ^ value) & 255}\n'
            for value in constants.values() for x, y in INPUTS)).encode('ascii')


def consumer_source(constants, nonce):
    require(isinstance(nonce, str) and re.fullmatch('[0-9a-f]{32}', nonce), 'invalid runtime nonce')
    calls = ''.join(f'println!("{{}}", ast_fixture::{name}({x}, {y}));\n'
                    for name in constants for x, y in INPUTS)
    return ('extern crate ast_fixture;\nfn main() {\nprintln!("' + nonce + '");\n'
            + calls + '}\n').encode('ascii')


def consumer_argv(compiler, root):
    return [str(compiler), str(root / 'consumer.rs'), '--crate-name=ast_consumer',
            '--edition=2024', '-Copt-level=0', '-Cdebuginfo=0',
            '--extern', 'ast_fixture=' + str(root / 'libfixture.rlib'),
            '-o', str(root / 'consumer')]


def workload(items, edits, repetitions, with_lexer=False):
    require(type(with_lexer) is bool, 'invalid lexer composition mode')
    require(type(edits) is int and 2 <= edits <= 32, 'edits outside 2..32')
    require(type(repetitions) is int and repetitions >= 1, 'repetitions must be positive')
    require(edits + 2 <= 64, 'resident request lifetime exceeded')
    revisions = []
    for revision in range(edits + 1):
        files, constants = fixture(items, revision)
        revisions.append(dict(revision=revision, sources={n: sha(d) for n, d in files.items()}, constants=constants))
    return dict(fixture='ordinary-root-ast-helper-arithmetic-rlib-v1', **({'with_lexer': True} if with_lexer else {}), items=items,
                edits=edits, repetitions=repetitions, revisions=revisions,
                inputs=list(map(list, INPUTS)), order=[order(r) for r in range(repetitions)],
                timing='producer-invocation-wall-only', warmup='cold+unchanged excluded',
                incremental='disabled',
                producer_outputs='ordinary rlib link only', lifetime=64)


def execution_identity(with_lexer=False):
    modules = (Path(__file__), Path(ast.__file__), Path(ast.owned.__file__),
               Path(shared.__file__), Path(base.__file__))
    return dict(argv=arguments(Path('.')), cwd='private-workspace',
                adapter_sha256={p.name: sha(p.read_bytes()) for p in modules},
                environment_sha256={a: sha(canonical(environment(a, with_lexer))) for a in ARMS},
                common_environment_sha256={a: sha(canonical({k: v for k, v in environment(a, with_lexer).items()
                    if k not in ({'JUSTRUST_RUSTC_RESIDENT_AST', 'JUSTRUST_RUSTC_RESIDENT_LEXER'} if with_lexer else {'JUSTRUST_RUSTC_RESIDENT_AST'})})) for a in ARMS},
                gates={a: {k: v for k, v in environment(a, with_lexer).items()
                           if k.startswith('JUSTRUST_RUSTC_RESIDENT_')} for a in ARMS})


def save_file(path, destination):
    data = path.read_bytes()
    destination.write_bytes(data)
    return dict(path=str(destination), sha256=sha(data))


def artifact_bytes(artifact, allow_empty=False):
    data = Path(artifact['path']).read_bytes()
    require((data or allow_empty) and sha(data) == artifact['sha256'], 'artifact identity mismatch')
    return data


def validate_runtime(record, expected):
    require(type(record['status']) is int and record['status'] == 0 and not record.get('timed_out'), 'consumer execution failed')
    require(record['stdout_b64'] == shared.encoded(expected) and record['stderr_b64'] == '', 'runtime oracle mismatch')


def validate_live(record, previous, arm, kind, with_lexer=False):
    require(type(with_lexer) is bool, 'invalid lexer composition mode')
    ast_record, ast_previous = record, previous
    if with_lexer:
        lexer = record.get('lexer')
        require(isinstance(lexer, dict) and set(lexer) == set(ast.COUNTERS), 'unknown lexer telemetry')
        require(all(type(lexer[k]) is int and lexer[k] >= 0 for k in ast.COUNTERS), 'invalid lexer telemetry')
        old = previous['lexer'] if previous else dict.fromkeys(ast.COUNTERS, 0)
        delta = {k: lexer[k] - old[k] for k in ast.COUNTERS[:3]}
        if arm == 'ast':
            # Root lexing still runs on AST hits, preserving parser effects.
            expected = {'hits': 0, 'misses': 2, 'rejected': 0} if kind == 'cold' else {
                'hits': 2 if kind == 'unchanged' else 1,
                'misses': 0 if kind == 'unchanged' else 1, 'rejected': 0}
            require(delta == expected, 'exact composed lexer delta required')
            require(2 <= lexer['retained_entries'] <= 128 and
                    0 < lexer['retained_estimated_bytes'] <= 8 * 1024 * 1024, 'lexer retention bounds exceeded')
        else:
            require(all(v == 0 for v in lexer.values()), 'disabled lexer telemetry nonzero')
        # Reuse the AST-only validator without weakening its independent checks.
        ast_record = dict(record, lexer=dict.fromkeys(ast.COUNTERS, 0))
        ast_previous = previous['ast'] if previous else None
    stats = ast.validate_stats(ast_record, ast_previous, '1' if arm == 'ast' else '0',
                               'cold' if kind == 'cold' else 'hit')
    require(set(stats) == set(ast.COUNTERS), 'unknown AST telemetry')
    require(stats['retained_entries'] <= 32 and stats['retained_estimated_bytes'] <= 8 * 1024 * 1024,
            'AST retention bounds exceeded')
    if arm == 'ast':
        require(stats['rejected'] == 0, 'admitted root rejected')
    return dict(ast=stats, lexer=dict(lexer)) if with_lexer else stats


def capture(compiler, output, items=128, edits=6, repetitions=3, timeout=180,
            *, expected_driver_sha256, expected_compiler_sha256, with_lexer=False):
    expected = attestation(expected_driver_sha256, expected_compiler_sha256)
    compiler = Path(compiler).resolve()
    require(compiler.is_file() and sha(compiler.read_bytes()) == expected['compiler_sha256'], 'unattested compiler before execution')
    drivers = [p for p in sorted((compiler.parent.parent / 'lib').glob('*rustc_driver*')) if p.is_file()]
    require([sha(p.read_bytes()) for p in drivers] == [expected['driver_sha256']], 'unattested driver before execution')
    require(type(timeout) in (int, float) and math.isfinite(timeout) and timeout > 0, 'invalid timeout')
    spec = workload(items, edits, repetitions, with_lexer)
    output = Path(output).resolve()
    output.mkdir(mode=0o700, parents=True, exist_ok=False)
    workspace = output / 'workspace'
    workspace.mkdir(mode=0o700)
    report = dict(schema=SCHEMA, claim=CLAIM, verdict='incomplete', workload=spec,
                  workload_sha256=sha(canonical(spec)), expected_identity=expected,
                  host=host_identity(), execution=execution_identity(with_lexer), workspace=str(workspace),
                  actual_execution=dict(argv=arguments(Path('.')), cwd=str(workspace)), rows=[], source_blobs={}, sessions=[])
    argv = report['execution']['argv']
    try:
        report['identity_before'] = shared.identity(compiler)
        check_identity(report['identity_before'], expected)
        for repetition in range(repetitions):
            for arm in order(repetition):
                evidence = output / f'rep-{repetition:03d}-{arm}'
                evidence.mkdir(mode=0o700)
                worker, previous = None, None
                session = dict(repetition=repetition, arm=arm, streams={})
                report['sessions'].append(session)
                try:
                    if arm != 'fresh':
                        worker = AstResident(compiler, workspace, arm, timeout, with_lexer=True) if with_lexer else AstResident(compiler, workspace, arm, timeout)
                        session.update(pid=worker.process.pid, command=worker.command,
                                       environment_sha256=worker.environment_sha256)
                        require(worker.environment_sha256 == report['execution']['environment_sha256'][arm], 'worker environment mismatch')
                    for step in range(edits + 2):
                        revision = max(0, step - 1)
                        kind = 'cold' if step == 0 else 'unchanged' if step == 1 else 'edit'
                        require(execution_identity(with_lexer) == report['execution'], 'execution environment changed')
                        files, constants = fixture(items, revision)
                        for name, data in files.items():
                            path = workspace / name
                            if not path.exists() or path.read_bytes() != data:
                                path.write_bytes(data)
                            report['source_blobs'][sha(data)] = shared.encoded(data)
                        row = dict(repetition=repetition, arm=arm, revision=revision, step=step,
                                   kind=kind, warmup=step < 2, sources={n: sha(d) for n, d in files.items()},
                                   validated=False, artifacts={}, nonce=uuid.uuid4().hex)
                        report['rows'].append(row)
                        (workspace / 'libfixture.rlib').unlink(missing_ok=True)
                        try:
                            env = environment(arm, with_lexer)
                            row['record'] = shared.run([compiler, *argv], timeout, cwd=workspace, env=env) if worker is None else worker.compile(argv)
                            path = workspace / 'libfixture.rlib'
                            if path.exists():
                                row['artifacts']['rlib'] = save_file(path, evidence / f'{step:02d}.rlib')
                            require(row['record']['status'] == 0 and not row['record'].get('timed_out'), 'producer failed')
                            require(set(row['artifacts']) == {'rlib'}, 'missing ordinary rlib')
                            require(all((workspace / n).read_bytes() == d for n, d in files.items()), 'source changed during producer')
                            source = workspace / 'consumer.rs'
                            source.write_bytes(consumer_source(constants, row['nonce']))
                            row['consumer_source'] = save_file(source, evidence / f'{step:02d}.consumer.rs')
                            binary = workspace / 'consumer'
                            binary.unlink(missing_ok=True)
                            row['consumer_rlib_sha256'] = sha(path.read_bytes())
                            require(row['consumer_rlib_sha256'] == row['artifacts']['rlib']['sha256'], 'producer artifact changed before link')
                            row['consumer_compile'] = shared.run(consumer_argv(compiler, workspace), timeout, cwd=workspace, env=env)
                            if binary.exists():
                                row['consumer_binary'] = save_file(binary, evidence / f'{step:02d}.consumer.bin')
                            require(row['consumer_compile']['status'] == 0 and not row['consumer_compile'].get('timed_out'), 'Rust consumer link failed')
                            require(sha(path.read_bytes()) == row['consumer_rlib_sha256'], 'producer artifact changed during link')
                            row['consumer_run'] = shared.run([str(binary)], timeout, cwd=workspace, env=env)
                            require(sha(binary.read_bytes()) == row['consumer_binary']['sha256'], 'consumer binary changed during execution')
                            validate_runtime(row['consumer_run'], expected_output(constants, row['nonce']))
                            row['oracle_sha256'] = sha(expected_output(constants, row['nonce']))
                            if worker is not None:
                                require(row['record'].get('pid') == worker.process.pid, 'PID changed')
                                previous = validate_live(row['record'], previous, arm, kind, with_lexer)
                            require(all((workspace / n).read_bytes() == d for n, d in files.items()), 'source changed during consumer')
                            row['validated'] = True
                        finally:
                            require(not (workspace / 'incremental').exists(), 'unexpected incremental state')
                finally:
                    failing = sys.exc_info()[0] is not None
                    try:
                        if worker is not None:
                            try:
                                worker.close()
                            except BaseException as close_error:
                                session['close_error'] = f'{type(close_error).__name__}: {close_error}'
                                if not failing:
                                    raise
                    finally:
                        for stream in ('resident.stdout', 'resident.stderr', 'resident.jsonl'):
                            path = workspace / stream
                            if path.exists():
                                session['streams'][stream] = save_file(path, evidence / stream)
                                path.unlink()
        report['identity_after'] = shared.identity(compiler)
        require(report['identity_after'] == report['identity_before'], 'compiler identity changed')
        require(host_identity() == report['host'], 'host changed')
        require(execution_identity(with_lexer) == report['execution'], 'execution changed')
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


def validate(report):
    require(report.get('schema') == SCHEMA and report.get('verdict') == 'passed' and not report.get('error'), 'not successful ast suite')
    with_lexer = report['workload'].get('with_lexer', False)
    require(type(with_lexer) is bool, 'invalid lexer composition mode')
    expected = report['expected_identity']
    require(set(expected) == {'compiler_sha256', 'driver_sha256'}, 'missing explicit attestation')
    expected = attestation(expected['driver_sha256'], expected['compiler_sha256'])
    check_identity(report['identity_before'], expected)
    require(report['identity_before'] == report['identity_after'], 'compiler changed')
    root = Path(report['workspace'])
    require(root.is_absolute(), 'nonabsolute workspace')
    execution = report['execution']
    require(set(execution.get('adapter_sha256', {})) == {'stable_resident_ast_suite.py', 'stable_resident_ast.py', 'stable_resident_owned.py', 'stable_resident.py', 'stable_resident_leaf_suite.py'} and all(re.fullmatch('[0-9a-f]{64}', v) for v in execution['adapter_sha256'].values()), 'missing adapter provenance')
    require(execution['argv'] == arguments(Path('.')) and execution['cwd'] == 'private-workspace', 'execution template mismatch')
    require(report['actual_execution'] == dict(argv=execution['argv'], cwd=str(root)), 'actual execution mismatch')
    require(set(execution['environment_sha256']) == set(ARMS) and all(re.fullmatch('[0-9a-f]{64}', v) for v in execution['environment_sha256'].values()), 'invalid environment identity')
    require(set(execution['common_environment_sha256']) == set(ARMS) and len(set(execution['common_environment_sha256'].values())) == 1 and all(re.fullmatch('[0-9a-f]{64}', v) for v in execution['common_environment_sha256'].values()), 'arms differ beyond ast flag')
    require(execution['environment_sha256']['fresh'] == execution['environment_sha256']['control'], 'fresh/control environments differ')
    require(execution['gates'] == {a: {'JUSTRUST_RUSTC_RESIDENT_TYPED': '0', 'JUSTRUST_RUSTC_RESIDENT_LEXER': '1' if with_lexer and a == 'ast' else '0', 'JUSTRUST_RUSTC_RESIDENT_AST': '1' if a == 'ast' else '0'} for a in ARMS}, 'unsafe retention gates')
    host = report['host']
    require(re.fullmatch(r'[0-9a-f]{8}(?:-[0-9a-f]{4}){3}-[0-9a-f]{12}', host.get('boot_id', '')) and host.get('affinity') and host.get('execution') == 'local-native-owned', 'missing host boot/affinity/execution')
    require(host['affinity'] == sorted(set(host['affinity'])) and all(type(v) is int and v >= 0 for v in host['affinity']), 'invalid affinity')
    spec = report['workload']
    require(spec == workload(spec['items'], spec['edits'], spec['repetitions'], with_lexer) and report['workload_sha256'] == sha(canonical(spec)), 'workload mismatch')
    rows = report['rows']
    require([(r['repetition'], r['arm'], r['step']) for r in rows] == [(rep, arm, step) for rep in range(spec['repetitions']) for arm in order(rep) for step in range(spec['edits'] + 2)], 'schedule mismatch')
    previous, pids, nonces, previous_sources = {}, {}, set(), {}
    for row in rows:
        rep, arm, step = row['repetition'], row['arm'], row['step']
        rev = max(0, step - 1)
        kind = 'cold' if step == 0 else 'unchanged' if step == 1 else 'edit'
        files, constants = fixture(spec['items'], rev)
        require(row['revision'] == rev and row['kind'] == kind and row['warmup'] == (step < 2) and row['validated'] is True, 'invalid sample')
        require(row['sources'] == {n: sha(d) for n, d in files.items()}, 'source identity mismatch')
        require(all(report['source_blobs'].get(sha(d)) == shared.encoded(d) for d in files.values()), 'source blob mismatch')
        key = rep, arm
        old_sources = previous_sources.get(key)
        if old_sources is not None:
            changed = [n for n in files if old_sources[n] != row['sources'][n]]
            require(changed == ([] if step == 1 else ['helper.rs']), 'unexpected source edit')
        previous_sources[key] = row['sources']
        require(row['nonce'] not in nonces, 'reused runtime nonce')
        nonces.add(row['nonce'])
        require(set(row['artifacts']) == {'rlib'}, 'missing artifacts')
        require(artifact_bytes(row['artifacts']['rlib']).startswith(b'!<arch>\n'), 'not a Rust rlib archive')
        require(artifact_bytes(row['consumer_source']) == consumer_source(constants, row['nonce']), 'consumer source mismatch')
        require(row['consumer_rlib_sha256'] == row['artifacts']['rlib']['sha256'], 'consumer linked artifact mismatch')
        artifact_bytes(row['consumer_binary'])
        require(row['consumer_compile']['command'] == consumer_argv(report['identity_before']['path'], root) and row['consumer_compile']['status'] == 0 and not row['consumer_compile'].get('timed_out'), 'consumer compilation mismatch')
        require(row['consumer_run']['command'] == [str(root / 'consumer')], 'consumer execution mismatch')
        validate_runtime(row['consumer_run'], expected_output(constants, row['nonce']))
        require(row['oracle_sha256'] == sha(expected_output(constants, row['nonce'])), 'oracle mismatch')
        record = row['record']
        seconds = record.get('elapsed_seconds')
        require(type(seconds) in (int, float) and math.isfinite(seconds) and seconds > 0, 'invalid producer timing')
        require(type(record['status']) is int and record['status'] == 0 and not record.get('timed_out'), 'failed producer')
        if arm == 'fresh':
            require(record['command'] == [report['identity_before']['path'], *execution['argv']], 'fresh invocation mismatch')
        else:
            pid = record.get('pid')
            require(type(pid) is int and pid > 0 and pid == pids.setdefault(key, pid), 'PID changed')
            require(record.get('request') == dict(id=step + 1, args=execution['argv']) and record.get('id') == step + 1 and record['owned']['revisions'] == step + 1 <= 64, 'revision identity mismatch')
            previous[key] = validate_live(record, previous.get(key), arm, kind, with_lexer)
    for rep in range(spec['repetitions']):
        for step in range(spec['edits'] + 2):
            paired = [r for r in rows if r['repetition'] == rep and r['step'] == step]
            require(all(shared.observable(r['record']) == shared.observable(paired[0]['record']) for r in paired), 'paired producer diagnostics differ')
    validate_sessions(report)
    values = aggregate(rows)
    require(values == report['aggregates'], 'stale aggregate')
    return values


def validate_sessions(report):
    sessions = report['sessions']
    spec = report['workload']
    require([(s['repetition'], s['arm']) for s in sessions] ==
            [(rep, arm) for rep in range(spec['repetitions']) for arm in order(rep)], 'session schedule mismatch')
    for session in sessions:
        arm, rep = session['arm'], session['repetition']
        if arm == 'fresh':
            require(session == dict(repetition=rep, arm=arm, streams={}), 'fresh worker evidence unexpected')
            continue
        require(session['command'] == [report['identity_before']['path'], '--jr-resident-owned'], 'worker command mismatch')
        require(session['environment_sha256'] == report['execution']['environment_sha256'][arm], 'worker environment mismatch')
        require(set(session['streams']) == {'resident.stdout', 'resident.stderr', 'resident.jsonl'}, 'missing original transport')
        stdout = artifact_bytes(session['streams']['resident.stdout'], True)
        stderr = artifact_bytes(session['streams']['resident.stderr'], True)
        frames = [json.loads(line) for line in artifact_bytes(session['streams']['resident.jsonl']).splitlines()]
        rows = [r for r in report['rows'] if r['repetition'] == rep and r['arm'] == arm]
        require(len(frames) == 2 * len(rows), 'transport frame count mismatch')
        require(stdout == b''.join(base64.b64decode(r['record']['stdout_b64'], validate=True) for r in rows), 'original stdout mismatch')
        require(stderr == b''.join(base64.b64decode(r['record']['stderr_b64'], validate=True) for r in rows), 'original stderr mismatch')
        for index, row in enumerate(rows):
            record = row['record']
            require(record['pid'] == session['pid'], 'session PID mismatch')
            require(frames[2 * index] == dict(request=record['request']), 'original request mismatch')
            response = json.loads(base64.b64decode(frames[2 * index + 1]['response_b64'], validate=True))
            require(response.get('elapsed_seconds') == record.get('worker_elapsed_seconds'), 'original worker timing mismatch')
            for key, value in response.items():
                if key != 'elapsed_seconds':
                    require(record.get(key) == value, 'original completion mismatch: ' + key)


def compare(a, b, floor=10):
    require(type(floor) in (int, float) and math.isfinite(floor) and floor >= 0, 'invalid floor')
    values = [validate(r) for r in (a, b)]
    keys = ('workload', 'workload_sha256', 'host', 'execution', 'identity_before', 'expected_identity')
    for key in keys:
        require(a[key] == b[key], 'comparison refuses different ' + key)
    print(CLAIM)
    for label, report, value in zip(('before', 'after'), (a, b), values):
        print(label, json.dumps(report['expected_identity'], sort_keys=True))
        mode = 'ast+lexer' if report['workload'].get('with_lexer', False) else 'ast'
        print(f"{label} {mode}/processcontrol: {value['ast']['median_seconds'] / value['control']['median_seconds']:.4f}x")
    for arm in ARMS:
        old, new = (v[arm]['median_seconds'] for v in values)
        delta = (new / old - 1) * 100
        print(f'{arm}: {old:.6f}s -> {new:.6f}s ({new / old:.4f}x, {delta:+.2f}%), floor {floor:g}% exceeded: {abs(delta) > floor}')


def run(args):
    existed = Path(args.output).exists()
    try:
        report = capture(args.compiler, args.output, args.items, args.edits, args.repetitions, args.timeout,
                         expected_driver_sha256=args.expected_driver_sha256,
                         expected_compiler_sha256=args.expected_compiler_sha256, with_lexer=getattr(args, 'with_lexer', False))
        print(json.dumps(dict(receipt=str(args.output / 'receipt.json'), aggregates=report['aggregates'], with_lexer=report['workload'].get('with_lexer', False), claim=CLAIM), indent=2))
    finally:
        if not existed:
            base.upload_result(args.output)


def add_parser(sub):
    parser = sub.add_parser('stable-native-ast', help=__doc__)
    parser.add_argument('--compiler', type=Path, required=True)
    parser.add_argument('--output', type=Path, required=True)
    parser.add_argument('--expected-driver-sha256', required=True)
    parser.add_argument('--expected-compiler-sha256', required=True)
    parser.add_argument('--items', type=int, default=128)
    parser.add_argument('--edits', type=int, default=6)
    parser.add_argument('-n', '--repetitions', type=int, default=3)
    parser.add_argument('--with-lexer', action='store_true', help='Compose lexer retention with AST retention in the enabled arm only')
    parser.add_argument('--timeout', type=float, default=180)
    parser.set_defaults(fn=run)
    return parser


if __name__ == '__main__':
    parser = argparse.ArgumentParser(description=__doc__)
    add_parser(parser.add_subparsers(dest='command', required=True))
    args = parser.parse_args()
    args.fn(args)
