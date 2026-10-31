#!/usr/bin/env python3
"""Stdlib-only semantic replay of historical declared-query evidence. Never launches native tools."""
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
ENV = {'BLAZE_BACKEND': 'cranelift', 'BLAZE_OPT': '0',
       'CARGO_PROFILE_DEV_DEBUG': '0', 'CARGO_PROFILE_DEV_INCREMENTAL': 'true',
       'CARGO_PROFILE_DEV_OPT_LEVEL': '0', 'CARGO_PROFILE_TEST_DEBUG': '0',
       'CARGO_PROFILE_TEST_INCREMENTAL': 'true', 'CARGO_PROFILE_TEST_OPT_LEVEL': '0'}
CONTROLS = {'RA_JIT_DEMAND_STD': '1', 'RA_JIT_CACHE_MISS_STATS': '0',
            'RA_JIT_STATS': '1', 'RA_JIT_WHOLE_RELOAD': '0'}
MARKER = r'declared-query-provider-entry pid=[0-9]+'
BINARY = '1a951b1bc9374c8dfb2e8b2977f5749c9df4a0a8fe6d291ff760066e6b05f474'
ORIGINAL = '0246385937d776e28fc87a0484db8830078f6bc629e930c988f5acb82b63d176'
CANDIDATE = '1b697f35ac35831b6e77e36980bda0dddeca026d034d624e4a0b0fa8f23bc34a'
OBSERVATIONS = 'dfe07c8c9ce653a02f9d4365457e2cf0838196f87e4109cfce5e95220bae81c0'

