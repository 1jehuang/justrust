#!/usr/bin/env python3
"""Versioned frozen1527 primitive-literal leaf diagnostic, not a broad gate.

Only compiler invocation wall time is aggregated. Source writes, artifact oracles,
worker startup, identity queries and warmup are outside the timed invocation.
"""
import argparse
import hashlib
import json
import math
import os
from pathlib import Path
import platform
import re
import statistics
import shutil
import subprocess
import sys

import stable_resident as shared
import stable_resident_typed as typed
import stable_resident_mir_leaf as leaf

SCHEMA = 'stable_native_leaf/v1'
FIXTURE = 'three-file-no-core-primitive-literal-v1'
COMPILER = Path.home() / '.jcode/scratch/stable-resident-leaf-20261116-1527-sysroot/bin/rustc'
BINARY_SHA = '8cb4b7089e2e70cbd4832fdf53a24ef2e27db676dcc3632c2501134e6f3dd567'
DRIVER_SHA = 'c1bf09a1bd1e88188dbfb238303c213a7e52aad727a344e526c3a15807887f3f'
ARMS = ('fresh', 'control', 'leaf')
CLAIM = 'Primitive literal leaf workload only, NOT broadgate or realproject. Descriptive timings, no significance claim.'


def require(ok, message):
    if not ok:
        raise ValueError(message)


def sha(data):
    return hashlib.sha256(data).hexdigest()


def canonical(value):
    return json.dumps(value, sort_keys=True, separators=(',', ':')).encode()


def order(repetition):
    offset = repetition % len(ARMS)
    return list(ARMS[offset:] + ARMS[:offset])


def fixture(items, revision):
    require(type(items) is int and 1 <= items <= 4096, 'items outside 1..4096')
    require(type(revision) is int and 0 <= revision <= 63, 'revision outside bounded64 lifetime')
    expected = {}
    files = {'fixture.rs': leaf.ROOT.encode()}
    # Each edit changes one module and preserves all definition identities.
    for mod in ('a', 'b'):
        lines = []
        for index in range(items):
            width = (8, 16, 32, 64)[index % 4]
            updates = (revision + (1 if mod == 'b' else 0)) // 2
            value = (index % 64 + 7 + (updates if index == 0 else 0))
            name = f'leaf_{mod}_{index:04d}'
            expected[name] = dict(width=width, value=value)
            lines.append(f'#[unsafe(no_mangle)]\npub fn {name}() -> u{width} {{ {value} }}\n')
        files[f'module_{mod}.rs'] = ''.join(lines).encode()
    return files, expected


def oracle(data, expected):
    text = data.decode('utf-8')
    exports = re.findall(r'^define\b[^\n]*@(leaf_[ab]_\d+)\(', text, re.MULTILINE)
    require(sorted(exports) == sorted(expected), 'missing, duplicate or unexpected exported primitive return')
    found = {}
    for name, value in expected.items():
        body = re.findall(r'^define\b[^\n]*@' + re.escape(name) + r'\([^\n]*\).*?^}', text, re.MULTILINE | re.DOTALL)
        require(len(body) == 1, 'missing or duplicate export ' + name)
        returns = re.findall(r'\bret i(\d+) (\d+)\b', body[0])
        require(returns == [(str(value['width']), str(value['value']))]
                and not re.search(r'\b(call|invoke|callbr)\b', body[0]), 'corrupt return ' + name)
        found[name] = value
    return found


def workload(items, edits, repetitions):
    require(type(edits) is int and 2 <= edits <= 63, 'edits outside 2..63')
    require(type(repetitions) is int and repetitions >= 1, 'repetitions must be positive')
    revisions = []
    for revision in range(edits + 1):
        files, expected = fixture(items, revision)
        revisions.append(dict(revision=revision, sources={n: sha(d) for n, d in files.items()}, expected=expected))
    return dict(fixture=FIXTURE, items=items, edits=edits, repetitions=repetitions,
                revisions=revisions, order=[order(r) for r in range(repetitions)],
                timing='invocation-wall-only', warmup='ordinary revision0, excluded',
                incremental='independent bank per repetition and arm', lifetime=64)


def host_identity():
    cpu = Path('/proc/cpuinfo').read_text()
    model = next((x.split(':', 1)[1].strip() for x in cpu.splitlines() if x.startswith('model name')), 'unknown')
    return dict(host=platform.node(), cpu=model, ncpu=os.cpu_count(), machine=platform.machine(),
                kernel=platform.release(), execution='local-native-owned', python=platform.python_version())


