#!/usr/bin/env python3
"""Offline audit of committed October13 hosted evidence, not runtime acceptance."""
import hashlib
import json
from pathlib import Path
import re
import subprocess
from datetime import datetime, timezone

ROOT = str(Path(__file__).resolve().parent.parent)
ID = re.compile(r'20261013-\d{9}-\d+(?:-[A-Za-z0-9_]+)*')


def git(*args):
    return subprocess.check_output(['git', '-C', ROOT, *args])


def committed(path):
    return git('show', 'HEAD:' + path)


def refs(value):
    if isinstance(value, dict):
        for key, v in value.items():
            yield from refs(key)
            yield from refs(v)
    elif isinstance(value, list):
        for v in value:
            yield from refs(v)
    elif isinstance(value, str):
        yield from ID.findall(value)


def observations(value):
    """Handle suite samples, round-robin stacks and flat acceptance rows."""
    if isinstance(value, dict):
        if isinstance(value.get('id'), str) and ID.fullmatch(value['id']):
            yield value
        for v in value.values():
            yield from observations(v)
    elif isinstance(value, list):
        for v in value:
            yield from observations(v)


def differences(path, sample, row):
    mismatches = []
    tests, units = row.get('tests', {}), row.get('units', {})
    pairs = [('tests_passed', tests.get('passed')), ('tests_failed', tests.get('failed')),
             ('units', units.get('compiled')), ('compiled', units.get('compiled')),
             ('fresh', units.get('fresh')), ('exit', row.get('exit'))]
    for field, actual in pairs:
        if field in sample and not isinstance(sample[field], dict) and actual is not None and sample[field] != actual:
            mismatches.append({'file': path, 'id': row['id'], 'field': field, 'sample': sample[field], 'export': actual})
    if isinstance(sample.get('tests'), dict):
        for key in ['passed', 'failed', 'ignored', 'filtered_out']:
            if sample['tests'].get(key) is not None and tests.get(key) is not None and sample['tests'][key] != tests[key]:
                mismatches.append({'file': path, 'id': row['id'], 'field': 'tests.' + key})
    remote = sample.get('remote') or {}
    if remote.get('machine') is not None and remote['machine'] != (row.get('remote') or {}).get('machine'):
        mismatches.append({'file': path, 'id': row['id'], 'field': 'remote.machine'})
    expected_compiler = sample.get('toolchain') or sample.get('compiler') or remote.get('toolchain')
    if expected_compiler and expected_compiler != (row.get('toolchain') or 'system'):
        mismatches.append({'file': path, 'id': row['id'], 'field': 'toolchain', 'sample': expected_compiler, 'export': row.get('toolchain') or 'system'})
    if 'args' in sample and sample['args'] != row.get('args'):
        mismatches.append({'file': path, 'id': row['id'], 'field': 'args'})
    return mismatches


def audit():
    paths = git('ls-files', 'docs/hosted*20261013.md', 'docs/cumulative*20261013.md',
                'bench/results/*20261013*.json', 'bench/results/versus-20261013*.json').decode().splitlines()
    records = {}
    for line in committed('data/runs/xps14/2026-10.jsonl').splitlines():
        row = json.loads(line)
        if row.get('id'):
            records[row['id']] = row
    out = {'checked_at': datetime.now(timezone.utc).isoformat(),
           'source_head': git('rev-parse', 'HEAD').decode().strip(),
           'scope': 'Committed October13 hosted/cumulative documents and benchmark outputs. Offline correspondence only, not fresh runtime acceptance.',
           'files': [], 'missing_references': [], 'sample_mismatches': [], 'acceptance': []}
    all_ids = set()
    for path in paths:
        raw = committed(path)
        obj = json.loads(raw) if path.endswith('.json') else raw.decode()
        ids = set(refs(obj))
        all_ids.update(ids)
        out['missing_references'].extend({'file': path, 'id': x} for x in sorted(ids - records.keys()))
        count = 0
        for sample in observations(obj):
            rid = sample['id']
            if rid in records:
                count += 1
                out['sample_mismatches'].extend(differences(path, sample, records[rid]))
        out['files'].append({'path': path, 'sha256': hashlib.sha256(raw).hexdigest(),
                             'referenced_runs': len(ids), 'mapped_observations_checked': count})
    for rid in ['20261013-160058612-3890266', '20261013-160059348-3890346', '20261013-160102069-3890192']:
        row = records[rid]
        assert row['exit'] == 0 and row['remote']['machine'] == 'hosted'
        assert row['units']['compiled'] == 0 and row['units']['fresh'] == 56
        assert row['agent_session'] == 'session_kikazaru_1791880615614_96f5e034cb0d5d07'
        out['acceptance'].append({'id': rid, 'args': row['args'], 'exit': row['exit'],
                                  'toolchain': row.get('toolchain'),
                                  'tests': {k: row.get('tests', {}).get(k) for k in ['passed', 'failed', 'ignored']},
                                  'compiled': row['units']['compiled'], 'fresh': row['units']['fresh'], 'where': row['remote']['machine']})
    assert out['acceptance'][1]['tests'] == {'passed': 360, 'failed': 0, 'ignored': 2}
    assert out['acceptance'][2]['args'] == ['clippy', '--all-targets', '--', '-D', 'warnings']
    out['unique_referenced_runs'] = len(all_ids)
    out['mapped_observations_checked'] = sum(x['mapped_observations_checked'] for x in out['files'])
    out['pass'] = not out['missing_references'] and not out['sample_mismatches']
    return out


if __name__ == '__main__':
    result = audit()
    print(json.dumps(result, indent=2))
    raise SystemExit(0 if result['pass'] else 1)
