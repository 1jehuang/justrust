#!/usr/bin/env python3
"""Portable stdlib-only replay of selected historical v65 outcomes, not a native run."""
import argparse
import hashlib
import json
import math
import re
from pathlib import Path

PHASES = ('cold-prepare', 'repeated-noop-1', 'manifest-feature-add', 'type-error',
          'type-recovery', 'assert-failure', 'assert-recovery')
CASES = ('off', 'on', 'on-repeat', 'off-repeat')
NAMES = ('summary::tests::parses_libtest_results', 'summary::tests::shares_overlapping_time',
         'summary::tests::splits_passes_without_double_counting',
         'summary::tests::test_failures_are_not_compile_errors')
FIXTURES = ('ready_notify_ready_before_wait_and_terminal_errors',
            'ready_notify_wrong_key_generation_and_loading_timeout',
            'ready_notify_during_wait_uses_the_predicate_mutex',
            'ready_notify_generation_transition_during_wait')
MARKER = 'ra-jit ready-notify: observed loading transition'
ENV = {'BLAZE_BACKEND': 'cranelift', 'BLAZE_OPT': '0',
       'CARGO_PROFILE_DEV_DEBUG': '0', 'CARGO_PROFILE_DEV_INCREMENTAL': 'true',
       'CARGO_PROFILE_DEV_OPT_LEVEL': '0', 'CARGO_PROFILE_TEST_DEBUG': '0',
       'CARGO_PROFILE_TEST_INCREMENTAL': 'true', 'CARGO_PROFILE_TEST_OPT_LEVEL': '0'}
CONTROLS = {'RA_JIT_DEMAND_STD': '1', 'RA_JIT_CACHE_MISS_STATS': '0',
            'RA_JIT_STATS': '1', 'RA_JIT_WHOLE_RELOAD': '0'}
BINARY = '5a0c64792bf11ef68e4ed38e218f2522f6527bd069f70480d543073a11106b27'
ORIGINAL = 'd488bffe1498847e8e234e6513474c26b38b19939b94915feacaa60decbe0b5c'
CANDIDATE = 'cb243a2d02abb2509db6203800efa537d048649ba784911f59c5679485026df7'
PATCH = '5f7494e7656402ff7b7ed53ab52ef0660f0628f5278492318e461c3395ca2209'
# Filled after the fixed allowlist export. This commitment binds historical fields
# beyond the semantic oracle and cannot be repaired by merely rehashing the manifest.
OBSERVATIONS = 'b62945f50abd468510553b3e28a8260f422b8d5ea07b92f52323e7994f337901'


def sha(data):
    return hashlib.sha256(data).hexdigest()


def require(condition, message):
    if not condition:
        raise ValueError(message)