def check_identity(identity):
    require(Path(identity.get('path', '')).is_absolute(), 'requires absolute frozen compiler path')
    require(identity.get('sha256') == BINARY_SHA, 'requires exact frozen1527 compiler bytes')
    require([d.get('sha256') for d in identity.get('drivers', [])] == [DRIVER_SHA], 'requires exact frozen1527 driver SHA')


def execution_identity():
    return dict(argv=leaf.arguments(Path('.')), cwd='private-workspace',
                rootnames=list(fixture(1, 0)[0]),
                environment_sha256={arm: sha(canonical(typed.environment('leaf' if arm == 'leaf' else 'control')))
                                    for arm in ARMS})


def validate_execution(report):
    actual = report['actual_execution']
    require(actual == dict(argv=leaf.arguments(Path('.')), cwd=report['workspace']), 'actual semantic invocation mismatch')
    require(Path(actual['cwd']).is_absolute(), 'nonabsolute actual workspace')
    execution = report['execution']
    require(execution['argv'] == actual['argv'] and execution['cwd'] == 'private-workspace'
            and execution['rootnames'] == list(fixture(1, 0)[0]), 'semantic execution template mismatch')
    hashes = execution.get('environment_sha256', {})
    require(set(hashes) == set(ARMS) and all(re.fullmatch('[0-9a-f]{64}', v) for v in hashes.values()), 'environment identity missing')


def aggregate(rows):
    result = {}
    for arm in ARMS:
        measured = [r for r in rows if r['arm'] == arm and not r['warmup']]
        require(measured, 'missing measured arm ' + arm)
        require(all(r['record'].get('status') == 0 and r.get('validated') is True for r in measured),
                'errors or unvalidated observations cannot enter performance aggregates')
        values = [r['record']['elapsed_seconds'] for r in measured]
        require(all(type(v) in (int, float) and math.isfinite(v) and v > 0 for v in values), 'invalid invocation timing')
        result[arm] = dict(n=len(values), median_seconds=statistics.median(values),
                           min_seconds=min(values), max_seconds=max(values))
    return result


