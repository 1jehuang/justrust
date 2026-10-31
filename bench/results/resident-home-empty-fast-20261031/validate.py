#!/usr/bin/env python3
"""Stdlib-only semantic replay of historical home-empty-fast evidence. Never launches native tools."""
import argparse
import ast
import hashlib
import json
import math
import re
from pathlib import Path
from types import SimpleNamespace

CASES = ('off', 'on', 'on-repeat', 'off-repeat')
PHASES = ('cold-prepare', 'repeated-noop-1', 'manifest-feature-add', 'type-error',
          'type-recovery', 'assert-failure', 'assert-recovery')
NAMES = ('summary::tests::parses_libtest_results', 'summary::tests::shares_overlapping_time',
         'summary::tests::splits_passes_without_double_counting',
         'summary::tests::test_failures_are_not_compile_errors')
FIXTURES = ('v68_real_database_same_repeated_cross_crate_and_cfg',
            'v68_real_database_signature_and_dependency_revisions',
            'v68_real_database_generic_closure_shim_and_caller_bounds_fallback')
GUARD = 'v68_real_database_no_home_guard'
ENV = {'BLAZE_BACKEND': 'cranelift', 'BLAZE_OPT': '0',
       'CARGO_PROFILE_DEV_DEBUG': '0', 'CARGO_PROFILE_DEV_INCREMENTAL': 'true',
       'CARGO_PROFILE_DEV_OPT_LEVEL': '0', 'CARGO_PROFILE_TEST_DEBUG': '0',
       'CARGO_PROFILE_TEST_INCREMENTAL': 'true', 'CARGO_PROFILE_TEST_OPT_LEVEL': '0'}
CONTROLS = {'RA_JIT_DEMAND_STD': '1', 'RA_JIT_CACHE_MISS_STATS': '0',
            'RA_JIT_STATS': '1', 'RA_JIT_WHOLE_RELOAD': '0'}
MARKER = 'ra-jit: home-empty-fast-v68 branch'
BINARY = '01d1f62e01c50819e4d9feb9ec86a204ca8c31de2a4e5aebaba8c27c0abdfa0e'
ORIGINAL = '17aa54c87bcf5ec64b3404577352d524c7d454e92bcb89ad98dba225ba6438a7'
CANDIDATE = 'e9279a0ad4af94a65fdb65108a2a503a43a1528369a348a5fd01acc1827b6468'
MANIFEST_ORIGINAL = 'f46dec5b2cf06f30456f2379967d4bb1747c9e690ae35a1861d3775c95eab381'
MANIFEST_CANDIDATE = '70951a5a4d68e479a7a1ab7a452459fe510b211db3705cc5201e91f5d2771159'
OBSERVATIONS = '5a85559487dd949a639e3055199c6decb30cdd873dc03311d2a6295ff2c9f7e1'


def sha(raw):
    return hashlib.sha256(raw).hexdigest()


def canonical(value):
    return json.dumps(value, sort_keys=True, separators=(',', ':')).encode()


def require(condition, message):
    if not condition:
        raise ValueError(message)