def semantic(data):
    require(data['schema'] == 1 and data['decision'] == 'rejected', 'rejection/schema')
    require(data['promotion'] is False and data['speed_claim'] is False
            and data['causal_regression_claim'] is False and data['noise'] == 'unknown', 'nonclaims')
    require(set(data['cases']) == set(CASES), 'case coverage')
    nonces = set()
    for name in CASES:
        case = data['cases'][name]
        enabled = name in ('on', 'on-repeat')
        require(case['status'] == 'passed' and case['unchanged'] is True, 'case status')
        require(case['controls'] == dict(CONTROLS, RA_JIT_READY_NOTIFY='1' if enabled else '0'), 'exact controls')
        require(case['source_before'] == case['source_after'], 'source unchanged')
        require(case['toolchain_before'] == case['toolchain_after'], 'toolchain unchanged')
        require(case['tool_before'] == case['tool_after'] == BINARY, 'binary identity')
        require([r['phase'] for r in case['rows']] == list(PHASES), 'seven ordered phases')
        for row in case['rows']:
            phase = row['phase']
            nonce = row['nonce']
            require(re.fullmatch('[0-9a-f]{32}', nonce) and nonce not in nonces, 'fresh nonce')
            nonces.add(nonce)
            require(row['accepted'] is True and row['input_unchanged'] is True
                    and row['failures'] == [] and row['raw_error'] is None
                    and row['error'] is None and row['timed_out'] is False, 'row acceptance')
            require(row['safe_environment'] == ENV and row['omitted_environment_entries'] == 12,
                    'explicit safe environment')
            require(row['noise_contamination'] == 'unknown', 'noise unknown')
            require(math.isfinite(row['elapsed_seconds']) and row['elapsed_seconds'] > 0, 'elapsed positive')
            require(row['marker_lines'] == ([MARKER] if enabled else []), 'ON positive / OFF zero marker')
            text = row['stdout']
            diagnostics = '\n'.join(row['diagnostic_lines'])
            runtime = re.findall(r'JR_MATRIX_RUNTIME_[^\s]*', text + '\n' + diagnostics)
            if phase == 'type-error':
                require(row['status'] == 101 and row['parsed'] is None and not runtime, 'type-error status/no runtime')
                require('error[E0308]:' in diagnostics and 'JR_MATRIX_TYPE_' + nonce in diagnostics,
                        'fresh type diagnostic')
                require('test result:' not in text, 'type-error no completed tests')
                require(len(row['diagnostic_lines']) == 2, 'type diagnostic line coverage')
            else:
                failed = phase == 'assert-failure'
                expected_tests = [[n, 'FAILED' if failed and n == NAMES[1] else 'ok'] for n in NAMES]
                expected = {'tests': expected_tests, 'passed': 3 if failed else 4,
                            'failed': 1 if failed else 0, 'ignored': 0, 'filtered': 144}
                require(row['parsed'] == expected, 'exact named results')
                require(row['status'] == (101 if failed else 0), 'exact exit status')
                require(runtime == ['JR_MATRIX_RUNTIME_' + nonce], 'exact invocation witness')
                # A runtime challenge can occur between the named test and its status.
                clean = text.replace('JR_MATRIX_RUNTIME_' + nonce + '\n', '')
                actual = re.findall(r'^test (\S+) \.\.\. (ok|FAILED)$', clean, re.M)
                require(actual == [tuple(x) for x in expected_tests], 'raw named results')
                summary = ('test result: FAILED. 3 passed; 1 failed;' if failed else
                           'test result: ok. 4 passed; 0 failed;')
                require(len(re.findall(r'^test result:', text, re.M)) == 1
                        and summary + ' 0 ignored; 0 measured; 144 filtered out;' in text,
                        'raw full libtest counts')
                require(row['diagnostic_lines'] == ([
                    'assertion `left == right` failed: JR_MATRIX_FAILURE_' + nonce] if failed else []),
                    'fresh assertion / no unexpected diagnostics')
    baseline = data['baseline']
    require(baseline['status'] == 'passed' and baseline['unchanged'] is True
            and baseline['binary_sha256'] == '005c6198fddf1a7a77b6fda6d0be27601a0a48a7f2279deac460ebf5d4898edd',
            'distinct accepted baseline')
    require([r['phase'] for r in baseline['rows']] == list(PHASES)
            and all(r['accepted'] is True and r['status'] ==
                    (101 if r['phase'] in ('type-error', 'assert-failure') else 0)
                    for r in baseline['rows']), 'baseline seven outcomes')
    validation = data['root_validation']
    require(validation['binary_sha256'] == BINARY and validation['activation_passed'] is True
            and validation['noise'] == 'unknown' and validation['general_speed_claim'] is False,
            'corrected root analysis')
    for name in CASES:
        expected = validation['cases'][name]
        require(expected['strict_rows'] == 7, 'root strict rows')
        require(expected['cumulative_marker_snapshots'] == [int(name in ('on', 'on-repeat'))] * 7,
                'root marker snapshots')
        for row in data['cases'][name]['rows']:
            require(expected['seconds'][row['phase']] == row['elapsed_seconds'], 'raw/root elapsed agreement')
    require(len(validation['pairs']) == 2, 'both descriptive pairs')
    for pair, (off, on) in zip(validation['pairs'], (('off', 'on'), ('off-repeat', 'on-repeat'))):
        require(pair['off'] == off and pair['on'] == on, 'pair identity')
        for phase in PHASES:
            computed = (validation['cases'][on]['seconds'][phase] /
                        validation['cases'][off]['seconds'][phase] - 1) * 100
            require(abs(computed - pair['percent_change'][phase]) < 1e-10, 'descriptive pair arithmetic')
    require([(a['version'], a['outcome']) for a in data['analysis_attempts']] ==
            [('v65', 'failed'), ('v65b', 'failed'), ('v65c', 'passed')], 'failed analyses retained')
    require(all(a['stderr_retained'] is False and a['historical_rerun'] is False
                for a in data['analysis_attempts']), 'no invented stderr/history rerun')
    native = data['native']
    for gate in ('check', 'compile', 'codegen-native', 'full-native'):
        require(native[gate]['exit'] == 0, 'native gate exit')
    for gate, count in (('codegen-native', 53), ('full-native', 368)):
        require(any(f'{count} tests passed' in line for line in native[gate]['summaries']), 'native count')
    require(len(native['fixtures']) == 4, 'four fixtures')
    for fixture in FIXTURES:
        require(sum(bool(re.fullmatch(r'test cargo::daemon::tests::' + fixture + r' \.\.\. ok <[0-9.]+s>', line))
                    for line in native['fixtures']) == 1, 'actual native fixture result')
    require(native['source_check_count'] == 3757 and native['source_check_ok'] is True
            and native['lock_check_ok'] is True, 'source/rootlock preservation')
    require(any('Ran 733 tests' in line for line in native['benchmark_regression_lines'])
            and 'OK' in native['benchmark_regression_lines'], 'benchmark tests 733')
    for name, count in (('baseline', 1), ('candidate', 4)):
        cleanup = data['cleanup'][name]
        require(cleanup['stores_deleted'] is False and len(cleanup['rows']) == count, 'cleanup stores/count')
        require(all(r['post_measurement_only'] is True and r['status'] == 0
                    and r['error'] is None and r['timed_out'] is False for r in cleanup['rows']),
                'post-validation stop outcomes')
    require(data['restoration']['daemon_sha256'] == ORIGINAL
            and data['restoration']['cmp_original'] is True
            and data['restoration']['head_diff_empty'] is True, 'restoration')
    require(data['privacy']['environment_allowlist'] == ENV
            and set(data['privacy']['control_allowlist']) == set(CONTROLS) | {'RA_JIT_READY_NOTIFY'},
            'privacy allowlists')
    require(all(re.fullmatch('[0-9a-f]{64}', r['sha256']) and r['bytes'] >= 0
                for r in data['private_originals'].values()), 'private provenance')