def capture(compiler, output, items=128, edits=8, repetitions=6, timeout=180):
    compiler = Path(compiler).resolve()
    require(compiler.is_file() and sha(compiler.read_bytes()) == BINARY_SHA,
            'requires exact frozen1527 compiler bytes')
    require(type(timeout) in (int, float) and math.isfinite(timeout) and timeout > 0, 'invalid timeout')
    spec = workload(items, edits, repetitions)
    output = Path(output).resolve()
    output.mkdir(mode=0o700, parents=True, exist_ok=False)
    workspace = output / 'workspace'
    workspace.mkdir(mode=0o700)
    report = dict(schema=SCHEMA, claim=CLAIM, verdict='incomplete', workload=spec,
                  workload_sha256=sha(canonical(spec)), host=host_identity(), rows=[], source_blobs={},
                  load_before=list(os.getloadavg()))
    argv = leaf.arguments(Path('.'))
    report['workspace'] = str(workspace)
    report['actual_execution'] = dict(argv=argv, cwd=str(workspace))
    report['execution'] = execution_identity()
    try:
        report['identity_before'] = shared.identity(Path(compiler).resolve())
        check_identity(report['identity_before'])
        for repetition in range(repetitions):
            for arm in order(repetition):
                evidence = output / f'rep-{repetition:03d}-{arm}'
                evidence.mkdir(mode=0o700)
                bank = evidence / 'incremental'
                active = workspace / 'incremental'
                worker = None
                previous = None
                try:
                    if arm != 'fresh':
                        worker = typed.TypedResident(compiler, workspace, arm, timeout)
                    for revision in range(edits + 1):
                        require(execution_identity() == report['execution'], 'execution environment changed')
                        files, expected = fixture(items, revision)
                        for name, data in files.items():
                            path = workspace / name
                            if not path.exists() or path.read_bytes() != data:
                                path.write_bytes(data)
                        for data in files.values():
                            report['source_blobs'].setdefault(sha(data), shared.encoded(data))
                        row = dict(repetition=repetition, arm=arm, revision=revision, warmup=revision == 0,
                                   sources={n: sha(d) for n, d in files.items()},
                                   validated=False)
                        report['rows'].append(row)
                        if bank.exists():
                            bank.rename(active)
                        for suffix in ('ll', 'mir'):
                            (workspace / ('fixture.' + suffix)).unlink(missing_ok=True)
                        try:
                            record = (shared.run([compiler, *argv], timeout, cwd=workspace, env=typed.environment('control'))
                                      if worker is None else worker.compile(argv))
                            row['record'] = record
                            require(record['status'] == 0, 'compiler failed, diagnostics retained in receipt')
                            if worker is not None:
                                typed.validate_pair(record, record, expected, worker.process.pid)
                                previous = leaf.validate_live(record, previous, arm, 'cold' if revision == 0 else 'cross')
                                if revision and arm == 'leaf':
                                    old = report['rows'][-2]['record']['live']['mir_leaf_hits']
                                    require(record['live']['mir_leaf_hits'] - old >= items, 'insufficient postgreen crossfile leaf hits')
                            row['artifacts'] = {}
                            for suffix in ('ll', 'mir'):
                                data = (workspace / ('fixture.' + suffix)).read_bytes()
                                require(bool(data), 'empty artifact')
                                saved = evidence / f'{revision:02d}.{suffix}'
                                saved.write_bytes(data)
                                row['artifacts'][suffix] = dict(path=str(saved), sha256=sha(data))
                                if suffix == 'll':
                                    row['oracle_sha256'] = sha(canonical(oracle(data, expected)))
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
        # Exact fresh oracle and observable diagnostics for every paired invocation.
        for repetition in range(repetitions):
            for revision in range(edits + 1):
                paired = [r for r in report['rows'] if r['repetition'] == repetition and r['revision'] == revision]
                require(len(paired) == 3, 'missing paired arm')
                require(all(shared.observable(r['record']) == shared.observable(paired[0]['record']) for r in paired), 'paired diagnostics differ')
                require(all(r['oracle_sha256'] == paired[0]['oracle_sha256'] for r in paired), 'fresh oracle differs')
        report['identity_after'] = shared.identity(Path(compiler).resolve())
        require(report['identity_after'] == report['identity_before'], 'compiler identity changed')
        require(host_identity() == report['host'], 'host changed')
        report['aggregates'] = aggregate(report['rows'])
        report['verdict'] = 'passed'
        return report
    except BaseException as exc:
        report.update(verdict='failed', error=f'{type(exc).__name__}: {exc}')
        raise
    finally:
        report['load_after'] = list(os.getloadavg())
        (output / 'receipt.json').write_text(json.dumps(report, indent=2) + '\n')


def validate(report):
    require(report.get('schema') == SCHEMA and report.get('verdict') == 'passed' and not report.get('error'), 'not a successful versioned leaf suite receipt')
    check_identity(report['identity_before'])
    require(report['identity_before'] == report['identity_after'], 'changed compiler identity')
    validate_execution(report)
    spec = report['workload']
    require(spec == workload(spec['items'], spec['edits'], spec['repetitions']), 'fixture or workload protocol mismatch')
    require(report['workload_sha256'] == sha(canonical(spec)), 'workload hash mismatch')
    rows = report['rows']
    schedule = [(rep, arm, rev) for rep in range(spec['repetitions']) for arm in order(rep) for rev in range(spec['edits'] + 1)]
    require([(r['repetition'], r['arm'], r['revision']) for r in rows] == schedule, 'counterbalance or revision schedule mismatch')
    previous = {}
    pids = {}
    for row in rows:
        rep, arm, rev = row['repetition'], row['arm'], row['revision']
        files, expected = fixture(spec['items'], rev)
        require(row['warmup'] == (rev == 0), 'warmup incorrectly aggregated')
        require(row['sources'] == {n: sha(d) for n, d in files.items()}, 'source identity mismatch')
        require(all(report['source_blobs'].get(sha(d)) == shared.encoded(d) for d in files.values()), 'source blob mismatch')
        require(row['oracle_sha256'] == sha(canonical(expected)), 'oracle mismatch')
        for suffix in ('ll', 'mir'):
            artifact = row['artifacts'][suffix]
            data = Path(artifact['path']).read_bytes()
            require(bool(data) and sha(data) == artifact['sha256'], 'artifact identity mismatch')
            if suffix == 'll':
                require(sha(canonical(oracle(data, expected))) == row['oracle_sha256'], 'artifact oracle mismatch')
        require(row['record']['status'] == 0 and row['validated'] is True, 'invalid successful sample')
        if arm == 'fresh':
            require(row['record'].get('command') == [report['identity_before']['path'], *report['execution']['argv']], 'fresh invocation identity mismatch')
        if arm != 'fresh':
            key = (rep, arm)
            record = row['record']
            pid = record.get('pid')
            # Transport itself checks protocol identity. Retain that full record.
            require(pid is not None and pid == pids.setdefault(key, pid), 'worker PID changed or missing')
            require(record.get('request') == dict(id=rev + 1, args=report['execution']['argv']), 'invocation identity mismatch')
            require(record.get('id') == rev + 1 and record['owned']['revisions'] == rev + 1 <= 64, 'bounded revision identity mismatch')
            before = previous.get(key)
            previous[key] = leaf.validate_live(record, before, arm, 'cold' if rev == 0 else 'cross')
            if rev and arm == 'leaf':
                require(record['live']['mir_leaf_hits'] - before['mir_leaf_hits'] >= spec['items'], 'insufficient crossfile leaf hits')
    for rep in range(spec['repetitions']):
        for rev in range(spec['edits'] + 1):
            paired = [r for r in rows if r['repetition'] == rep and r['revision'] == rev]
            require(all(shared.observable(r['record']) == shared.observable(paired[0]['record']) for r in paired), 'paired diagnostics differ')
    values = aggregate(rows)
    require(report['aggregates'] == values, 'stale performance aggregate')
    return values


