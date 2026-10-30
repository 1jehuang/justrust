#!/usr/bin/env python3
"""Portable stdlib-only raw evidence audit. Never runs native code or source runners."""
import argparse
import hashlib
import itertools
import json
import re
import statistics
from pathlib import Path

SOURCE_COMMIT = 'bea4cc1dc29dbe65b1c2073c92e43f79e4d735a2'
HELPER_SHA = '9a9d2dab6718dd7e84c675f72edda5832e9b7ebc7d422f9c5c11e95f9fcb8f06'
MAIN_SHA = '87345acf075e0502e69d6e5a5ed4f86b9b7fdf1bacd2b834ad70ec47da5ae67d'
TOOLS = {'old': '3c79ef300c79d1d98fe9f58136e4b67778a6263252d271b9b36a501d31fea6bd',
         'new': '31e916f280feb2e79bea42e1ef3a03b68ca16df09e361f437459b10be8b03219'}
NAMES = {'summary::tests::' + name for name in (
    'shares_overlapping_time', 'splits_passes_without_double_counting',
    'parses_libtest_results', 'test_failures_are_not_compile_errors')}
LABELS = {'diagnostics.broken_in.wall', 'diagnostics.skeleton_key_clean',
          'diagnostics.semantic_inclusive_asked', 'diagnostics.filter_cache_update'}
RESULT = re.compile(r'test result: (ok|FAILED)\. (\d+) passed; (\d+) failed; (\d+) ignored;')
CALLBACK = r'asked_callback_calls=(\d+) asked_callback_cumulative_ns=(\d+)'
TRANSPORT = r'diagnostics transport: retained=(\d+) stale=(\d+) dropped=(\d+) cap=4000'


def require(ok, message):
    if not ok:
        raise ValueError(message)


def local(root, name):
    path = root / name
    require(not Path(name).is_absolute() and '..' not in Path(name).parts,
            f'nonlocal path: {name}')
    require(not path.is_symlink() and path.resolve().is_relative_to(root.resolve()),
            f'escaping artifact: {name}')
    return path


def digest(path):
    return hashlib.sha256(path.read_bytes()).hexdigest()


def load(root, name):
    return json.loads(local(root, name).read_text())


def successful(row):
    require(row['status'] == 0 and not row['error'] and not row['timed_out'], 'process failed')


def raw_text(root, prefix):
    return '\n'.join(local(root, prefix + '.' + suffix).read_text() for suffix in ('stdout', 'stderr'))


def runtime(text, failing=False):
    results = RESULT.findall(text)
    expected = [('FAILED', '3', '1', '0')] if failing else [('ok', '4', '0', '0')]
    require(results == expected, 'raw runtime test counts mismatch')
    tests = re.findall(r'^test (summary::tests::\w+) \.\.\. (?:JR_RUNTIME_EXECUTED_\d+\n)?(ok|FAILED)', text, re.M)
    require(len(tests) == 4 and {name for name, _ in tests} == NAMES, 'raw runtime test names mismatch')
    require(sum(verdict == 'FAILED' for _, verdict in tests) == int(failing), 'raw test verdicts mismatch')


def check_phase(root, folder, row, offset, flipped=False):
    phase, rep, stack = row['phase'], row['replicate'], row['stack']
    prefix = f'{folder}/{stack}-{phase}-{rep}' if flipped else f'{folder}/{phase}-{rep}-{stack}'
    text = raw_text(root, prefix)
    for suffix in ('stdout', 'stderr'):
        if suffix + '_sha256' in row:
            require(digest(local(root, prefix + '.' + suffix)) == row[suffix + '_sha256'],
                    'raw stream recorded hash mismatch')
    require(not row['error'] and not row['timed_out'], 'runtime process error')
    if phase in ('type-error', 'assert-failure'):
        require(row['status'] == 101, 'expected failure did not exit101')
        if phase == 'type-error':
            require('E0308' in text and 'JR_RUNTIME_TYPE_ERROR' in text, 'raw type error missing')
        else:
            runtime(text, failing=True)
            require('JR_RUNTIME_FAILURE' in text, 'raw assertion failure missing')
    else:
        successful(row)
        if phase != 'version':
            runtime(text)
        if phase == 'edit':
            require(text.count(f'JR_RUNTIME_EXECUTED_{offset + rep + 1}') == 1,
                    'missing/duplicate exact raw edit marker')
            require(row.get('accepted'), 'edit not accepted')
    return text