def original_oracles(root):
    """Extract only hash-bound pure functions, never import execution modules."""
    sources = {
        'resident_state_matrix': ('138aa2fb7e1791de92d43c8855c6151abd188e5f57fdb4d28c22e326c54be738', ('contract', 'oracle')),
        'resident_workload_compare': ('e47ef667315c0044e2b0072eb43a9db68860f32b50757bdd927b49522de1e92d', ('valid',)),
        'blaze_runtime_compare': ('a7e2af863e2b1ea2878ae5c66a0f01aab1ad7fb9efe2e1027254a1269eefb568', ('test_results',)),
        'blaze_edit_compare': ('9c451fdb939d3a8c024ddf7e2b8c2fd3a8badc7c164a429415a7c636eb8c41a8', ('succeeded',)),
    }
    namespaces = {}
    for name, (digest, wanted) in sources.items():
        raw = (root / 'oracles' / (name + '.py.txt')).read_bytes()
        require(sha(raw) == digest, 'fixed original oracle source')
        tree = ast.parse(raw)
        functions = [node for node in tree.body if isinstance(node, ast.FunctionDef) and node.name in wanted]
        require({node.name for node in functions} == set(wanted), 'exact original pure function closure')
        scope = {'re': re}
        exec(compile(ast.Module(body=functions, type_ignores=[]), name, 'exec'), scope)
        namespaces[name] = scope
    matrix = namespaces['resident_state_matrix']
    runtime = namespaces['blaze_runtime_compare']
    matrix['shared'] = SimpleNamespace(NAMES=list(NAMES), EDIT_TEST=NAMES[1])
    matrix['workload'] = SimpleNamespace(valid=namespaces['resident_workload_compare']['valid'])
    matrix['runtime'] = SimpleNamespace(base=SimpleNamespace(succeeded=namespaces['blaze_edit_compare']['succeeded']))
    return runtime['test_results'], matrix['contract'], matrix['oracle']


def replay_original_oracles(root, data):
    parse, contract, oracle = original_oracles(root)
    for case in CASES:
        expected = None
        for row in data['cases'][case]['rows']:
            text = row['stdout'] + '\n' + '\n'.join(row['diagnostic_lines'])
            parsed = parse(text)
            # JSON serializes tuples as lists. Reparse actual raw selected text into the
            # original parser's tuples, rather than weakening workload.valid equality.
            require(canonical(parsed) == canonical(row['parsed']), 'original parser agrees with serialized receipt')
            if row['phase'] == 'cold-prepare':
                expected = contract(parsed, row['phase'])
                require(expected is not None, 'original cold contract')
            native_row = dict(row, parsed=parsed)
            require(oracle(native_row, text, expected, row['phase'], row['nonce'], 0) == [], 'exact original strict oracle')
    return 28


