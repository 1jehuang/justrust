#!/usr/bin/env python3
"""Stdlib-only semantic replay of historical home-query evidence. Never launches native tools."""
import argparse
import hashlib
import json
import math
import re
from pathlib import Path

CASES = ('off', 'on', 'on-repeat', 'off-repeat')
PHASES = ('cold-prepare', 'repeated-noop-1', 'manifest-feature-add', 'type-error',
          'type-recovery', 'assert-failure', 'assert-recovery')
NAMES = ('summary::tests::parses_libtest_results', 'summary::tests::shares_overlapping_time',
         'summary::tests::splits_passes_without_double_counting',
         'summary::tests::test_failures_are_not_compile_errors')
FIXTURES = ('native_home_exact_opt_in', 'native_home_real_db_cold_repeat_full_key_and_empty_bypass',
            'native_home_real_db_opaque_fallback', 'native_home_real_db_signature_and_dependency_revisions',
            'native_home_real_db_closure_owner_revision')
ENV = {'BLAZE_BACKEND': 'cranelift', 'BLAZE_OPT': '0',
       'CARGO_PROFILE_DEV_DEBUG': '0', 'CARGO_PROFILE_DEV_INCREMENTAL': 'true',
       'CARGO_PROFILE_DEV_OPT_LEVEL': '0', 'CARGO_PROFILE_TEST_DEBUG': '0',
       'CARGO_PROFILE_TEST_INCREMENTAL': 'true', 'CARGO_PROFILE_TEST_OPT_LEVEL': '0'}
CONTROLS = {'RA_JIT_DEMAND_STD': '1', 'RA_JIT_CACHE_MISS_STATS': '0',
            'RA_JIT_STATS': '1', 'RA_JIT_WHOLE_RELOAD': '0'}
MARKER = 'ra_jit: native-home-query-v67 QUERY ENTERED (not workload cache-hit evidence)'
BINARY = 'c3fa3dad9de29af5884061770dd612b706023509e3733267fcdd6df53b932b70'
ORIGINAL = '17aa54c87bcf5ec64b3404577352d524c7d454e92bcb89ad98dba225ba6438a7'
V66 = '9e0100e5fef4413c594d32fafa1292d75ea1c90bd44d18bf648c2ddd28877e8e'
V67 = '692f9dd54ed132f6dc508fb2e869a7aff68f4bd0dd40dc413ce99a51542ea8f0'
MANIFEST_ORIGINAL = 'f46dec5b2cf06f30456f2379967d4bb1747c9e690ae35a1861d3775c95eab381'
MANIFEST_CANDIDATE = '70951a5a4d68e479a7a1ab7a452459fe510b211db3705cc5201e91f5d2771159'
OBSERVATIONS = 'eaab9d2dfd73e1b52a4c5b2a570c9722fd4580e31b55c81fe9ac44337c575b01'


def sha(raw):
    return hashlib.sha256(raw).hexdigest()


def canonical(value):
    return json.dumps(value, sort_keys=True, separators=(',', ':')).encode()