def compare(a, b, floor=10.0):
    require(type(floor) in (int, float) and math.isfinite(floor) and floor >= 0, 'invalid floor')
    sa, sb = validate(a), validate(b)
    for key in ('identity_before', 'workload', 'workload_sha256', 'host', 'execution'):
        require(a[key] == b[key], 'comparison refuses different ' + key)
    print(CLAIM)
    for arm in ARMS:
        old, new = sa[arm]['median_seconds'], sb[arm]['median_seconds']
        delta = (new / old - 1) * 100
        print(f'{arm}: {old:.6f}s -> {new:.6f}s ({delta:+.2f}%), floor {floor:g}% exceeded: {abs(delta) > floor}')
    for label, values in (('before', sa), ('after', sb)):
        print(f'{label} leaf/control={values["leaf"]["median_seconds"] / values["control"]["median_seconds"]:.6f}, leaf/fresh={values["leaf"]["median_seconds"] / values["fresh"]["median_seconds"]:.6f}')


def upload_result(output):
    if os.environ.get('JUSTRUST_RECORDS', '').lower() in ('off', '0', 'false'):
        return
    path = Path(output) / 'receipt.json'
    if not path.is_file():
        return
    record_id = re.sub(r'[^A-Za-z0-9._-]', '-', Path(output).name)[:128].lstrip('.-_') or 'stable-leaf'
    try:
        result = subprocess.run([shutil.which('justrust') or 'justrust', 'records', 'put',
                                 'bench', record_id, str(path)],
                                capture_output=True, text=True, timeout=60)
        print((result.stderr or result.stdout).strip(), file=sys.stderr)
    except (OSError, subprocess.TimeoutExpired) as error:
        print(f'bench: original receipt retained, upload unavailable: {error}', file=sys.stderr)


def run(args):
    output_existed = Path(args.output).exists()
    try:
        report = capture(args.compiler, args.output, args.items, args.edits, args.repetitions, args.timeout)
        print(json.dumps(dict(receipt=str(args.output / 'receipt.json'), aggregates=report['aggregates'], claim=CLAIM), indent=2))
    finally:
        if not output_existed:
            upload_result(args.output)


def add_parser(sub):
    parser = sub.add_parser('stable-native-leaf', help=__doc__)
    parser.add_argument('--compiler', type=Path, required=True)
    parser.add_argument('--output', type=Path, required=True)
    parser.add_argument('--items', type=int, default=128)
    parser.add_argument('--edits', type=int, default=8)
    parser.add_argument('-n', '--repetitions', type=int, default=6)
    parser.add_argument('--timeout', type=float, default=180)
    parser.set_defaults(fn=run)
    return parser


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    add_parser(parser.add_subparsers(dest='command', required=True))
    args = parser.parse_args()
    require(math.isfinite(args.timeout) and args.timeout > 0, 'invalid timeout')
    args.fn(args)


if __name__ == '__main__':
    main()
