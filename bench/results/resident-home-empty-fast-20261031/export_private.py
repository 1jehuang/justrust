#!/usr/bin/env python3
"""Fixed allowlist export of historical v68 evidence, never a native execution."""
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
            'RA_JIT_WHOLE_RELOAD', 'RA_JIT_HOME_EMPTY_FAST')
MARKER = 'ra-jit: home-empty-fast-v68 branch'
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
        folder = 'home-empty-fast-v68-' + case + '-seven'
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
            safe_env = {k: row['environment'][k] for k in ENV}
            assert safe_env == ENV
            assert row['accepted'] is True and row['input_unchanged'] is True and row['failures'] == []
            assert row['error'] is None and row['raw_error'] is None and row['timed_out'] is False
            assert row['status_receipt']['error'] is None and row['status_receipt']['timed_out'] is False
            assert set(row['inputs']) == set(source)
            result = {k: row[k] for k in ('phase', 'nonce', 'accepted', 'status', 'error', 'timed_out',
                       'elapsed_seconds', 'raw_error', 'parsed', 'input_unchanged', 'failures')}
            result.update(stdout=stdout, diagnostic_lines=diagnostics,
                          marker_lines=[line for line in worker.splitlines() if line == MARKER],
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

    for folder in ('baseline', 'candidate', 'live-before-restoration'):
        for name in ('home.rs', 'Cargo.toml'):
            save('frozen/' + folder + '/' + name, read('home-empty-fast-v68/' + folder + '/' + name))
    for folder in ('candidate', 'live-before-restoration'):
        save('frozen/' + folder + '/doc.md', safe_text(read('home-empty-fast-v68/' + folder + '/resident-home-empty-fast.md')).encode())
    for name in ('candidate.patch', 'RELEASE.txt', 'SHA256SUMS', 'sourcechecks.txt', 'baseline/doc-absence.txt'):
        save('frozen/' + name.replace('/', '--') + '.txt', safe_text(read('home-empty-fast-v68/' + name)).encode())
    save('frozen/lock.patch.txt', read('home-empty-fast-v68-root-integration/lock.patch'))
    for name in ('home.rs', 'Cargo.toml'):
        save('frozen/change-' + name + '.patch', ''.join(difflib.unified_diff(
            (output / 'frozen/baseline' / name).read_text().splitlines(keepends=True),
            (output / 'frozen/candidate' / name).read_text().splitlines(keepends=True),
            fromfile='a/' + name, tofile='b/' + name)).encode())
    for gate, folder in [('check', 'candidate-build-v68'), ('compile', 'candidate-build-v68'),
                         ('codegen-native', 'candidate-unit-v68'), ('full-native', 'candidate-unit-v68')]:
        log = read(folder + '/' + gate + '.log').decode()
        data['native'][gate] = {'exit': int(read(folder + '/' + gate + '.exit')),
            'summaries': [line for line in log.splitlines() if line.startswith('justrust: ok:')
                          or line.startswith('  tests   ')]}
    for name, run in [('codegen', '20261031-045941862-977964'), ('resident', '20261031-050018487-981591')]:
        journal = read('candidate-unit-v68/justrust-home/runs/' + run + '/output.jsonl')
        lines = [json.loads(line)['l'] for line in journal.splitlines()]
        selected = [line for line in lines if re.fullmatch(r'test [A-Za-z0-9_:]+ \.\.\. (?:ok|FAILED) <[0-9.]+s>', line)
                    or line == 'test home::empty_fast_tests::v68_real_database_no_home_guard ... ignored, run in a fresh process with RA_JIT_NO_HOME=1, independently of regular fixtures'
                    or re.fullmatch(r'test result: ok\. \d+ passed; 0 failed; [01] ignored; 0 measured; 0 filtered out; finished in [0-9.]+s', line)]
        jsave('native/' + name + '-results.json', selected)
    data['fixture_controls'] = {}
    runs = {'unset': '20261031-050328326-1001960', 'off': '20261031-050328715-1002015',
            'on': '20261031-050329164-1002079', 'no-home': '20261031-050329749-1002229'}
    for label, run in runs.items():
        log = read('candidate-unit-v68/fixtures-' + label + '.log').decode()
        raw = read('candidate-unit-v68/justrust-home/runs/' + run + '/output.jsonl')
        lines = [json.loads(line)['l'] for line in raw.splitlines()]
        selected = [line for line in lines if (line.startswith('test home::empty_fast_tests::')
                    or line.startswith('test result:') or line == MARKER)]
        jsave('native/fixtures-' + label + '.json', selected)
        data['fixture_controls'][label] = {'run_id': run, 'marker_count': lines.count(MARKER),
            'summaries': [line for line in log.splitlines() if line.startswith('justrust: ok:') or line.startswith('  tests   ')]}
    witness = load('v68-fixture-launch-coordinator-witness.json')
    data['fixture_launch_witness'] = {key: witness[key] for key in (
        'scope', 'recorded_at', 'task_id', 'observed_task_started_at', 'observed_task_exit',
        'observed_task_output', 'runner', 'working_directory', 'common_arguments', 'limits')}
    data['fixture_launch_witness']['cases'] = [{key: row[key] for key in (
        'label', 'RA_JIT_HOME_EMPTY_FAST', 'RA_JIT_NO_HOME', 'filter', 'extra_arguments', 'exit', 'run_id')}
        for row in witness['cases']]
    jsave('native/fixture-launch-witness.json', data['fixture_launch_witness'])
    before = read('candidate-build-v68/all-source-before.sha256').decode().splitlines()
    after = read('candidate-unit-v68/all-source-after.log').decode().splitlines()
    data['native']['source_count'] = len(before)
    source_inventory = {line[66:]: line[:64] for line in before}
    assert len(source_inventory) == len(before)
    assert all(not p.startswith('/') and '..' not in Path(p).parts for p in source_inventory)
    jsave('inventories/native-source3757.json', source_inventory)
    data['native']['source_inventory_identity'] = sha(canonical(source_inventory))
    data['native']['source_check_ok'] = len(before) == len(after) and all(line.endswith(': OK') for line in after)
    data['native']['lock_check_ok'] = all(line.endswith(b': OK') for line in read('candidate-unit-v68/lock-after.log').splitlines())
    data['native']['lock_sha256'] = read('candidate-build-v68/lock-before.sha256').decode().split()[0]
    data['native']['engine_version'] = read('candidate-build-v68/engine-version.txt').decode().strip()
    data['root_analysis'] = load('home-empty-fast-v68-validation.json')
    save('analysis/root-analysis.py.txt', safe_text(read('validate-home-empty-fast-v68.py')).encode())
    jsave('analysis/root-output.json', data['root_analysis'])
    # Preserve the exact strict oracle source as historical text, never import its execution driver.
    bench = Path(__file__).resolve().parents[2]
    for name in ('resident_state_matrix.py', 'resident_engine_compare.py', 'resident_workload_compare.py', 'blaze_runtime_compare.py', 'blaze_edit_compare.py'):
        raw = (bench / name).read_bytes()
        provenance['repository/bench/' + name] = {'sha256': sha(raw), 'bytes': len(raw)}
        save('oracles/' + name + '.txt', safe_text(raw).encode())
    save('oracles/runtime-driver.py.txt', safe_text(read('probe-home-empty-fast-v68.py')).encode())
    save('oracles/native-gate-v68.sh.txt', safe_text(read('build-test-candidate-v68.sh')).encode())
    cleanup = load('completed-home-empty-fast-v68-stop/receipt.json')
    data['cleanup'] = {'stores_deleted': cleanup['stores_deleted'],
        'rows': [{k: row[k] for k in ('case', 'post_measurement_only', 'status', 'error', 'timed_out')}
                 for row in cleanup['rows']]}
    data['restoration'] = {'home_sha256': sha((output / 'frozen/baseline/home.rs').read_bytes()),
        'manifest_sha256': sha((output / 'frozen/baseline/Cargo.toml').read_bytes()),
        'source_cmp_original': True, 'source_head_diff_empty': True,
        'witness': 'worker actual exact backup cmp and scoped HEAD diff after write restoration',
        'lock_owner': 'root, separate restoration', 'time_utc': '2026-10-31T12:11:00Z',
        'original_lock_sha256': sha(read('home-empty-fast-v68-root-integration/Cargo.lock.original'))}
    jsave('checks/restoration.json', data['restoration'])
    data['lock_integration'] = {'package': 'hir-ty-codegen', 'added_existing_edges': ['ide-db', 'test-fixture'],
                              'new_packages': 0, 'source_lane_lock_edits': 0}
    data['privacy'] = {'environment_allowlist': ENV, 'control_allowlist': list(CONTROLS),
        'method': 'fixed-field selection and full-line raw-output allowlists, no environment redaction',
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