def require(condition, message):
    if not condition:
        raise ValueError(message)


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
        require(case['controls'] == dict(CONTROLS, RA_JIT_HOME_MEMO='1' if enabled else '0'), 'exact controls')
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
            and analysis['native_codegen'] == 58 and analysis['native_resident'] == 364
            and analysis['native_home_fixtures'] == 5 and analysis['promotion'] is False
            and analysis['speed_claim'] is False and analysis['noise'] == 'unknown'
            and analysis['decision'] == 'REJECTED: no repeatable useful scoped latency improvement', 'root rejection')
    require(analysis['marker_scope'] == 'Cumulative provider-entry snapshot only, not workload cache hit or per-phase provider avoidance',
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
    for gate, count in [('codegen-native', 58), ('full-native', 364)]:
        require(any(f'{count} tests passed' in line for line in native[gate]['summaries']), 'native summary count')
    require(native['source_count'] == 3757 and native['source_check_ok'] is True and native['lock_check_ok'] is True,
            'whole native source and lock checks')
    require(data['v66_failure']['exit'] == data['v66_failure']['orchestration_exit'] == 101, 'v66 compile failure retained')
    headers = '\n'.join(data['v66_failure']['error_headers'])
    require('E0277' in headers and 'E0271' in headers and all(x in headers for x in
            ('SalsaStructInDb', 'AsId', 'FromIdWithDb', 'Copied<Iter')), 'actual v66 error kinds')
    require(data['v66_failure']['source_inventory_count'] == 3757, 'v66 inventory')
    require(data['lock_integration'] == {'package': 'hir-ty-codegen', 'added_existing_edges': ['ide-db', 'test-fixture'],
                                       'new_packages': 0, 'source_lane_lock_edits': 0}, 'only two existing lock edges')
    restore = data['restoration']
    require(restore['home_sha256'] == ORIGINAL and restore['manifest_sha256'] == MANIFEST_ORIGINAL
            and restore['source_cmp_original'] is True and restore['source_head_diff_empty'] is True,
            'exact point-in-time restoration')
    cleanup = data['cleanup']
    require(cleanup['stores_deleted'] is False and len(cleanup['rows']) == 4, 'cleanup non-destructive')
    require(all(row['post_measurement_only'] is True and row['status'] == 0 and row['error'] is None
                and row['timed_out'] is False for row in cleanup['rows']), 'post-measurement cleanup')
    require(data['privacy']['environment_allowlist'] == ENV
            and set(data['privacy']['control_allowlist']) == set(CONTROLS) | {'RA_JIT_HOME_MEMO'}, 'privacy allowlists')
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
    source = json.loads((root / 'inventories/source63.json').read_text())
    toolchain = json.loads((root / 'inventories/toolchain4020.json').read_text())
    native_source = json.loads((root / 'inventories/native-source3757.json').read_text())
    v66_source = json.loads((root / 'inventories/v66-native-source3757.json').read_text())
    require(len(v66_source) == 3757 and v66_source['crates/hir-ty-codegen/home.rs'] == V66,
            'full failed-v66 source inventory')
    require(native_source['crates/hir-ty-codegen/home.rs'] == V67, 'v67 built source identity')
    require(len(source) == 63 and len(toolchain) == 4020 and len(native_source) == 3757, 'full inventory sizes')
    require(sha(canonical(native_source)) == data['native']['source_inventory_identity'], 'native inventory identity')
    for case in data['cases'].values():
        require(case['source_before'] == sha(canonical(source))
                and case['toolchain_before'] == sha(canonical(toolchain)), 'bound source/toolchain inventories')
    frozen = root / 'frozen'
    for folder, digest in [('baseline', ORIGINAL), ('v66', V66), ('v67', V67), ('live-before-restoration', V67)]:
        require(sha((frozen / folder / 'home.rs').read_bytes()) == digest, 'exact source ' + folder)
    require(sha((frozen / 'baseline/Cargo.toml').read_bytes()) == MANIFEST_ORIGINAL, 'original manifest')
    for version in ('v66', 'v67', 'live-before-restoration'):
        require(sha((frozen / version / 'Cargo.toml').read_bytes()) == MANIFEST_CANDIDATE, 'candidate manifest')
    for name in ('home.rs', 'Cargo.toml', 'doc.md'):
        require(replay_patch((frozen / 'v66' / name).read_text(), (frozen / ('repair-' + name + '.patch')).read_text())
                == (frozen / 'v67' / name).read_text(), 'exact incremental source/doc patch replay')
    a, b = (frozen / 'v66/home.rs').read_text(), (frozen / 'v67/home.rs').read_text()
    require(a[a.index('mod tests {'):] == b[b.index('mod tests {'):], 'strict fixture sources unchanged')
    require('instance: Instance, _unit: ()' in b and 'native_home_crates(db, instance.clone(), ())' in b,
            'sound full instance plus unit key')
    for name, count in [('codegen', 58), ('resident', 364)]:
        lines = json.loads((root / ('native/' + name + '-results.json')).read_text())
        results = [re.fullmatch(r'test ([A-Za-z0-9_:]+) \.\.\. ok <[0-9.]+s>', line) for line in lines[:-1]]
        require(len(results) == count and all(results) and len({m[1] for m in results}) == count, 'full native named results')
        require(re.fullmatch(r'test result: ok\. ' + str(count)
                + r' passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in [0-9.]+s', lines[-1]), 'full native raw summary')
        if name == 'codegen':
            for fixture in FIXTURES:
                require(sum(m[1] == 'home::tests::' + fixture for m in results) == 1, 'actual native fixture ' + fixture)
    lock_patch = (frozen / 'home-query-v66-root-integration--lock.patch.txt').read_text().splitlines()
    require([line for line in lock_patch if line.startswith('+') and not line.startswith('+++')]
            == ['+ "ide-db",', '+ "test-fixture",']
            and not [line for line in lock_patch if line.startswith('-') and not line.startswith('---')],
            'actual lock patch only adds two existing edges')
    require(json.loads((root / 'analysis/root-output.json').read_text()) == data['root_analysis'], 'root analysis output retained')
    restored = (root / 'checks/restored.sha256.txt').read_text().splitlines()
    require([line[:64] for line in restored] == [ORIGINAL, MANIFEST_ORIGINAL], 'actual restoration hashes')
    # Exclude path-bearing env keys by construction. Source/script derivatives are not receipts.
    observation_text = raw.decode()
    for forbidden in ('/home/', 'Authorization', 'API_KEY', 'RUSTUP_HOME', 'XDG_CACHE_HOME'):
        require(forbidden not in observation_text, 'receipt privacy')
    if manifest:
        expected = json.loads((root / 'SHA256.json').read_text())
        actual = {str(p.relative_to(root)): sha(p.read_bytes()) for p in root.rglob('*')
                  if p.is_file() and p.name != 'SHA256.json' and '__pycache__' not in p.parts}
        require(expected == actual, 'complete package manifest')
    return {'decision': 'rejected', 'strict_rows': 28, 'native': [58, 364], 'fixtures': 5,
            'marker': 'cumulative QUERY ENTERED only, not cache-hit proof',
            'source63': 63, 'toolchain4020': 4020, 'speed_claim': False, 'native_jobs_executed': 0}


if __name__ == '__main__':
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--manifest', action='store_true')
    args = parser.parse_args()
    print(json.dumps(validate_package(Path(__file__).resolve().parent, args.manifest), sort_keys=True))
