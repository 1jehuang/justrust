#!/usr/bin/env python3
"""Offline-only replay of one cold diagnostic. No native tools, Git or network."""
import hashlib
import json
from pathlib import Path
import re

EXCLUDED = {'manifest.json', 'audit.json', 'offline-tests.log'}
TOOLS = {'reference': 'fa622aeca09858c050264f701fbbebd1a14aa07d2b02c1eb4de511b684ba0263',
         'engine': 'c2606d25eb820ee515f5c5ae9d42f803916a4e3af974adb872c2fc4ba31c4e8b'}
MAIN = 'a1b78692eeb5fa95b6bb43c40e2c063f82740ae5c2e07f1293b715b3b4b9ab48'
SOURCE = '79c53ff6e87a22c84c9bc7bac6917c69b3f8d0fbc29b107f734cc00cc39a64df'
TC = '8bdbccdd1ca3fd7f2935cfb5667047eb4c46824928823e96311d8eb5ec0a6371'
NAMES = sorted('summary::tests::' + n for n in ('parses_libtest_results',
    'shares_overlapping_time', 'splits_passes_without_double_counting',
    'test_failures_are_not_compile_errors'))


def need(value, message):
    if not value:
        raise ValueError(message)


def sha(raw):
    return hashlib.sha256(raw).hexdigest()


def identity(value):
    return sha(json.dumps(value, sort_keys=True, separators=(',', ':')).encode())


class Evidence:
    def __init__(self, root=None, overrides=None):
        self.root = Path(root or Path(__file__).resolve().parent)
        self.overrides = overrides or {}

    def read(self, name):
        need(not Path(name).is_absolute() and '..' not in Path(name).parts, 'unsafe relative path')
        path = self.root / name
        need(not any(p.is_symlink() for p in (path, *path.parents)), 'symlink evidence')
        return self.overrides.get(name, path.read_bytes())

    def text(self, name):
        return self.read(name).decode()

    def json(self, name):
        return json.loads(self.read(name))