def check_identity(state):
    require(state['source_before'] == state['source_after'], 'source restoration mismatch')
    require(state['toolchain_before'] == state['toolchain_after'], 'toolchain restoration mismatch')


def native(root, version, run, count):
    prefix = f'journals/{version}/{run}'
    summary = load(root, prefix + '/summary.json')
    require(summary['id'] == run and summary['exit'] == 0, 'native journal nonzero or wrong id')
    text = '\n'.join(json.loads(line)['l'] for line in local(root, prefix + '/output.jsonl').read_text().splitlines())
    require(RESULT.findall(text) == [('ok', str(count), '0', '0')], 'raw native libtest counts mismatch')
    passed = re.findall(r'^test (\S+) \.\.\. ok(?:\s|$)', text, re.M)
    require(len(passed) == count and len(set(passed)) == count, 'raw native passing-name coverage mismatch')
    return passed


def check_sources(root):
    snapshots = {}
    for version, size in (('v22', 11), ('v23', 12)):
        lines = local(root, f'build/{version}/source-before.sha256').read_text().splitlines()
        require(len(lines) == size, 'selected source manifest size changed')
        snapshots[version] = {}
        for line in lines:
            sha, name = line.split(maxsplit=1)
            require(digest(local(root, f'build/{version}/frozen-source/{name}')) == sha,
                    f'frozen source SHA mismatch: {version}/{name}')
            snapshots[version][name] = sha
        after = local(root, f'build/{version}/source-after.log').read_text().splitlines()
        require(len(after) == size and all(line.endswith(': OK') for line in after), 'build source boundary failed')
        identity = local(root, f'build/{version}/engine-identity.sha256').read_text().split()[0]
        require(identity == TOOLS['old' if version == 'v22' else 'new'], 'binary identity receipt mismatch')
    a, b = snapshots['v22'], snapshots['v23']
    helper = 'crates/ra-jit/src/diagnostic_hash.rs'
    main = 'crates/ra-jit/src/main.rs'
    require(set(b) - set(a) == {helper} and not set(a) - set(b), 'unexpected new frozen source path')
    require([name for name in a if a[name] != b[name]] == [main], 'unexpected shared source change')
    require(b[helper] == HELPER_SHA and b[main] == MAIN_SHA, 'wrong accepted sources')
    require(digest(local(root, 'source-ready/diagnostic_hash.rs')) == HELPER_SHA, 'source-ready helper mismatch')
    receipt = local(root, 'source-ready/source-ready.txt').read_text()
    require(digest(local(root, 'source-ready/diagnostic_hash.pre-refinement.rs')) in receipt
            and 'cmp identical' in receipt and '19:19:15.481Z' in receipt, 'incident receipt missing')


