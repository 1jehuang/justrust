#!/usr/bin/env python3
"""Fixed allowlist export of historical v71 evidence, never a native execution."""
import argparse
import difflib
import hashlib
import json
import re
from pathlib import Path

CASES = ('off', 'on', 'on-repeat', 'off-repeat')
ENV = {'BLAZE_BACKEND': 'cranelift', 'BLAZE_OPT': '0',
       'CARGO_PROFILE_DEV_DEBUG': '0', 'CARGO_PROFILE_DEV_INCREMENTAL': 'true',
       'CARGO_PROFILE_DEV_OPT_LEVEL': '0', 'CARGO_PROFILE_TEST_DEBUG': '0',
       'CARGO_PROFILE_TEST_INCREMENTAL': 'true', 'CARGO_PROFILE_TEST_OPT_LEVEL': '0'}
CONTROLS = ('RA_JIT_DEMAND_STD', 'RA_JIT_CACHE_MISS_STATS', 'RA_JIT_STATS',
            'RA_JIT_WHOLE_RELOAD', 'RA_JIT_LISTING_FLAT_THREADS')
MARKER = r'listing: v70 flat nested branch activated'
NAMES = ('summary::tests::parses_libtest_results', 'summary::tests::shares_overlapping_time',
         'summary::tests::splits_passes_without_double_counting',
         'summary::tests::test_failures_are_not_compile_errors')


def sha(raw):
    return hashlib.sha256(raw).hexdigest()


def canonical(value):
    return json.dumps(value, sort_keys=True, separators=(',', ':')).encode()


def manifest(output):
    files = {str(p.relative_to(output)): sha(p.read_bytes()) for p in output.rglob('*')
             if p.is_file() and p.name != 'SHA256.json' and '__pycache__' not in p.parts}
    (output / 'SHA256.json').write_text(json.dumps(files, sort_keys=True, indent=2) + '\n')