def semantic(data):
    require(data['schema'] == 1 and data['decision'] == 'rejected', 'rejected schema')
    require(data['promotion'] is False and data['speed_claim'] is False
            and data['causal_regression_claim'] is False and data['noise'] == 'unknown', 'nonclaims')
    require(set(data['cases']) == set(CASES), 'all four cases')
    nonces = set()
    for name in CASES:
        case = data['cases'][name]
        enabled = name in ('on', 'on-repeat')
        require(case['status'] == 'passed' and case['unchanged'] is True, 'case accepted')
        require(case['controls'] == dict(CONTROLS, RA_JIT_HOME_EMPTY_FAST='1' if enabled else '0'), 'exact controls')
        require(case['source_before'] == case['source_after'], 'source unchanged')
        require(case['toolchain_before'] == case['toolchain_after'], 'toolchain unchanged')
        require(case['tool_before'] == case['tool_after'] == BINARY, 'same binary')
        require([row['phase'] for row in case['rows']] == list(PHASES), 'seven ordered phases')
        for row in case['rows']:
            phase, nonce = row['phase'], row['nonce']
            require(re.fullmatch('[0-9a-f]{32}', nonce) and nonce not in nonces, 'fresh nonce')
            nonces.add(nonce)
            require(row['accepted'] is True and row['input_unchanged'] is True and row['failures'] == []
                    and row['raw_error'] is None and row['error'] is None and row['timed_out'] is False,
                    'strict row accepted')
            require(row['safe_environment'] == ENV and row['omitted_environment_entries'] == 12, 'safe env allowlist')
            require(row['noise_contamination'] == 'unknown', 'noise remains unknown')
            require(math.isfinite(row['elapsed_seconds']) and row['elapsed_seconds'] > 0, 'positive elapsed')
            require(row['marker_lines'] == ([MARKER] if enabled else []), 'cumulative entry snapshot')
            require(row['status_receipt'] == {'status': 0, 'error': None, 'timed_out': False}, 'status command')
            require(len(row['input_inventory']) == 63, 'all source input identities')
            text = row['stdout']
            diagnostics = '\n'.join(row['diagnostic_lines'])
            runtime = re.findall(r'JR_MATRIX_RUNTIME_[^\s]*', text + '\n' + diagnostics)
            if phase == 'type-error':
                require(row['status'] == 101 and row['parsed'] is None and not runtime, 'type-error no runtime')
                require('error[E0308]:' in diagnostics and 'JR_MATRIX_TYPE_' + nonce in diagnostics,
                        'fresh type diagnostic')
                require('test result:' not in text and len(row['diagnostic_lines']) == 2, 'type-error no results')
            else:
                failed = phase == 'assert-failure'
                tests = [[n, 'FAILED' if failed and n == NAMES[1] else 'ok'] for n in NAMES]
                expected = {'tests': tests, 'passed': 3 if failed else 4, 'failed': int(failed),
                            'ignored': 0, 'filtered': 144}
                require(row['parsed'] == expected, 'exact named results')
                require(row['status'] == (101 if failed else 0), 'exact process status')
                require(runtime == ['JR_MATRIX_RUNTIME_' + nonce], 'exact fresh invocation witness')
                clean = text.replace('JR_MATRIX_RUNTIME_' + nonce + '\n', '')
                require(re.findall(r'^test (\S+) \.\.\. (ok|FAILED)$', clean, re.M) == [tuple(t) for t in tests],
                        'raw named results')
                summary = ('test result: FAILED. 3 passed; 1 failed;' if failed else
                           'test result: ok. 4 passed; 0 failed;')
                require(len(re.findall(r'^test result:', text, re.M)) == 1
                        and summary + ' 0 ignored; 0 measured; 144 filtered out;' in text, 'raw full counts')
                require(row['diagnostic_lines'] == ([
                    'assertion `left == right` failed: JR_MATRIX_FAILURE_' + nonce] if failed else []),
                    'fresh assertion/no unexpected diagnostics')

    analysis = data['root_analysis']
    require(analysis['status'] == 'passed' and analysis['strict_rows'] == 28
            and analysis['native_codegen'] == 56 and analysis['native_codegen_ignored'] == 1 and analysis['native_resident'] == 364
            and analysis['native_home_fixtures'] == 3 and analysis['separate_no_home_guard_passed'] == 1 and analysis['promotion'] is False
            and analysis['speed_claim'] is False and analysis['noise'] == 'unknown'
            and analysis['decision'] == 'REJECTED: no repeatable useful scoped latency improvement', 'root rejection')
    require(analysis['marker_scope'] == 'Cumulative actual branch-entry snapshot only, not per-phase work counts or latency attribution',
            'no marker-to-hit inference')
    for name in CASES:
        require(analysis['markers'][name] == [int(name in ('on', 'on-repeat'))] * 7, 'root cumulative markers')
    require(len(analysis['comparisons']) == 2, 'both descriptive pairs')
    for pair, (off, on) in zip(analysis['comparisons'], (('off', 'on'), ('off-repeat', 'on-repeat'))):
        require((pair['off'], pair['on']) == (off, on), 'pair identity')
        require([r['phase'] for r in pair['rows']] == list(PHASES), 'pair phases')
        for result, baseline, candidate in zip(pair['rows'], data['cases'][off]['rows'], data['cases'][on]['rows']):
            old, new = baseline['elapsed_seconds'], candidate['elapsed_seconds']
            require(result['off_seconds'] == old and result['on_seconds'] == new, 'raw pair seconds')
            require(abs(result['percent_on_vs_off'] - 100 * (new / old - 1)) < 1e-10, 'pair arithmetic')

    native = data['native']
    require(all(native[g]['exit'] == 0 for g in ('check', 'compile', 'codegen-native', 'full-native')), 'native gate exits')
    for gate, count in [('codegen-native', 56), ('full-native', 364)]:
        require(any(f'{count} tests passed' in line for line in native[gate]['summaries']), 'native summary count')
    require(native['source_count'] == 3757 and native['source_check_ok'] is True and native['lock_check_ok'] is True,
            'whole native source and lock checks')
    require(native['lock_sha256'] == 'df2ba303eac0f8b2337763c14433a0233b597eb35c0131141e1436761c471a52', 'actual integrated lock identity')
    runs = {'unset': '20261031-050328326-1001960', 'off': '20261031-050328715-1002015',
            'on': '20261031-050329164-1002079', 'no-home': '20261031-050329749-1002229'}
    require(set(data['fixture_controls']) == set(runs), 'four actual fresh-process fixture controls')
    for label, run in runs.items():
        item = data['fixture_controls'][label]
        require(item['run_id'] == run and item['marker_count'] == int(label == 'on'), 'actual fixture control identity and marker')
        summary = '1 test passed' if label == 'no-home' else '3 tests passed'
        require(any(summary in line for line in item['summaries']), 'actual fixture control count')
    witness = data['fixture_launch_witness']
    require(witness['scope'] == 'Coordinator post-hoc witness of actual tool invocation, not a contemporaneous worker-generated launch receipt', 'honest launch provenance scope')
    require(witness['observed_task_exit'] == 0 and witness['task_id'] == '20830930vv'
            and witness['observed_task_output'] == 'all four independent fixture processes passed', 'actual coordinator fixture task')
    require(witness['working_directory'] == 'toolchain/resident'
            and witness['common_arguments'] == ['test', '--release', '--locked', '-p', 'hir-ty-codegen', '-j', '6'], 'actual fixture launch arguments')
    expected_launches = []
    for label, run in runs.items():
        expected_launches.append({'label': 'no_home' if label == 'no-home' else label,
            'RA_JIT_HOME_EMPTY_FAST': None if label == 'unset' else ('0' if label == 'off' else '1'),
            'RA_JIT_NO_HOME': '1' if label == 'no-home' else None,
            'filter': GUARD if label == 'no-home' else 'v68_real_database_',
            'extra_arguments': ['--', '--ignored'] if label == 'no-home' else [], 'exit': 0, 'run_id': run})
    require(witness['cases'] == expected_launches, 'four exact launch controls and exits')
    require(data['lock_integration'] == {'package': 'hir-ty-codegen', 'added_existing_edges': ['ide-db', 'test-fixture'],
                                       'new_packages': 0, 'source_lane_lock_edits': 0}, 'only two existing lock edges')
    restore = data['restoration']
    require(restore['home_sha256'] == ORIGINAL and restore['manifest_sha256'] == MANIFEST_ORIGINAL
            and restore['source_cmp_original'] is True and restore['source_head_diff_empty'] is True,
            'exact point-in-time restoration')
    require(restore['original_lock_sha256'] == 'bf3307542b2b5c0e7a4b8c5b1a1e650bba9afece80284cc497a3800560a0c2bc', 'original root lock restoration identity')
    cleanup = data['cleanup']
    require(cleanup['stores_deleted'] is False and len(cleanup['rows']) == 4, 'cleanup non-destructive')
    require(all(row['post_measurement_only'] is True and row['status'] == 0 and row['error'] is None
                and row['timed_out'] is False for row in cleanup['rows']), 'post-measurement cleanup')
    require(data['privacy']['environment_allowlist'] == ENV
            and set(data['privacy']['control_allowlist']) == set(CONTROLS) | {'RA_JIT_HOME_EMPTY_FAST'}, 'privacy allowlists')
    require(all(re.fullmatch('[0-9a-f]{64}', item['sha256']) and item['bytes'] >= 0
                for item in data['private_originals'].values()), 'private original provenance')


