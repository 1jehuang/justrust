#!/usr/bin/env python3
"""Attested ordinary rlib lexer diagnostic. No compiler builds or speed claims.

Only producer invocation wall time is measured. Consumer compilation, execution,
source generation, persistence, startup and two warmup requests are excluded.
"""
import argparse
import base64
import os
import selectors
import subprocess
import json
import math
from pathlib import Path
import re
import uuid

import stable_resident as shared
import stable_resident_typed as typed
import stable_resident_lexer as lexer
import stable_resident_leaf_suite as base

require, sha, canonical = base.require, base.sha, base.canonical
host_identity, check_identity, attestation = base.host_identity, base.check_identity, base.attestation
SCHEMA = 'stable_native_lexer/v1'
ARMS = ('fresh', 'control', 'lexer')
CLAIM = ('Ordinary Rust rlib lexer diagnostic only, NOT general Cargo, fullgate or '
         'realproject performance. Descriptive producer invocation timings only.')
INPUTS = ((0, 0), (1, 7), (19, 3), (255, 511))
ROOT = b'pub mod module_a;\npub mod module_b;\n'


def order(repetition):
    return ['lexer' if arm == 'leaf' else arm for arm in base.order(repetition)]


def aggregate(rows):
    translated = [dict(row, arm='leaf' if row['arm'] == 'lexer' else row['arm']) for row in rows]
    return {('lexer' if arm == 'leaf' else arm): value
            for arm, value in base.aggregate(translated).items()}


def environment(arm):
    require(arm in ARMS, 'invalid arm')
    env = typed.environment('control')
    env['JUSTRUST_RUSTC_RESIDENT_TYPED'] = '0'
    env['JUSTRUST_RUSTC_RESIDENT_LEXER'] = '1' if arm == 'lexer' else '0'
    return env


class LexerResident(lexer.LexerResident):
    """Reuse native transport, explicitly disable typed retention at startup."""
    def __init__(self, compiler, root, arm, timeout):
        self.command = [str(compiler), '--jr-resident-owned']
        self.root, self.timeout = root, timeout
        self.stdout = (root / 'resident.stdout').open('w+b', buffering=0)
        self.stderr = (root / 'resident.stderr').open('w+b', buffering=0)
        self.transcript = (root / 'resident.jsonl').open('wb')
        read_fd, write_fd = os.pipe()
        env = environment(arm)
        self.environment_sha256 = sha(canonical(env))
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
    return [str(root / 'fixture.rs'), '--crate-name=lexer_fixture',
            '--crate-type=rlib', '--edition=2024', '-Copt-level=0',
            '-Cdebuginfo=0', '-Cincremental=incremental', '--emit=link', '-o', str(root / 'libfixture.rlib')]


def fixture(items, revision):
    require(type(items) is int and 1 <= items <= 128, 'items outside 1..128')
    require(type(revision) is int and 0 <= revision <= 32, 'revision outside 0..32')
    files, constants = {'fixture.rs': ROOT}, {}
    for module in ('a', 'b'):
        lines = []
        for index in range(items):
            updates = (revision + (module == 'b')) // 2
            value = (7 if module == 'a' else 11) + index + (updates if index == 0 else 0)
            name = f'module_{module}::value_{index:04d}'
            constants[name] = value
            lines.append(f'pub fn value_{index:04d}(x: u32, y: u32) -> u32 {{ ((x + y) ^ {value}) * 3 }}\n')
        files[f'module_{module}.rs'] = ''.join(lines).encode('ascii')
    return files, constants


def expected_output(constants, nonce):
    return (nonce + '\n' + ''.join(f'{((x + y) ^ value) * 3}\n'
            for value in constants.values() for x, y in INPUTS)).encode('ascii')


def consumer_source(constants, nonce):
    require(isinstance(nonce, str) and re.fullmatch('[0-9a-f]{32}', nonce), 'invalid runtime nonce')
    calls = ''.join(f'println!("{{}}", lexer_fixture::{name}({x}, {y}));\n'
                    for name in constants for x, y in INPUTS)
    return ('extern crate lexer_fixture;\nfn main() {\nprintln!("' + nonce + '");\n'
            + calls + '}\n').encode('ascii')