def collect(root, output):
    output.mkdir(parents=True, exist_ok=True)
    provenance, private_reads = {}, {}

    def read(relative):
        p = root / relative
        raw = p.read_bytes()
        provenance[relative] = {'sha256': sha(raw), 'bytes': len(raw)}
        private_reads[p] = sha(raw)
        return raw

    def load(relative):
        return json.loads(read(relative))

    def save(relative, raw):
        p = output / relative
        p.parent.mkdir(parents=True, exist_ok=True)
        p.write_bytes(raw)

    def jsave(relative, value):
        save(relative, (json.dumps(value, sort_keys=True, indent=2) + '\n').encode())

    def safe_text(raw):
        # This transforms only specifically selected source/script files, never env dumps.
        private_home = root.parents[2]
        return raw.decode().replace(str(root), '<PRIVATE_EXPERIMENT_ROOT>').replace(
            str(private_home / 'justrust'), '<PRIVATE_CHECKOUT>').replace(str(private_home), '<PRIVATE_HOME>')

    data = {'schema': 1, 'decision': 'rejected', 'promotion': False, 'speed_claim': False,
            'causal_regression_claim': False, 'noise': 'unknown', 'cases': {}, 'native': {}}
    source = toolchain = None
    for case in CASES:
        folder = 'listing-scheduler-v71-' + case + '-seven'
        state = load(folder + '/state.json')
        if source is None:
            source = state['source_before']
            toolchain = state['toolchain_before']['files']
            jsave('inventories/source63.json', source)
            jsave('inventories/toolchain4020.json', toolchain)
        assert source == state['source_before'] == state['source_after']
        assert toolchain == state['toolchain_before']['files'] == state['toolchain_after']['files']
        rows = []
        for index, row in enumerate(state['rows']):
            stem = folder + f'/{index:02}-' + row['phase']
            stdout_raw, stderr_raw = read(stem + '.stdout'), read(stem + '.stderr')
            for kind, raw in [('stdout', stdout_raw), ('stderr', stderr_raw)]:
                assert row['raw'][kind]['bytes'] == len(raw)
                assert row['raw'][kind]['sha256'] == sha(raw)
            stdout = stdout_raw.decode()
            # Exact stdout line grammar. Unexpected lines abort rather than leak data.
            for line in stdout.splitlines():
                assert (line in ('', 'running 4 tests', 'ok', 'FAILED', 'failures:', '    ' + NAMES[1])
                        or re.fullmatch(r'test summary::tests::[a-z_]+ \.\.\. (?:ok|FAILED|JR_MATRIX_RUNTIME_[0-9a-f]{32})', line)
                        or re.fullmatch(r'test result: (?:ok|FAILED)\. [0-4] passed; [01] failed; 0 ignored; 0 measured; 144 filtered out; finished in [0-9.]+s', line))
            diagnostics = [line for line in stderr_raw.decode().splitlines() if (
                line == "error[E0308]: expected u64, found &'static str"
                or re.fullmatch(r'\s*\d+ \| let _: u64 = "JR_MATRIX_TYPE_[0-9a-f]{32}";', line)
                or re.fullmatch(r'assertion `left == right` failed: JR_MATRIX_FAILURE_[0-9a-f]{32}', line))]
            worker = read(stem + '-worker-0.log').decode()
            marker_lines = [line for line in worker.splitlines() if line == MARKER]
            assert worker.count(MARKER) == len(marker_lines)
            safe_env = {k: row['environment'][k] for k in ENV}
            assert safe_env == ENV
            assert row['accepted'] is True and row['input_unchanged'] is True and row['failures'] == []
            assert row['error'] is None and row['raw_error'] is None and row['timed_out'] is False
            assert row['status_receipt']['error'] is None and row['status_receipt']['timed_out'] is False
            assert set(row['inputs']) == set(source)
            result = {k: row[k] for k in ('phase', 'nonce', 'accepted', 'status', 'error', 'timed_out',
                       'elapsed_seconds', 'raw_error', 'parsed', 'input_unchanged', 'failures')}
            result.update(stdout=stdout, diagnostic_lines=diagnostics,
                          marker_lines=marker_lines,
                          safe_environment=safe_env,
                          omitted_environment_entries=len(row['environment']) - len(safe_env),
                          input_inventory=row['inputs'], noise_contamination=row['noise']['contamination'],
                          status_receipt={k: row['status_receipt'][k] for k in ('status', 'error', 'timed_out')})
            rows.append(result)
        data['cases'][case] = {'status': state['status'], 'unchanged': state['unchanged'],
            'controls': {k: state['startup_controls'][k] for k in CONTROLS},
            'source_before': sha(canonical(state['source_before'])),
            'source_after': sha(canonical(state['source_after'])),
            'tool_before': state['tool_before'], 'tool_after': state['tool_after'],
            'toolchain_before': sha(canonical(state['toolchain_before']['files'])),
            'toolchain_after': sha(canonical(state['toolchain_after']['files'])), 'rows': rows}


    # Explicit full-source allowlist, with separately sanitized historical docs/patches.
    for name, relative in [
        ('baseline', 'listing-scheduler-v70/originals/main.rs'),
        ('failed-v70', 'listing-scheduler-v70/candidates/main.rs'),
        ('candidate', 'listing-scheduler-v71/candidates/main.rs'),
        ('live-before-restoration', 'listing-scheduler-v71/live-before-restoration/main.rs')]:
        save('frozen/' + name + '/main.rs', read(relative))
    for name, relative in [
        ('failed-v70', 'listing-scheduler-v70/candidates/resident-listing-scheduler.md'),
        ('candidate', 'listing-scheduler-v71/candidates/resident-listing-scheduler.md'),
        ('live-before-restoration', 'listing-scheduler-v71/live-before-restoration/resident-listing-scheduler.md')]:
        save('frozen/' + name + '/doc.md', safe_text(read(relative)).encode())
    for version, names in [('v70', ('candidate.patch', 'RELEASE', 'SHA256SUMS', 'checks.txt', 'static-checks.py')),
                           ('v71', ('incremental.patch', 'full.patch', 'RELEASE', 'SHA256SUMS', 'checks.txt',
                                    'rustfmt-parse.exit', 'rustfmt-parse.stderr', 'rustfmt-check.exit', 'rustfmt-check.stderr', 'rustfmt-check.stdout'))]:
        for name in names:
            save('frozen/' + version + '/' + name + '.txt', safe_text(read('listing-scheduler-' + version + '/' + name)).encode())
    for old, new, name in [('baseline', 'candidate', 'change-main.rs.patch'),
                            ('failed-v70', 'candidate', 'fixture-repair-main.rs.patch')]:
        save('frozen/' + name, ''.join(difflib.unified_diff(
            (output / 'frozen' / old / 'main.rs').read_text().splitlines(keepends=True),
            (output / 'frozen' / new / 'main.rs').read_text().splitlines(keepends=True),
            fromfile='a/main.rs', tofile='b/main.rs')).encode())
    failure = read('candidate-build-v70/check.log')
    assert sha(failure) == 'f2655e2a19d65e57bc6dd79e1cc44eedb8b97e6045056495799c0497376b5fa8'
    # This exact fixed-digest full compiler failure transcript contains no private paths.
    assert b'/home/' not in failure and b'Authorization' not in failure and b'API_KEY' not in failure
    save('native/failed-v70-check.log', failure)
    failure_exit = int(read('candidate-build-v70/check.exit'))
    assert failure_exit == 101
    data['failed_v70'] = {'exit': failure_exit, 'log_sha256': sha(failure),
        'error': 'E0308 EditionedFileId passed to relevant_crates requiring FileId',
        'fixture_only_repair': True, 'assertions_weakened': False, 'production_changes_in_v71': False,
        'v71_rustfmt_parse_exit': int(read('listing-scheduler-v71/rustfmt-parse.exit')),
        'v71_rustfmt_check_exit': int(read('listing-scheduler-v71/rustfmt-check.exit'))}
    for gate, folder in [('check', 'candidate-build-v71'), ('compile', 'candidate-build-v71'),
                         ('codegen-native', 'candidate-unit-v71'), ('full-native', 'candidate-unit-v71')]:
        log = read(folder + '/' + gate + '.log').decode()
        data['native'][gate] = {'exit': int(read(folder + '/' + gate + '.exit')),
            'summaries': [line for line in log.splitlines() if line.startswith('justrust: ok:')
                          or line.startswith('  tests   ')]}
    for name, run in [('codegen', '20261031-060411424-1373945'), ('resident', '20261031-060411827-1374092')]:
        journal = read('candidate-unit-v71/justrust-home/runs/' + run + '/output.jsonl')
        lines = [json.loads(line)['l'] for line in journal.splitlines()]
        selected = [line for line in lines if re.fullmatch(r'test [A-Za-z0-9_:]+ \.\.\. (?:ok|FAILED) <[0-9.]+s>', line)
                    or re.fullmatch(r'test result: ok\. \d+ passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in [0-9.]+s', line)]
        jsave('native/' + name + '-results.json', selected)
        data['native'][name + '_run_id'] = run
    before = read('candidate-build-v71/all-source-before.sha256').decode().splitlines()
    after = read('candidate-unit-v71/all-source-after.log').decode().splitlines()
    source_inventory = {line[66:]: line[:64] for line in before}
    assert len(source_inventory) == len(before) == 3757
    assert all(not p.startswith('/') and '..' not in Path(p).parts for p in source_inventory)
    jsave('inventories/native-source3757.json', source_inventory)
    data['native'].update(source_count=len(before), source_inventory_identity=sha(canonical(source_inventory)),
        source_check_ok=len(before) == len(after) and all(line.endswith(': OK') for line in after),
        lock_check_ok=all(line.endswith(b': OK') for line in read('candidate-unit-v71/lock-after.log').splitlines()),
        lock_sha256=read('candidate-build-v71/lock-before.sha256').decode().split()[0],
        engine_version=read('candidate-build-v71/engine-version.txt').decode().strip())
    data['native']['engine_sha256'] = read('candidate-build-v71/engine-identity.sha256').decode().split()[0]
    data['root_analysis'] = load('listing-scheduler-v71-validation.json')
    save('analysis/root-analysis.py.txt', safe_text(read('validate-listing-scheduler-v71.py')).encode())
    jsave('analysis/root-output.json', data['root_analysis'])
    bench = Path(__file__).resolve().parents[2]
    for name in ('resident_state_matrix.py', 'resident_engine_compare.py', 'resident_workload_compare.py', 'blaze_runtime_compare.py', 'blaze_edit_compare.py'):
        raw = (bench / name).read_bytes()
        provenance['repository/bench/' + name] = {'sha256': sha(raw), 'bytes': len(raw)}
        save('oracles/' + name + '.txt', safe_text(raw).encode())
    save('oracles/runtime-driver.py.txt', safe_text(read('probe-listing-scheduler-v70.py')).encode())
    save('oracles/native-gate-v71.sh.txt', safe_text(read('build-test-candidate-v71.sh')).encode())
    cleanup = load('completed-listing-scheduler-v71-stop/receipt.json')
    data['cleanup'] = {'stores_deleted': cleanup['stores_deleted'],
        'rows': [{k: row[k] for k in ('post_measurement_only', 'status', 'error', 'timed_out')}
                 for row in cleanup['rows']]}
    data['restoration'] = {'main_sha256': sha((output / 'frozen/baseline/main.rs').read_bytes()),
        'source_cmp_original': True, 'source_head_diff_empty': True,
        'live_main_cmp_frozen': True, 'live_doc_cmp_frozen': True,
        'witness': 'Actual worker cmp live-to-frozen then preserved live snapshots, write original, cmp original and scoped HEAD diff PASS',
        'time_utc': '2026-10-31T13:11:54Z', 'lock_modified': False,
        'scope': 'Only main.rs restored, point-in-time receipt; source released to coordinator'}
    jsave('checks/restoration.json', data['restoration'])
    data['privacy'] = {'environment_allowlist': ENV, 'control_allowlist': list(CONTROLS),
        'method': 'fixed-field selection and full-line raw-output allowlists, not ambient environment redaction',
        'excluded': ['ambient environment', 'raw private state JSON', 'private homes/cache paths',
                     'raw worker/daemon logs', 'telemetry', 'credentials', 'binaries', 'build targets']}
    data['private_originals'] = provenance
    for path, expected in private_reads.items():
        assert sha(path.read_bytes()) == expected, 'private input changed during export'
    jsave('observations.json', data)


if __name__ == '__main__':
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('private_root', type=Path, nargs='?')
    parser.add_argument('--manifest-only', action='store_true')
    parser.add_argument('--output', type=Path, default=Path(__file__).resolve().parent)
    args = parser.parse_args()
    if not args.manifest_only:
        if args.private_root is None:
            parser.error('private_root required for fixed allowlist export')
        collect(args.private_root, args.output)
    manifest(args.output)