def audit(e=None):
    e = e or Evidence()
    inventory = e.json('manifest.json')
    actual = {str(p.relative_to(e.root)) for p in e.root.rglob('*')
              if p.is_file() and p.name not in EXCLUDED and '__pycache__' not in p.parts}
    need(set(inventory) == actual and len(inventory) >= 25, 'inventory coverage')
    for name, digest in inventory.items():
        need(sha(e.read(name)) == digest, 'checksum ' + name)
    s = e.json('raw/state.json')
    need(s['status'] == 'passed' and s['unchanged'] is True, 'actual run completion')
    need(s['scope'] == 'Actual cold native statistics, diagnostic only, not a performance comparison.', 'scope')
    need(len(s['source_before']) == 63 and identity(s['source_before']) == SOURCE,
         'immutable source identity')
    need(identity(s['toolchain_before']) == TC, 'immutable toolchain identity')
    need(s['tools_before'] == TOOLS, 'immutable binary identity')
    for field in ('source', 'toolchain', 'tools'):
        need(s[field + '_before'] == s[field + '_after'], field + ' changed during native diagnostic')
    need(e.json('provenance/source63-manifest.json') == s['source_before'], 'source63 manifest')
    need(sha(e.read('provenance/main.rs.frozen')) == MAIN, 'frozen source identity')
    full = e.text('provenance/all-source-before.sha256')
    need(len(full.splitlines()) == 3757 and sha(full.encode()) ==
         '88ee16223d467d71c3ded7ad450be1170a8fcccabdcc7f95804b461f928318e2', 'full-source inventory')
    need(MAIN + '  crates/ra-jit/src/main.rs\n' in full, 'frozen source inclusion')
    capture = e.json('provenance/capture.json')
    need(capture['source_commit'] == '91220ff43' and capture['frozen_main_sha256'] == MAIN
         and capture['native_jobs_executed_by_capture'] is False, 'capture provenance')
    need(e.text('provenance/engine-identity.sha256').split()[0] == TOOLS['engine'], 'engine build identity')
    need('build resident-v1-97212fd12b5cdb54922fcec68f797b98348695687cc91b26d75ad852ac77017f)'
         in e.text('provenance/engine-version.txt'), 'engine build key')
    for name in ('check.exit', 'compile.exit', 'orchestration-exit'):
        need(e.text('provenance/' + name).strip() == '0', 'native build receipt')
    need([r['stack'] for r in s['rows']] == ['reference', 'engine'], 'exactly two actual cold calls')
    for row in s['rows']:
        stack = row['stack']
        need(row['status'] == 0 and row['error'] is None and row['timed_out'] is False
             and row['accepted'] is True, 'native call status')
        suffix = ['test', '--lib', '--', 'summary::tests::', '--test-threads=1', '--nocapture']
        need(row['command'][1:] == (['cargo'] if stack == 'reference' else []) + suffix,
             'actual exact test command')
        need(Path(row['command'][0]).name == ('ra-jit' if stack == 'reference' else 'jr-engine'), 'direct native command')
        tests = re.findall(r'^test (\S+) \.\.\. (\S+)$', e.text('raw/' + stack + '.stdout'), re.M)
        need(tests == [(n, 'ok') for n in NAMES], 'actual four named tests')
        need('test result: ok. 4 passed; 0 failed; 0 ignored; 0 measured; 144 filtered out;'
             in e.text('raw/' + stack + '.stdout'), 'actual test result count')
        need(row['noise']['contamination'] == 'unknown' and
             'does not establish a quiet machine' in row['noise']['reason'], 'noise limitations')
        for p in ('DEV', 'TEST'):
            for key, value in (('DEBUG', '0'), ('OPT_LEVEL', '0'), ('INCREMENTAL', 'true')):
                need(row['environment']['CARGO_PROFILE_' + p + '_' + key] == value, 'same profile')
        need(row['environment']['BLAZE_BACKEND'] == 'cranelift'
             and row['environment']['BLAZE_OPT'] == '0', 'backend settings')
        namespace = s['private_namespaces'][stack]
        need(row['environment']['BLAZE_HOME'].startswith(namespace['root'] + '/'), 'private namespace')
        if stack == 'engine':
            need(row['environment']['BLAZE_HOME'] == namespace['home'] + '/resident-v1/runtime', 'native state authority')
        log = e.text('raw/' + stack + '-worker-0.log')
        need(len(re.findall(r'^standard library: [0-9]+ crates, compiled', log, re.M)) == 1
             and len(re.findall(r'^cold:', log, re.M)) == 1, 'one cold worker request')
    log = e.text('raw/engine-worker-0.log')
    need('standard library: 17 crates, compiled 25924 of 25960 fns (+16200 instances) in 9.1s' in log,
         'eager standard-library counts/timing')
    need('cold: compiled 3990 of 3990 fns (+40606 instances) in 5.1s' in log, 'project cold counts/timing')
    need('check: 31 files in 1.4s' in log, 'source checking count/timing')
    need('stats: 43723 of 44574 functions compiled, 851 reused' in log, 'project function totals')
    phases = re.findall(r'^steps: infer ([^,]+), mir ([^,]+), monomorphize ([^,]+), translate ([^,]+),', log, re.M)
    need(len(phases) == 2 and phases[0] == ('15.6s', '8.7s', '1.1s', '80.4s'), 'overlapping CPU phase receipt')
    timing = re.search(r'^blaze time: ([0-9.]+) ms: daemon ([0-9.]+).*client planning ([0-9.]+)',
                       e.text('raw/engine.stderr'), re.M)
    need(timing is not None and tuple(map(float, timing.groups())) == (26338.5, 22983.0, 3337.5),
         'actual client timing receipt')
    return {'status': 'passed', 'immutable_files': len(inventory), 'source_files': 63,
        'full_source_inventory_entries': 3757, 'actual_cold_calls': 2, 'actual_tests_per_call': 4,
        'candidate_source_commit': '91220ff43', 'total_ms': 26338.5, 'daemon_ms': 22983.0,
        'planning_ms': 3337.5, 'eager_std': {'crates': 17, 'roots_compiled': 25924,
        'roots_total': 25960, 'instances': 16200, 'seconds': 9.1},
        'project': {'roots_compiled': 3990, 'instances': 40606, 'seconds': 5.1},
        'source_check': {'files': 31, 'seconds': 1.4}, 'cpu_phases_additive': False,
        'comparison': False, 'savings_claimed': False, 'quiet_machine_proven': False,
        'v31_validation_claimed': False, 'scope': 'one profiled cold diagnostic only'}


if __name__ == '__main__':
    print(json.dumps(audit(), indent=2))