def validate(root, sealed=True):
    root = root.resolve()
    collection = load(root, 'collection.json')
    require(collection['source_commit'] == SOURCE_COMMIT, 'source commit mismatch')
    for item in collection['files']:
        require(digest(local(root, item['artifact'])) == item['sha256'], 'collected raw bytes changed')
    if sealed:
        manifest = load(root, 'manifest.json')
        files = {str(p.relative_to(root)) for p in root.rglob('*') if p.is_file()
                 and '__pycache__' not in p.parts and p.name != 'manifest.json'}
        require(files == set(manifest['files']), 'sealed inventory changed')
        for name, sha in manifest['files'].items():
            require(digest(local(root, name)) == sha, f'sealed hash mismatch: {name}')
    check_sources(root)
    admissions = load(root, 'native/v23/positive-test-admission.json')
    expected = {'diagnostic_hash::tests': 2, 'diagnostics_timing_tests': 3,
                'resident_graph_reuse': 1, 'the_roots_of_a_listed_crate': 1, 'telemetry': 3}
    require(admissions['verdict'] == 'passed', 'native admission failed')
    require({r['filter']: r['passed'] for r in admissions['native_test_rows']} == expected, 'admission counts changed')
    for row in admissions['native_test_rows']:
        native(root, 'v23', row['run_id'], row['passed'])
    native(root, 'v23', '20261030-123822295-3023351', 351)
    restored = list(local(root, 'journals/v22-restored').iterdir())
    require(len(restored) == 1, 'restored baseline journal ambiguous')
    native(root, 'v22-restored', restored[0].name, 349)
    regression = local(root, 'provenance/bench-regression-v23.log').read_text()
    require(re.search(r'Ran 700 tests in [\d.]+s\s+OK\s*$', regression), 'bench regression raw count not700')
    require(local(root, 'provenance/bench-regression-v23.exit').read_text().strip() == '0', 'bench regression exit nonzero')
    medians = {}
    fixed_meta = {}
    for name, kind in (('rsb14', 'old'), ('rsa20', 'new')):
        folder = f'fixed-five/{name}'
        meta = load(root, folder + '/metadata.json')
        fixed_meta[kind] = meta
        require(meta['status'] == 'complete' and meta['source_restored'] and meta['unchanged'], 'fixed-five incomplete')
        check_identity(meta)
        require(meta['tools_before'] == meta['tools_after'], 'fixed-five binary changed')
        require(meta['tools_before']['engine']['sha256'] == TOOLS[kind], 'fixed-five binary not frozen build')
        rows = load(root, folder + '/records.json')
        require(len(rows) == 22, 'fixed-five row count not22')
        for row in rows:
            check_phase(root, folder, row, 0, flipped=True)
        medians[kind] = {stack: statistics.median(row['elapsed_seconds'] for row in rows
                         if row['phase'] == 'edit' and row['stack'] == stack)
                         for stack in ('engine', 'reference')}
        require(all(sum(row['phase'] == 'edit' and row['stack'] == stack for row in rows) == 5
                    for stack in ('engine', 'reference')), 'fixed-five sample count changed')
    require(fixed_meta['old']['source_before'] == fixed_meta['new']['source_before'], 'fixed-five sources unmatched')
    extended = {}
    for folder, offset, order in (('interleaved', 800, ['old', 'new', 'blaze']),
                                   ('interleaved-reverse', 1000, ['blaze', 'new', 'old'])):
        state = load(root, folder + '/state.json')
        check_identity(state)
        require(state['status'] == 'passed' and state['source_restored'] and state['identities_unchanged']
                and state['toolchain_unchanged'], 'interleaved identity/pass gate failed')
        require(len(state['rows']) == 141, 'interleaved row count not141')
        require(state['source_before'] == fixed_meta['old']['source_before'], 'interleaved source not fixed-five fixture')
        for key in ('old', 'new'):
            require(state['configs'][key]['sha256'] == TOOLS[key], 'interleaved wrong binary')
        require(state['configs']['blaze']['sha256'] == fixed_meta['new']['tools_before']['reference']['sha256'], 'wrong reference binary')
        for row in state['rows']:
            check_phase(root, folder, row, offset)
        permutations = list(itertools.permutations(order))
        for rep in range(42):
            rows = [row for row in state['rows'] if row['phase'] == 'edit' and row['replicate'] == rep]
            require([row['stack'] for row in rows] == list(permutations[rep % 6]), 'interleaved order changed')
            require(len({row['edit_sha256'] for row in rows}) == 1, 'interleaved edit bytes unmatched')
        extended[folder] = {stack: statistics.median(row['elapsed_seconds'] for row in state['rows']
                           if row['phase'] == 'edit' and row['stack'] == stack) for stack in order}
        require(extended[folder] == state['medians_seconds'], 'recorded interleaved medians not raw-derived')
    state = load(root, 'profile-calibrated/state.json')
    check_identity(state)
    require(state['status'] == 'passed' and state['unchanged'] and len(state['rows']) == 16, 'calibrated profile not16passes')
    require(state['source_before'] == fixed_meta['old']['source_before'], 'profile source not fixture')
    require(len(state['prepares']) == 2, 'calibration prepares missing')
    for name in ('old', 'new'):
        require(state['configs'][name]['sha256'] == state['tools_after'][name] == TOOLS[name], 'profile binary identity changed')
        successful(state['prepares'][0 if name == 'old' else 1])
        runtime(raw_text(root, f'profile-calibrated/{name}-prepare'))
    selected = {'old': [], 'new': []}
    transports = []
    for rep in range(8):
        rows = state['rows'][rep * 2:rep * 2 + 2]
        require([row['stack'] for row in rows] == (['old', 'new'] if rep % 2 == 0 else ['new', 'old']), 'profile alternating order changed')
        require(len({row['edit_sha256'] for row in rows}) == 1, 'profile edit bytes unmatched')
        for row in rows:
            successful(row)
            name = row['stack']
            prefix = f'profile-calibrated/{rep}-{name}'
            text = raw_text(root, prefix)
            runtime(text)
            marker = f'JR_RUNTIME_EXECUTED_{rep + 1401}'
            require(row['replicate'] == rep and row['marker'] == marker and text.count(marker) == 1, 'profile exact raw marker mismatch')
            trace_path = local(root, prefix + '.trace.json')
            require(digest(trace_path) == row['trace_sha256'], 'profile trace recorded SHA mismatch')
            trace = json.loads(trace_path.read_text())
            events = [e for e in trace['traceEvents'] if e.get('ph') == 'X' and str(e.get('name', '')).startswith('diagnostics.')]
            require(events == row['events'] and {e['name'] for e in events} == LABELS, 'raw actual diagnostic events missing')
            require(all(e['dur'] >= 0 and e['pid'] > 0 and e['tid'] > 0 for e in events), 'invalid trace attribution')
            worker_path = local(root, prefix + '.worker-delta.log')
            require(digest(worker_path) == row['worker_delta_sha256'], 'profile worker SHA mismatch')
            worker = worker_path.read_text()
            callbacks = [list(map(int, x)) for x in re.findall(CALLBACK, worker)]
            transport = [list(map(int, x)) for x in re.findall(TRANSPORT, worker)]
            require(callbacks == row['callbacks'] and transport == row['transport'], 'raw callback/transport receipts differ')
            matched = [ns for calls, ns in callbacks if calls == 373]
            require(len(matched) == 1 and all(dropped == 0 for _, _, dropped in transport) and transport, 'selected callback missing or dropped transport')
            require([calls for calls, _ in callbacks] == ([373, 231] if rep == 0 else [373]), 'unexpected extra-file callback coverage')
            require(transport == ([[7, 94, 0]] if rep == 0 else [[4, 0, 0]]), 'actual transport receipts changed')
            selected[name].extend(matched)
            transports.extend(transport)
    require((min(selected['old']), max(selected['old'])) == (7627533, 9105572), 'old callback raw range changed')
    require((min(selected['new']), max(selected['new'])) == (326941, 506480), 'new callback raw range changed')
    failed = load(root, 'profile-failed/state.json')
    check_identity(failed)
    require(failed['status'] == 'failed' and failed['unchanged'] and failed['rows'] == []
            and failed['error'] == "ValueError('missing positive diagnostics spans')", 'original failed capture lost')
    runtime(raw_text(root, 'profile-failed/0-old'))
    trace = load(root, 'profile-failed/0-old.trace.json')
    labels = {e.get('name') for e in trace['traceEvents'] if e.get('ph') == 'X'}
    require(not {'diagnostics.broken_in.wall', 'diagnostics.semantic_inclusive_asked'}.issubset(labels), 'failed trace no longer lacks required spans')
    runner = local(root, 'provenance/profile-flat-hash-v23-calibrated.py').read_text()
    require("env['RA_JIT_DIAGNOSTICS_STATS']='1'" in runner and "state['prepares']=[]" in runner,
            'calibrated startup/preparation provenance missing')
    return dict(verdict='passed', source_commit=SOURCE_COMMIT, native_passed=351, baseline_restored_passed=349,
                benchmark_regression_passed=700, fixed_five_medians_seconds=medians,
                interleaved_medians_seconds=extended, profile_rows=16, selected_callback_ns=selected,
                failed_profile_preserved=True,
                claim='Scoped experimental warm selected-fixture improvement only, not comprehensive/default promotion. Inclusive callbacks are not additive critical-path savings.')


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument('--root', type=Path, default=Path(__file__).resolve().parent)
    args = parser.parse_args()
    print(json.dumps(validate(args.root), indent=2))


if __name__ == '__main__':
    main()