def consumer_argv(compiler, root):
    return [str(compiler), str(root / 'consumer.rs'), '--crate-name=lexer_consumer',
            '--edition=2024', '-Copt-level=0', '-Cdebuginfo=0',
            '--extern', 'lexer_fixture=' + str(root / 'libfixture.rlib'),
            '-o', str(root / 'consumer')]


def workload(items, edits, repetitions):
    require(type(edits) is int and 2 <= edits <= 32, 'edits outside 2..32')
    require(type(repetitions) is int and repetitions >= 1, 'repetitions must be positive')
    revisions = []
    for revision in range(edits + 1):
        files, constants = fixture(items, revision)
        revisions.append(dict(revision=revision, sources={n: sha(d) for n, d in files.items()}, constants=constants))
    return dict(fixture='ordinary-ascii-public-arithmetic-rlib-v1', items=items,
                edits=edits, repetitions=repetitions, revisions=revisions,
                inputs=list(map(list, INPUTS)), order=[order(r) for r in range(repetitions)],
                timing='producer-invocation-wall-only', warmup='cold+unchanged excluded',
                incremental='independent bank per repetition and arm',
                producer_outputs='ordinary rlib link only', lifetime=64)


def execution_identity():
    return dict(argv=arguments(Path('.')), cwd='private-workspace',
                environment_sha256={a: sha(canonical(environment(a))) for a in ARMS},
                common_environment_sha256={a: sha(canonical({k: v for k, v in environment(a).items()
                    if k != 'JUSTRUST_RUSTC_RESIDENT_LEXER'})) for a in ARMS},
                gates={a: {k: v for k, v in environment(a).items()
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


def validate_live(record, previous, arm, kind):
    require(record.get('executed') is True and record['owned'].get('retirement_reason') is None, 'worker did not execute or retired')
    live = record.get('live')
    require(isinstance(live, dict), 'missing typed/MIR telemetry')
    numeric = (*typed.COUNTERS, 'mir_leaf_hits', 'mir_leaf_candidate_misses',
               'identity_begin_rejections', 'identity_complete_rejections',
               'published_revisions', 'aborted_revisions')
    require(set(live) == {*numeric, 'mir_leaf_last_rejection'}, 'unknown typed/MIR telemetry')
    require(live['mir_leaf_last_rejection'] is None, 'typed/MIR rejection while disabled')
    for key in numeric:
        require(type(live.get(key)) is int and live[key] == 0, 'typed/MIR retention must be disabled: ' + key)
    stats = record.get('lexer')
    require(isinstance(stats, dict) and set(stats) == set(lexer.COUNTERS), 'missing or unknown lexer counters')
    require(all(type(v) is int and v >= 0 for v in stats.values()), 'invalid lexer counters')
    require(stats['retained_entries'] <= 128 and stats['retained_estimated_bytes'] <= 8 * 1024 * 1024, 'lexer bounds exceeded')
    for key in lexer.COUNTERS[:3]:
        require(previous is None or stats[key] >= previous[key], 'counter decreased')
    if arm == 'control':
        require(not any(stats.values()), 'disabled lexer fields nonzero')
    else:
        hits = stats['hits'] - (previous['hits'] if previous else 0)
        if kind == 'cold':
            require(hits == 0, 'cold lexer hit')
        else:
            require(hits > 0 and stats['retained_entries'] > 0, 'no unchanged/unrelated-file lexer hits')
        if kind == 'edit':
            require(stats['misses'] > previous['misses'], 'edited source did not miss')
    return dict(stats)


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
                  actual_execution=dict(argv=arguments(Path('.')), cwd=str(workspace)), rows=[], source_blobs={}, sessions=[])
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
                session = dict(repetition=repetition, arm=arm, streams={})
                report['sessions'].append(session)
                try:
                    if arm != 'fresh':
                        worker = LexerResident(compiler, workspace, arm, timeout)
                        session.update(pid=worker.process.pid, command=worker.command,
                                       environment_sha256=worker.environment_sha256)
                        require(worker.environment_sha256 == report['execution']['environment_sha256'][arm], 'worker environment mismatch')
                    for step in range(edits + 2):
                        revision = max(0, step - 1)
                        kind = 'cold' if step == 0 else 'unchanged' if step == 1 else 'edit'
                        require(execution_identity() == report['execution'], 'execution environment changed')
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
                        if bank.exists():
                            bank.rename(active)
                        (workspace / 'libfixture.rlib').unlink(missing_ok=True)
                        try:
                            env = environment(arm)
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
                                previous = validate_live(row['record'], previous, arm, kind)
                            require(all((workspace / n).read_bytes() == d for n, d in files.items()), 'source changed during consumer')
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
                                session['streams'][stream] = save_file(path, evidence / stream)
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


def validate(report, allow_compiler_change=False):
    require(type(allow_compiler_change) is bool, 'invalid compiler opt-in')
    require(report.get('schema') == SCHEMA and report.get('verdict') == 'passed' and not report.get('error'), 'not successful lexer suite')
    expected = report['expected_identity']
    require(set(expected) == {'compiler_sha256', 'driver_sha256'}, 'missing explicit attestation')
    expected = attestation(expected['driver_sha256'], expected['compiler_sha256'])
    check_identity(report['identity_before'], expected)
    require(report['identity_before'] == report['identity_after'], 'compiler changed')
    root = Path(report['workspace'])
    require(root.is_absolute(), 'nonabsolute workspace')
    execution = report['execution']
    require(execution['argv'] == arguments(Path('.')) and execution['cwd'] == 'private-workspace', 'execution template mismatch')
    require(report['actual_execution'] == dict(argv=execution['argv'], cwd=str(root)), 'actual execution mismatch')
    require(set(execution['environment_sha256']) == set(ARMS) and all(re.fullmatch('[0-9a-f]{64}', v) for v in execution['environment_sha256'].values()), 'invalid environment identity')
    require(set(execution['common_environment_sha256']) == set(ARMS) and len(set(execution['common_environment_sha256'].values())) == 1 and all(re.fullmatch('[0-9a-f]{64}', v) for v in execution['common_environment_sha256'].values()), 'arms differ beyond lexer flag')
    require(execution['environment_sha256']['fresh'] == execution['environment_sha256']['control'], 'fresh/control environments differ')
    require(execution['gates'] == {a: {'JUSTRUST_RUSTC_RESIDENT_TYPED': '0', 'JUSTRUST_RUSTC_RESIDENT_LEXER': '1' if a == 'lexer' else '0'} for a in ARMS}, 'unsafe retention gates')
    host = report['host']
    require(re.fullmatch(r'[0-9a-f]{8}(?:-[0-9a-f]{4}){3}-[0-9a-f]{12}', host.get('boot_id', '')) and host.get('affinity') and host.get('execution') == 'local-native-owned', 'missing host boot/affinity/execution')
    require(host['affinity'] == sorted(set(host['affinity'])) and all(type(v) is int and v >= 0 for v in host['affinity']), 'invalid affinity')
    spec = report['workload']
    require(spec == workload(spec['items'], spec['edits'], spec['repetitions']) and report['workload_sha256'] == sha(canonical(spec)), 'workload mismatch')
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
            require(changed == ([] if step == 1 else ['module_b.rs' if rev % 2 else 'module_a.rs']), 'unexpected source edit')
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
            previous[key] = validate_live(record, previous.get(key), arm, kind)
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


def compare(a, b, floor=10, allow_compiler_change=False):
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
    for label, report, value in zip(('before', 'after'), (a, b), values):
        print(label, json.dumps(report['expected_identity'], sort_keys=True))
        print(f"{label} lexer/processcontrol: {value['lexer']['median_seconds'] / value['control']['median_seconds']:.4f}x")
    for arm in ARMS:
        old, new = (v[arm]['median_seconds'] for v in values)
        delta = (new / old - 1) * 100
        print(f'{arm}: {old:.6f}s -> {new:.6f}s ({new / old:.4f}x, {delta:+.2f}%), floor {floor:g}% exceeded: {abs(delta) > floor}')


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
    parser = sub.add_parser('stable-native-lexer', help=__doc__)
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