def replay_patch(original, patch):
    """Apply one exact-context unified diff in memory, without subprocesses."""
    if not patch:
        return original
    lines = patch.splitlines(keepends=True)
    require(lines[0].startswith('--- ') and lines[1].startswith('+++ '), 'patch header')
    old, result, cursor, at = original.splitlines(keepends=True), [], 0, 2
    while at < len(lines):
        header = re.match(r'@@ -(\d+)(?:,\d+)? \+(\d+)(?:,\d+)? @@', lines[at])
        require(header is not None, 'patch hunk')
        position = max(int(header[1]) - 1, 0)
        require(cursor <= position <= len(old), 'patch ordering')
        result.extend(old[cursor:position])
        cursor, at = position, at + 1
        while at < len(lines) and not lines[at].startswith('@@ '):
            kind, line = lines[at][0], lines[at][1:]
            require(kind in ' +-\n', 'patch body prefix')
            if kind in ' -':
                require(cursor < len(old) and old[cursor] == line, 'patch exact context')
                cursor += 1
            if kind in ' +':
                result.append(line)
            at += 1
    result.extend(old[cursor:])
    return ''.join(result)


def validate_package(root, manifest=False):
    raw = (root / 'observations.json').read_bytes()
    require(sha(raw) == OBSERVATIONS, 'fixed historical observation commitment')
    data = json.loads(raw)
    semantic(data)
    require(replay_original_oracles(root, data) == 28, 'all original strict oracle outcomes')
    source = json.loads((root / 'inventories/source63.json').read_text())
    toolchain = json.loads((root / 'inventories/toolchain4020.json').read_text())
    native_source = json.loads((root / 'inventories/native-source3757.json').read_text())
    require(native_source['crates/hir-ty-codegen/home.rs'] == CANDIDATE, 'v68 built source identity')
    require(len(source) == 63 and len(toolchain) == 4020 and len(native_source) == 3757, 'full inventory sizes')
    require(sha(canonical(native_source)) == data['native']['source_inventory_identity'], 'native inventory identity')
    for case in data['cases'].values():
        require(case['source_before'] == sha(canonical(source))
                and case['toolchain_before'] == sha(canonical(toolchain)), 'bound source/toolchain inventories')
    frozen = root / 'frozen'
    for folder, digest in [('baseline', ORIGINAL), ('candidate', CANDIDATE), ('live-before-restoration', CANDIDATE)]:
        require(sha((frozen / folder / 'home.rs').read_bytes()) == digest, 'exact source ' + folder)
    require(sha((frozen / 'baseline/Cargo.toml').read_bytes()) == MANIFEST_ORIGINAL, 'original manifest')
    for version in ('candidate', 'live-before-restoration'):
        require(sha((frozen / version / 'Cargo.toml').read_bytes()) == MANIFEST_CANDIDATE, 'candidate manifest')
    require((frozen / 'candidate/doc.md').read_bytes() == (frozen / 'live-before-restoration/doc.md').read_bytes(), 'live proposal matched frozen candidate')
    for name in ('home.rs', 'Cargo.toml'):
        require(replay_patch((frozen / 'baseline' / name).read_text(), (frozen / ('change-' + name + '.patch')).read_text())
                == (frozen / 'candidate' / name).read_text(), 'exact candidate source patch replay')
    b = (frozen / 'candidate/home.rs').read_text()
    restored = b[:b.index('\n#[cfg(test)]')]
    start = restored.index('/// Process-latched, exact opt-in.')
    end = restored.index('impl Homes {', start)
    restored = restored[:start] + restored[end:]
    start = restored.index('        if empty_fast_enabled()')
    end = restored.index('        let mentions = Mentions::of(db, instance);', start)
    restored = restored[:start] + restored[end:]
    require(restored == (frozen / 'baseline/home.rs').read_text(), 'bytewise original path retained')
    for name, count, ignored in [('codegen', 56, 1), ('resident', 364, 0)]:
        lines = json.loads((root / ('native/' + name + '-results.json')).read_text())
        ignored_lines = [line for line in lines[:-1] if ' ... ignored, ' in line]
        expected_ignored = ['test home::empty_fast_tests::' + GUARD + ' ... ignored, run in a fresh process with RA_JIT_NO_HOME=1, independently of regular fixtures'] if ignored else []
        require(ignored_lines == expected_ignored, 'actual ignored native guard result')
        results = [re.fullmatch(r'test ([A-Za-z0-9_:]+) \.\.\. ok <[0-9.]+s>', line) for line in lines[:-1] if line not in ignored_lines]
        require(len(results) == count and all(results) and len({m[1] for m in results}) == count, 'full native named results')
        require(re.fullmatch(r'test result: ok\. ' + str(count)
                + r' passed; 0 failed; ' + str(ignored) + r' ignored; 0 measured; 0 filtered out; finished in [0-9.]+s', lines[-1]), 'full native raw summary')
        if name == 'codegen':
            for fixture in FIXTURES:
                require(sum(m[1] == 'home::empty_fast_tests::' + fixture for m in results) == 1, 'actual native fixture ' + fixture)
    for label in ('unset', 'off', 'on', 'no-home'):
        lines = json.loads((root / ('native/fixtures-' + label + '.json')).read_text())
        expected_names = [GUARD] if label == 'no-home' else list(FIXTURES)
        passed = [re.fullmatch(r'test home::empty_fast_tests::([A-Za-z0-9_]+) \.\.\. ok <[0-9.]+s>', line) for line in lines]
        require(sorted(m[1] for m in passed if m) == sorted(expected_names), 'four actual fixture-control result sets')
        require(lines.count(MARKER) == int(label == 'on'), 'fixture branch marker control')
        require(data['fixture_controls'][label]['marker_count'] == int(label == 'on'), 'fixture control marker receipt')
        expected_ignored = [] if label == 'no-home' else ['test home::empty_fast_tests::' + GUARD + ' ... ignored, run in a fresh process with RA_JIT_NO_HOME=1, independently of regular fixtures']
        require([line for line in lines if ' ... ignored' in line] == expected_ignored, 'fixture control exact ignored guard')
        require(len(lines) == (2 if label == 'no-home' else 5 + int(label == 'on')), 'fixture control complete selected lines')
        summary = r'test result: ok\. 1 passed; 0 failed; 0 ignored; 0 measured; 56 filtered out; finished in [0-9.]+s' if label == 'no-home' else r'test result: ok\. 3 passed; 0 failed; 1 ignored; 0 measured; 53 filtered out; finished in [0-9.]+s'
        require(sum(bool(re.fullmatch(summary, line)) for line in lines) == 1, 'fixture control full summary')
    lock_patch = (frozen / 'lock.patch.txt').read_text().splitlines()
    require([line for line in lock_patch if line.startswith('+') and not line.startswith('+++')]
            == ['+ "ide-db",', '+ "test-fixture",']
            and not [line for line in lock_patch if line.startswith('-') and not line.startswith('---')],
            'actual lock patch only adds two existing edges')
    require(json.loads((root / 'analysis/root-output.json').read_text()) == data['root_analysis'], 'root analysis output retained')
    require(json.loads((root / 'native/fixture-launch-witness.json').read_text()) == data['fixture_launch_witness'], 'coordinator launch witness retained')
    require(json.loads((root / 'checks/restoration.json').read_text()) == data['restoration'], 'actual restoration receipt')
    # Exclude path-bearing env keys by construction. Source/script derivatives are not receipts.
    observation_text = raw.decode()
    for forbidden in ('/home/', 'Authorization', 'API_KEY', 'RUSTUP_HOME', 'XDG_CACHE_HOME'):
        require(forbidden not in observation_text, 'receipt privacy')
    if manifest:
        expected = json.loads((root / 'SHA256.json').read_text())
        actual = {str(p.relative_to(root)): sha(p.read_bytes()) for p in root.rglob('*')
                  if p.is_file() and p.name != 'SHA256.json' and '__pycache__' not in p.parts}
        require(expected == actual, 'complete package manifest')
    return {'decision': 'rejected', 'strict_rows': 28, 'native': [56, 364], 'fixtures': 3, 'fixture_controls': 4,
            'marker': 'cumulative branch entry only, not per-phase work or latency attribution',
            'source63': 63, 'toolchain4020': 4020, 'speed_claim': False, 'native_jobs_executed': 0}


if __name__ == '__main__':
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--manifest', action='store_true')
    args = parser.parse_args()
    print(json.dumps(validate_package(Path(__file__).resolve().parent, args.manifest), sort_keys=True))