def replay_patch(original, patch):
    lines = patch.splitlines(keepends=True)
    chunks = []
    at = 0
    while at < len(lines):
        if lines[at].startswith('--- '):
            start = lines[at].strip()[4:]
            target = lines[at + 1].strip()[4:]
            at += 2
            old = original.splitlines(keepends=True) if start != '/dev/null' else []
            cursor = 0
            out = []
            while at < len(lines) and not lines[at].startswith('--- '):
                match = re.match(r'@@ -(\d+)(?:,\d+)? \+(\d+)(?:,\d+)? @@', lines[at])
                require(match is not None, 'patch hunk header')
                position = max(int(match.group(1)) - 1, 0)
                out.extend(old[cursor:position])
                cursor = position
                at += 1
                while at < len(lines) and lines[at][0] in ' +-':
                    if lines[at].startswith('--- '):
                        break
                    kind, line = lines[at][0], lines[at][1:]
                    if kind in ' -':
                        require(cursor < len(old) and old[cursor] == line, 'patch exact context')
                        cursor += 1
                    if kind in ' +':
                        out.append(line)
                    at += 1
            out.extend(old[cursor:])
            chunks.append((target, ''.join(out)))
        else:
            raise ValueError('patch file header')
    return chunks


def validate_package(root, manifest=True):
    raw = (root / 'observations.json').read_bytes()
    require(sha(raw) == OBSERVATIONS, 'fixed historical observation commitment')
    semantic(json.loads(raw))
    frozen = root / 'frozen'
    require(sha((frozen / 'daemon.rs.original').read_bytes()) == ORIGINAL, 'original source hash')
    require(sha((frozen / 'daemon.rs.candidate').read_bytes()) == CANDIDATE, 'candidate source hash')
    require(sha((frozen / 'candidate.patch').read_bytes()) == PATCH, 'patch hash')
    chunks = replay_patch((frozen / 'daemon.rs.original').read_text(), (frozen / 'candidate.patch').read_text())
    require(len(chunks) == 2 and chunks[0][1] == (frozen / 'daemon.rs.candidate').read_text()
            and chunks[1][1] == (frozen / 'resident-ready-notify.md.candidate').read_text(), 'full patch replay')
    for version, expected in (
        ('v65', '11854563a465533b86c567200321ef36b78e0b1a5dfb7f5e4da4351e9ca14e53'),
        ('v65b', '142eccbc4d696f043124f9a22db39ff86c681aa26b366192a26e379b7797ed34'),
        ('v65c', '1c5a457dd1ff667d6f9ca0f684ccedbaa61275cdc5b59338e22fa18c4ad54d94'),
    ):
        require(sha((root / f'analyses/validate-{version}.py.txt').read_bytes()) == expected,
                'fixed distinct historical analysis derivative')
    require((root / 'analyses/v65-output.json').read_bytes() == b''
            and (root / 'analyses/v65b-output.json').read_bytes() == b'', 'failed empty outputs')
    require(json.loads((root / 'analyses/v65c-output.json').read_text()) == json.loads(raw)['root_validation'],
            'corrected exact root output')
    if manifest:
        inventory = json.loads((root / 'SHA256.json').read_text())
        actual = {str(p.relative_to(root)): sha(p.read_bytes()) for p in root.rglob('*')
                  if p.is_file() and p.name != 'SHA256.json' and '__pycache__' not in p.parts}
        require(actual == inventory, 'complete package manifest')
    return {'decision': 'rejected', 'strict_rows': 28, 'native': [53, 368],
            'fixtures': 4, 'ON_wait_observed': True, 'OFF_marker_zero': True,
            'speed_claim': False, 'native_jobs_executed': 0}


if __name__ == '__main__':
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--manifest', action='store_true', help='verify complete package inventory too')
    args = parser.parse_args()
    print(json.dumps(validate_package(Path(__file__).resolve().parent, args.manifest), sort_keys=True))