NATIVE_DIGESTS = {'codegen': 'd73cf62d0053eba607ace09a476d7e41d513d49c91bf06ec24b784405d498164',
                  'resident': 'd95d8b154c2125e94e4b090484e83fec43e24753cb862edcf0ac8c5aaa678b58'}

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
        require(case['controls'] == dict(CONTROLS, RA_JIT_DECLARED_QUERY='1' if enabled else '0'), 'exact controls')
        require(case['source_before'] == case['source_after'], 'source unchanged')
        require(case['toolchain_before'] == case['toolchain_after'], 'toolchain unchanged')
        require(case['tool_before'] == case['tool_after'] == BINARY, 'same binary')
        require([row['phase'] for row in case['rows']] == list(PHASES), 'seven ordered phases')
        for index, row in enumerate(case['rows']):
            phase, nonce = row['phase'], row['nonce']
            require(re.fullmatch('[0-9a-f]{32}', nonce) and nonce not in nonces, 'fresh nonce')
            nonces.add(nonce)
            require(row['accepted'] is True and row['input_unchanged'] is True and row['failures'] == []
                    and row['raw_error'] is None and row['error'] is None and row['timed_out'] is False,
                    'strict row accepted')
            require(row['safe_environment'] == ENV and row['omitted_environment_entries'] == 12, 'safe env allowlist')
            require(row['noise_contamination'] == 'unknown', 'noise remains unknown')
            require(math.isfinite(row['elapsed_seconds']) and row['elapsed_seconds'] > 0, 'positive elapsed')
            require(len(row['marker_lines']) == ([1, 1, 2, 3, 4, 4, 4][index] if enabled else 0)
                    and all(re.fullmatch(MARKER, line) for line in row['marker_lines']), 'cumulative entry snapshot')
            require(case['rows'][-1]['marker_lines'][:len(row['marker_lines'])] == row['marker_lines'], 'cumulative marker prefix')
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
            and analysis['native_codegen'] == 53 and analysis['native_resident'] == 367
            and analysis['promotion'] is False and analysis['speed_claim'] is False
            and analysis['noise'] == 'unknown'
            and analysis['decision'] == 'REJECTED: no useful replicated scoped latency improvement', 'root rejection')
    require(analysis['marker_scope'] == 'Cumulative once-per-process provider entry only, not cache hits or fresh phase work', 'provider-entry-only scope')
    for name in CASES:
        expected = [0] * 7 if name.startswith('off') else [1, 1, 2, 3, 4, 4, 4]
        require(analysis['markers'][name] == expected, 'root cumulative markers')
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
    for gate, count in [('codegen-native', 53), ('full-native', 367)]:
        require(any(f'{count} tests passed' in line for line in native[gate]['summaries']), 'native summary count')
    require(native['source_count'] == 3757 and native['source_check_ok'] is True and native['lock_check_ok'] is True, 'native source and lock checks')
    require(native['lock_sha256'] == 'bf3307542b2b5c0e7a4b8c5b1a1e650bba9afece80284cc497a3800560a0c2bc', 'unchanged root lock identity')
    require(native['engine_sha256'] == BINARY, 'actual engine identity')
    restore = data['restoration']
    require(restore['main_sha256'] == ORIGINAL and restore['source_cmp_original'] is True
            and restore['source_head_diff_empty'] is True and restore['live_main_cmp_frozen'] is True
            and restore['live_doc_cmp_frozen'] is True and restore['lock_modified'] is False, 'point-in-time exact restoration')
    cleanup = data['cleanup']
    require(cleanup['stores_deleted'] is False and len(cleanup['rows']) == 4, 'non-destructive cleanup')
    require(all(row['post_measurement_only'] is True and row['status'] == 0 and row['error'] is None
                and row['timed_out'] is False for row in cleanup['rows']), 'post-measurement cleanup')
    require(data['privacy']['environment_allowlist'] == ENV
            and set(data['privacy']['control_allowlist']) == set(CONTROLS) | {'RA_JIT_DECLARED_QUERY'}, 'privacy allowlists')
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
    require(native_source['crates/ra-jit/src/main.rs'] == CANDIDATE, 'built candidate main identity')
    require(len(source) == 63 and len(toolchain) == 4020 and len(native_source) == 3757, 'full inventory sizes')
    for inventory in (source, toolchain, native_source):
        require(all(not p.startswith('/') and '..' not in Path(p).parts for p in inventory), 'relative inventory privacy')
    require(sha(canonical(native_source)) == data['native']['source_inventory_identity'], 'native inventory commitment')
    for case in data['cases'].values():
        require(case['source_before'] == sha(canonical(source))
                and case['toolchain_before'] == sha(canonical(toolchain)), 'bound source/toolchain inventories')
    frozen = root / 'frozen'
    for folder, digest in [('baseline', ORIGINAL), ('candidate', CANDIDATE), ('live-before-restoration', CANDIDATE)]:
        require(sha((frozen / folder / 'main.rs').read_bytes()) == digest, 'exact full source ' + folder)
    require((frozen / 'candidate/doc.md').read_bytes() == (frozen / 'live-before-restoration/doc.md').read_bytes(), 'live proposal matched frozen')
    baseline = (frozen / 'baseline/main.rs').read_text()
    candidate = (frozen / 'candidate/main.rs').read_text()
    require(replay_patch(baseline, (frozen / 'change-main.rs.patch').read_text()) == candidate, 'exact source patch replay')
    start = 'fn declared_in(db: &RootDatabase, krate: Crate) -> Declared {'
    end = '\n/// The functions of a crate that no body declares'
    require(baseline.split(start, 1)[1].split(end, 1)[0] == candidate.split(
        start.replace('declared_in(', 'declared_in_original('), 1)[1].split(end, 1)[0], 'bytewise original disabled algorithm')
    for name, count in [('codegen', 53), ('resident', 367)]:
        selected_raw = (root / ('native/' + name + '-results.json')).read_bytes()
        require(sha(selected_raw) == NATIVE_DIGESTS[name], 'fixed historical native named results')
        lines = json.loads(selected_raw)
        results = [re.fullmatch(r'test ([A-Za-z0-9_:]+) \.\.\. ok <[0-9.]+s>', line) for line in lines[:-1]]
        require(len(results) == count and all(results) and len({m[1] for m in results}) == count, 'complete native named results')
        require(re.fullmatch(r'test result: ok\. ' + str(count)
                + r' passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in [0-9.]+s', lines[-1]), 'complete native raw summary')
        if name == 'resident':
            for fixture in ('declared_query_flag_is_exact_and_default_off',
                            'declared_query_real_database_revisions_match_both_original_walkers',
                            'declared_query_tracks_dependency_macros_and_reuses_unrelated_crate'):
                require(sum(m[1] == 'tests::' + fixture for m in results) == 1, 'actual declared query fixture ' + fixture)
    require(json.loads((root / 'analysis/root-output.json').read_text()) == data['root_analysis'], 'historical root output')
    require(json.loads((root / 'checks/restoration.json').read_text()) == data['restoration'], 'restoration receipt')
    for forbidden in ('/home/', 'Authorization', 'API_KEY', 'RUSTUP_HOME', 'XDG_CACHE_HOME'):
        require(forbidden not in raw.decode(), 'receipt privacy')
    if manifest:
        expected = json.loads((root / 'SHA256.json').read_text())
        actual = {str(p.relative_to(root)): sha(p.read_bytes()) for p in root.rglob('*')
                  if p.is_file() and p.name != 'SHA256.json' and '__pycache__' not in p.parts}
        require(expected == actual, 'complete package manifest')
    return {'decision': 'rejected', 'strict_rows': 28, 'native': [53, 367], 'fixtures': 3,
            'marker': 'cumulative once-per-process provider entry, not cache-hit proof',
            'native_source3757': 3757, 'source63': 63, 'toolchain4020': 4020,
            'speed_claim': False, 'causal_regression_claim': False, 'noise': 'unknown', 'native_jobs_executed': 0}


if __name__ == '__main__':
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--manifest', action='store_true')
    args = parser.parse_args()
    print(json.dumps(validate_package(Path(__file__).resolve().parent, args.manifest), sort_keys=True))
