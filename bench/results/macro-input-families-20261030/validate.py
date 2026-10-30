#!/usr/bin/env python3
"""Offline validation of retained frozen-compiler failures, not new native testing."""
import gzip
import hashlib
import json
from pathlib import Path

ROOT = Path(__file__).resolve().parent
REPORT_SHA256 = 'a5aa0c43d1d13d7506c11a4005955e3e396167046cac645b49885442ff628dfd'
DRIVER_SHA256 = '3527393f3db7e09eeef270a83f51c5aeedb787c77f845ab76a76618f76ae137e'
LAUNCHER_SHA256 = '570faeb27338cea8625c4a4a7eaeef71fc75470508657ff489c3da96ce5f9144'
KINDS = {'constant', 'env', 'tracked_env', 'file', 'tracked_path'}
FAMILIES = {'derive', 'attribute', 'functionlike'}


def read(name):
    return json.loads((ROOT / name).read_text())


def check(condition, message):
    if not condition:
        raise ValueError(message)


def sha(data):
    return hashlib.sha256(data).hexdigest()


def validate():
    manifest = read('sha256.json')
    actual_files = {str(p.relative_to(ROOT)) for p in ROOT.rglob('*') if p.is_file()
                    and '__pycache__' not in p.parts and p.name not in {'sha256.json', 'README.md', 'validate.py'}}
    check(set(manifest) == actual_files, 'Manifest must cover every retained data artifact')
    for name, digest in manifest.items():
        check(sha((ROOT / name).read_bytes()) == digest, 'Artifact integrity: ' + name)
    check(sha((ROOT / 'report.json').read_bytes()) == REPORT_SHA256, 'Original report bytes changed')
    report, identity, probes = read('report.json'), read('identity.json'), read('probes.json')
    check(report['schema'] == 2 and report['status'] == 'fail', 'Expected failing schema-2 report')
    for key in ('schema', 'rustc', 'rustc_sha256', 'driver_libraries', 'version', 'prior_report', 'work_dir'):
        check(identity[key] == report[key], 'Identity/report mismatch: ' + key)
    check(identity['report_sha256'] == REPORT_SHA256, 'Report receipt')
    check(report['rustc_sha256'] == LAUNCHER_SHA256, 'Frozen launcher identity')
    check(len(report['driver_libraries']) == 1 and report['driver_libraries'][0]['sha256'] == DRIVER_SHA256,
          'Frozen driver identity')
    check(report['version']['status'] == 0, 'Version query succeeded')
    check('not cached-family safety proof' in report['prior_report']['scope'], 'Old root scope limitation')
    cases = {(c['family'], c['kind']): c for c in report['cases']}
    check(len(report['cases']) == 15 and set(cases) == {(f, k) for f in FAMILIES for k in KINDS}, 'Full family/input matrix')
    failures = 0
    for (family, kind), case in cases.items():
        unsafe = family != 'functionlike' and kind != 'constant'
        check(case['status'] == ('fail' if unsafe else 'pass'), 'Case verdict')
        check(case['api'] == 'modern', 'Supported tracked API probe')
        check(case['coverage'] == ('uncached-control' if family == 'functionlike' else 'cached-family'), 'Coverage classification')
        labels = ['warm', 'body-edit'] + ([] if kind == 'constant' else ['external-input-and-body-edit']) + ['macro-rebuild-and-body-edit']
        check([s['state']['label'] for s in case['steps']] == labels, 'Transition coverage')
        for step in case['steps']:
            state = step['state']
            bad = unsafe and state['label'] == 'external-input-and-body-edit'
            check(step['status'] == ('fail' if bad else 'pass'), 'Step verdict')
            check(step['reference_matches_expected'] and step['equal'] == (not bad), 'Independent reference and equality')
            check(set(step['records']) == {'off', 'on', 'fresh-off', 'fresh-on', 'shared-off', 'shared-on'}, 'Six configurations')
            for mode, record in step['records'].items():
                check(record['compile'] == {'status': 0, 'stdout': '', 'stderr': ''}, 'Compile observation')
                value = 'constant' if kind == 'constant' else state['input']
                if bad and mode == 'on':
                    value = 'alpha'
                    failures += 1
                expected = f"{value}:{state['revision']}|{state['body']}\n"
                check(record['runtime'] == {'status': 0, 'stdout': expected, 'stderr': ''}, 'Runtime observation')
        if family == 'functionlike':
            check(case['cache_evidence'] is None, 'Control must not claim cached-query proof')
    check(failures == 8, 'Eight stale observations required')
    check(all(o['result']['status'] == 0 for o in report['observations']), 'All macro fixtures compiled')
    check(len(probes) == 10 and {(p['family'], p['kind']) for p in probes} ==
          {(f, k) for f in {'derive', 'attribute'} for k in KINDS}, 'Ten named probes')
    for probe in probes:
        evidence = cases[probe['family'], probe['kind']]['cache_evidence']
        query = 'derive_macro_expansion' if probe['family'] == 'derive' else 'attr_macro_expansion'
        check(probe['hit'] and evidence['hit'] and probe['query'] == evidence['query'] == query, 'Exact query claim')
        analysis = read(probe['analysis'])
        check(analysis == evidence['analysis'], 'Retained named analysis equals full report')
        rows = [r for r in analysis['query_data'] if r['label'] == query]
        check(len(rows) == 1, 'Exactly one named expansion query')
        row = rows[0]
        check(row['incremental_load_time']['secs'] > 0 or row['incremental_load_time']['nanos'] > 0, 'Actual incremental load evidence')
        check(row['invocation_count'] == 0, 'Loaded without provider invocation')
        check([p['phase'] for p in probe['profiles']] == ['warm', 'body-edit'], 'Two-process probe receipt')
        for receipt, original in zip(probe['profiles'], evidence['records']):
            check(receipt['compile'] == original['compile'] == {'status': 0, 'stdout': '', 'stderr': ''}, 'Probe compilation succeeded')
            raw = gzip.decompress((ROOT / receipt['artifact']).read_bytes())
            check(sha(raw) == receipt['uncompressed_sha256'], 'Uncompressed profile integrity')
    return {'verdict': 'evidence-valid', 'stale_cases': failures, 'named_disk_load_probes': len(probes),
            'constant_cases_passed': 2, 'uncached_controls_passed': 5,
            'scope': 'Frozen compiler failure evidence only. Does not test safe-default fix e5bc943e9 or make a speed claim.'}


if __name__ == '__main__':
    print(json.dumps(validate(), indent=2))
