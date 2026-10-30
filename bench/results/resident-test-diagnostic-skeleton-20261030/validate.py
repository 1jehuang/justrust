#!/usr/bin/env python3
"""Portable offline evidence admission. Never runs native tools or copied commands."""
import hashlib
import itertools
import json
import math
import re
import statistics
import sys
from pathlib import Path

SOURCE_SHA = {
    'outline.rs': 'f20477d485c0e7272d2986d410931439cb62fe72a105bbddbab7f8fae99e9362',
    'main.rs': 'a76b2fb21419ff9f7dfdf173dbc1c4be64111566220f2b02d7dd282bf9ae510e',
}
EXPECTED_NAMES = sorted('summary::tests::' + name for name in (
    'parses_libtest_results', 'shares_overlapping_time',
    'splits_passes_without_double_counting', 'test_failures_are_not_compile_errors'))
FOCUSED = {'resident_diagnostic_skeleton': 8, 'outline::tests': 21,
           'diagnostic_hash::tests': 2, 'diagnostics_timing_tests': 3,
           'resident_graph_reuse': 1, 'the_roots_of_a_listed_crate': 1, 'telemetry': 3}


def require(ok, message):
    if not ok:
        raise ValueError(message)


def digest(path):
    return hashlib.sha256(path.read_bytes()).hexdigest()


def safe(root, name):
    path = Path(name)
    require(not path.is_absolute() and '..' not in path.parts and path.parts, 'unsafe evidence path')
    target = root / path
    require(not any(p.is_symlink() for p in [target, *target.parents] if p != root.parent), 'symlink evidence')
    require(target.resolve().is_relative_to(root.resolve()), 'escaping evidence path')
    return target


def seal(root):
    rows = []
    for path in sorted(root.rglob('*')):
        if '__pycache__' in path.parts:
            continue
        require(not path.is_symlink(), 'symlink evidence')
        if path.is_file() and path.relative_to(root).as_posix() not in ('SHA256SUMS', 'validation.json'):
            rows.append(f'{digest(path)}  {path.relative_to(root).as_posix()}\n')
    (root / 'SHA256SUMS').write_text(''.join(rows))


def parse_tests(text):
    names = re.findall(r'^test (\S+) \.\.\. (?:JR_RUNTIME_EXECUTED_\d+\n)?(ok|FAILED)(?: <[0-9.]+s>)?$', text, re.M)
    totals = re.findall(r'test result: (?:ok|FAILED)\. (\d+) passed; (\d+) failed; (\d+) ignored; \d+ measured; (\d+) filtered out;', text)
    if not totals:
        return None
    require(len(totals) == 1, 'duplicate runtime result')
    passed, failed, ignored, filtered = map(int, totals[0])
    return dict(tests=[list(x) for x in sorted(names)], passed=passed, failed=failed,
                ignored=ignored, filtered=filtered)


def runtime(row, text, marker_base=1):
    phase = row['phase']
    require(row['error'] is None and not row['timed_out'], 'runtime error/timeout')
    require(math.isfinite(row['elapsed_seconds']) and row['elapsed_seconds'] > 0, 'invalid duration')
    require(not any('DIAGNOSTICS_STATS' in k for k in row['environment']), 'runtime telemetry enabled')
    if phase == 'version':
        require(row['status'] == 0, 'version failed')
        return
    expected_status = 101 if phase in ('type-error', 'assert-failure') else 0
    require(row['status'] == expected_status, 'runtime status mismatch')
    parsed = parse_tests(text)
    require(parsed == row['parsed'], 'raw runtime result mismatch')
    if phase == 'type-error':
        require(parsed is None and 'E0308' in text and 'JR_RUNTIME_TYPE_ERROR' in text, 'type error oracle missing')
    else:
        require(parsed is not None and sorted(n for n, _ in parsed['tests']) == EXPECTED_NAMES, 'exact four runtime tests missing')
        failure = phase == 'assert-failure'
        require((parsed['passed'], parsed['failed'], parsed['ignored'], parsed['filtered']) ==
                ((3, 1, 0, 144) if failure else (4, 0, 0, 144)), 'runtime verdict mismatch')
        require(sum(s == 'FAILED' for _, s in parsed['tests']) == int(failure), 'individual runtime verdict mismatch')
        if failure:
            require('JR_RUNTIME_FAILURE' in text, 'assertion oracle missing')
            require([n for n, status in parsed['tests'] if status == 'FAILED'] ==
                    ['summary::tests::shares_overlapping_time'], 'wrong failing test')
        if phase == 'edit':
            require(re.findall('JR_RUNTIME_EXECUTED_\\d+', text) ==
                    [f"JR_RUNTIME_EXECUTED_{marker_base + row['replicate']}"], 'runtime edit marker mismatch')


def validate(root):
    root = root.resolve()
    def read(name):
        return json.loads(safe(root, name).read_text())
    def text(name):
        return safe(root, name).read_text()
    entries = {}
    for line in text('SHA256SUMS').splitlines():
        sha, name = line.split('  ', 1)
        require(re.fullmatch('[a-f0-9]{64}', sha) and name not in entries, 'invalid/duplicate manifest')
        entries[name] = sha
        require(digest(safe(root, name)) == sha, f'hash mismatch: {name}')
    actual = set()
    for p in root.rglob('*'):
        require(not p.is_symlink(), 'symlink evidence')
        if p.is_file() and '__pycache__' not in p.parts and p.relative_to(root).as_posix() not in ('SHA256SUMS', 'validation.json'):
            actual.add(p.relative_to(root).as_posix())
    require(actual == set(entries), 'manifest coverage mismatch')
    for version in ('v27', 'v28'):
        for gate in ('check-exit', 'compile-exit'):
            require(text(f'build/{version}/{gate}').strip() == '0', 'failed build gate')
        frozen = {}
        for line in text(f'build/{version}/source-before.sha256').splitlines():
            sha, name = line.split('  ', 1)
            frozen[name] = sha
            require(digest(safe(root, f'build/{version}/frozen-source/{name}')) == sha, 'compiled source mismatch')
        require(len(frozen) == 17, 'compiled source coverage mismatch')
        require(sorted(text(f'build/{version}/source-after.log').splitlines()) ==
                sorted(f'{name}: OK' for name in frozen), 'post-build source check mismatch')
        for name in ('outline.rs', 'main.rs'):
            relative = f'toolchain/resident/crates/ra-jit/src/{name}'
            candidate = safe(root, f'source-ready/{version}/candidate/{relative}')
            require(digest(candidate) == frozen[f'crates/ra-jit/src/{name}'], 'source readiness mismatch')
            if version == 'v28':
                require(digest(candidate) == SOURCE_SHA[name], 'accepted source identity mismatch')
            require(candidate.read_bytes() == safe(root, f'source-ready/v28/originals/{relative}').read_bytes()
                    if version == 'v27' else True, 'v27 original continuity mismatch')
    # The repair changes fixture setup only: production prefixes are byte-identical.
    for name in ('outline.rs', 'main.rs'):
        prefix = f'build/{{}}/frozen-source/crates/ra-jit/src/{name}'
        before, after = (text(prefix.format(v)).split('#[cfg(test)]')[0] for v in ('v27', 'v28'))
        require(before == after, 'v28 production delta beyond fixture repair')
    journals = sorted((root / 'unit/v28/raw-runs').glob('*/summary.json'))
    require(len(journals) == 8, 'native journal coverage mismatch')
    observed_gates = []
    for path in journals:
        summary = json.loads(path.read_text())
        filters = [f for f in FOCUSED if f in summary['args']]
        require(len(filters) <= 1, 'ambiguous native gate')
        observed_gates.append(filters[0] if filters else 'full-native')
        expected = FOCUSED[filters[0]] if filters else 359
        require(summary['exit'] == 0 and summary['tests']['passed'] == expected and
                summary['tests']['failed'] == summary['tests']['ignored'] == 0, 'native gate count mismatch')
        raw = '\n'.join(json.loads(line)['l'] for line in path.with_name('output.jsonl').read_text().splitlines())
        require(f'test result: ok. {expected} passed; 0 failed' in raw, 'raw native verdict missing')
        require(len(re.findall(r'^test .* \.\.\. ok(?: <[0-9.]+s>)?$', raw, re.M)) == expected, 'raw individual native count mismatch')
        names = re.findall(r'^test (\S+) \.\.\. ok(?: <[0-9.]+s>)?$', raw, re.M)
        require(len(set(names)) == expected, 'duplicate raw native tests')
    require(sorted(observed_gates) == sorted([*FOCUSED, 'full-native']), 'native filter coverage mismatch')
    for gate in (*FOCUSED, 'full-native'):
        require(text(f'unit/v28/{gate}.exit').strip() == '0', 'failed native gate')
    for name in ('build/v28/orchestration-exit', 'unit/v28/exit',
                 'root/suite-v27-baseline.exit', 'root/suite-v28-candidate.exit',
                 'root/suite-v28-repeat.exit', 'root/suite-v28-repeat-baseline.exit',
                 'root/suite-v28-repeat-candidate.exit'):
        require(text(name).strip() == '0', 'root gate failed')
    require(text('unit/v27/resident_diagnostic_skeleton.exit').strip() == '101' and
            text('build/v27/orchestration-exit').strip() == '101', 'v27 failed status lost')
    failed = read('unit/v27/raw-runs/20261030-142404178-4153131/summary.json')
    require(failed['exit'] == 101 and failed['tests']['passed'] == 3 and failed['tests']['failed'] == 5, 'v27 failure lost')
    failed_raw = '\n'.join(json.loads(line)['l'] for line in text('unit/v27/raw-runs/20261030-142404178-4153131/output.jsonl').splitlines())
    require('test result: FAILED. 3 passed; 5 failed' in failed_raw and 'no entry found for key' in failed_raw, 'raw v27 failure missing')
    require('Ran 700 tests in 9.624s' in text('root/bench-v28-regression.log') and
            text('root/bench-v28-regression.log').rstrip().endswith('OK'), 'benchmark regression gate missing')
    admission = read('root/candidate-admission-v28.json')
    require(admission['verdict'] == 'passed' and [r['passed'] for r in admission['rows']] == [8, 359], 'native admission mismatch')
    engine_shas = {s: text(f'root/old-engine/engine-identity.sha256').split()[0] if s == 'old'
                   else text('build/v28/engine-identity.sha256').split()[0] for s in ('old', 'new')}
    sessions = {}
    base_source = None
    source_edits = {}
    for name in ('rsb18', 'rsa24', 'rsa25', 'rsb19'):
        base = f'suites/{name}'
        metadata, suite, rows = (read(f'{base}/{f}.json') for f in ('metadata', 'suite', 'records'))
        require(metadata['status'] == 'complete' and metadata['unchanged'] and metadata['source_restored'] and metadata['toolchain_links_valid'], 'suite incomplete/dirty')
        require(metadata['source_before'] == metadata['source_after'] and metadata['tools_before'] == metadata['tools_after'] and
                metadata['toolchain_before'] == metadata['toolchain_after'], 'suite identity changed')
        if base_source is None:
            base_source = metadata['source_before']
        require(base_source == metadata['source_before'], 'suite source differs')
        expected_engine = engine_shas['new' if name in ('rsa24', 'rsa25') else 'old']
        require(metadata['tools_before']['engine']['sha256'] == expected_engine, 'suite engine identity mismatch')
        require(suite['metadata'] == metadata, 'suite metadata mismatch')
        for artifact, sha in suite['artifacts_sha256'].items():
            require(digest(safe(root, f'{base}/{artifact}')) == sha, 'suite artifact mismatch')
        require(len(rows) == 22, 'suite row coverage mismatch')
        for stack in ('engine', 'reference'):
            selected = [r for r in rows if r['stack'] == stack]
            require([(r['phase'], r['replicate']) for r in selected] ==
                    [(p, 0) for p in ('version', 'prepare', 'type-error', 'type-recovery', 'assert-failure', 'assert-recovery')] +
                    [('edit', i) for i in range(5)], 'suite phase coverage mismatch')
            for row in selected:
                prefix = f"{base}/{stack}-{row['phase']}-{row['replicate']}"
                output = text(prefix + '.stdout') + '\n' + text(prefix + '.stderr')
                runtime(row, output)
                edit = (row['phase'], row['replicate'])
                require(row['source_sha256'] == source_edits.setdefault(edit, row['source_sha256']), 'suite edits differ across sessions/stacks')
            samples = [r['elapsed_seconds'] for r in selected if r['phase'] == 'edit']
            require(samples == suite['measurements'][stack]['samples_seconds'] and
                    statistics.median(samples) == suite['measurements'][stack]['median_seconds'], 'suite median mismatch')
        sessions[name] = suite
    for direction in ('forward', 'reverse'):
        base = f'warm/{direction}'
        state = read(f'{base}/state.json')
        require(state['status'] == 'passed' and state['source_restored'] and state['identities_unchanged'] and state['toolchain_unchanged'], 'warm admission failed')
        require(state['source_before'] == state['source_after'] == base_source and state['toolchain_before'] == state['toolchain_after'], 'warm identity changed')
        require(len(state['rows']) == 141, 'warm row coverage mismatch')
        stack_order = ('old', 'new', 'blaze') if direction == 'forward' else ('blaze', 'new', 'old')
        phases = ('prepare', 'type-error', 'type-recovery', 'assert-failure', 'assert-recovery')
        permutations = list(itertools.permutations(stack_order))
        require([(r['phase'], r['replicate'], r['stack']) for r in state['rows']] ==
                [(p, 0, s) for s in stack_order for p in phases] +
                [('edit', i, s) for i in range(42) for s in permutations[i % 6]],
                'warm exact phase/permutation order mismatch')
        for stack in ('old', 'new', 'blaze'):
            require(state['configs'][stack]['sha256'] == (engine_shas[stack] if stack != 'blaze' else
                    sessions['rsa24']['metadata']['tools_before']['reference']['sha256']), 'warm tool identity mismatch')
            rows = [r for r in state['rows'] if r['stack'] == stack]
            require(len(rows) == 47 and sorted(r['replicate'] for r in rows if r['phase'] == 'edit') == list(range(42)), 'warm edit coverage mismatch')
            require({r['phase'] for r in rows} == {'prepare', 'type-error', 'type-recovery', 'assert-failure', 'assert-recovery', 'edit'}, 'warm phase coverage mismatch')
            for row in rows:
                prefix = f"{base}/{row['phase']}-{row['replicate']}-{stack}"
                require(row['accepted'], 'warm row rejected')
                require(digest(safe(root, prefix + '.stdout')) == row['stdout_sha256'] and
                        digest(safe(root, prefix + '.stderr')) == row['stderr_sha256'], 'warm raw hash mismatch')
                runtime(row, text(prefix + '.stdout') + '\n' + text(prefix + '.stderr'),
                        2801 if direction == 'forward' else 2901)
            require(statistics.median(r['elapsed_seconds'] for r in rows if r['phase'] == 'edit') == state['medians_seconds'][stack], 'warm median mismatch')
        for rep in range(42):
            edits = [r for r in state['rows'] if r['phase'] == 'edit' and r['replicate'] == rep]
            require(len({r['edit_sha256'] for r in edits}) == 1, 'warm edits differ across stacks')
    state = read('profile/state.json')
    require(state['status'] == 'passed' and state['unchanged'] and len(state['rows']) == 12, 'profile admission failed')
    require(state['source_before'] == state['source_after'] == base_source and state['toolchain_before'] == state['toolchain_after'], 'profile identity changed')
    require([(r['replicate'], r['stack']) for r in state['rows']] == [(i, s) for i in range(6) for s in ('old', 'new')], 'profile row coverage mismatch')
    for stack in ('old', 'new'):
        require(state['configs'][stack]['sha256'] == state['tools_after'][stack] == engine_shas[stack], 'profile tool identity mismatch')
    positive_labels = set()
    for row in state['rows']:
        rep, stack = row['replicate'], row['stack']
        prefix = f'profile/{rep}-{stack}'
        require(row['status'] == 0 and not row['timed_out'] and row['error'] is None, 'profile runtime failed')
        output = text(prefix + '.stdout') + '\n' + text(prefix + '.stderr')
        parsed = parse_tests(output)
        require(row['marker'] == f'JR_RUNTIME_EXECUTED_{rep + 3001}' and
                re.findall('JR_RUNTIME_EXECUTED_\\d+', output) == [row['marker']], 'profile exact marker mismatch')
        require(parsed is not None and parsed['passed'] == 4 and parsed['failed'] == 0 and sorted(n for n, _ in parsed['tests']) == EXPECTED_NAMES and output.count(row['marker']) == 1, 'profile runtime oracle missing')
        require(digest(safe(root, prefix + '.trace.json')) == row['trace_sha256'] and
                digest(safe(root, prefix + '.worker-delta.log')) == row['worker_delta_sha256'], 'profile raw hash mismatch')
        delta = text(prefix + '.worker-delta.log')
        inventory = [dict(label=label, calls=int(calls), cumulative_ns=int(ns)) for label, calls, ns in re.findall(
            r'diagnostics collector: label=(hir\.collect\.[a-z_]+) calls=(\d+) cumulative_ns=(\d+) inclusive=true', delta)]
        require(inventory == row['collector_inventory'], 'raw inventory mismatch')
        groups = []
        for counter in inventory:
            if counter['label'] == 'hir.collect.module_inclusive':
                groups.append([])
            require(bool(groups), 'missing inventory group header')
            groups[-1].append(counter)
        expected = [373 if stack == 'old' else 1] + ([231] if rep == 0 else [])
        require([c['calls'] for c in inventory if c['label'] == 'hir.collect.selected_body_inclusive'] == expected, 'selected body proof mismatch')
        for group in groups:
            require(0 < len(group) <= 16 and len({c['label'] for c in group}) == len(group), 'unbounded/duplicate inventory')
            require(all(c['calls'] > 0 and c['cumulative_ns'] > 0 for c in group), 'nonpositive inventory')
        for field, pattern in (
            ('callbacks', r'asked_callback_calls=(\d+) asked_callback_cumulative_ns=(\d+)'),
            ('transport', r'diagnostics transport: retained=(\d+) stale=(\d+) dropped=(\d+) cap=4000'),
        ):
            require([[int(v) for v in match] for match in re.findall(pattern, delta)] == row[field] and row[field], 'raw callback/transport mismatch')
        require(all(drop == 0 for _, _, drop in row['transport']), 'dropped profile spans')
        trace = read(prefix + '.trace.json')['traceEvents']
        events = [e for e in trace if e.get('ph') == 'X' and str(e.get('name', '')).startswith('diagnostics.')]
        require(events == row['events'], 'raw profile event mismatch')
        require(not any(str(e.get('name', '')).startswith('hir.collect.') for e in trace), 'synthetic collector event')
        outer = ('diagnostics.skeleton_key_clean', 'diagnostics.semantic_inclusive_asked',
                 'diagnostics.filter_cache_update', 'diagnostics.broken_in.wall')
        semantic = tuple('diagnostics.semantic.' + name for name in
                         ('prepare_parse', 'syntax_walk', 'module_context', 'collect', 'render_and_filter'))
        require(set(outer + semantic).issubset({e['name'] for e in events}), 'positive phase missing')
        for event in events:
            require(all(isinstance(event.get(k), (int, float)) and math.isfinite(event[k])
                        for k in ('ts', 'dur')) and event['dur'] >= 0, 'invalid profile event duration')
            if event['dur'] > 0:
                positive_labels.add(event['name'])
            require('pid' in event and 'tid' in event, 'missing profile event thread')
        def contains(parent, child):
            return parent['ts'] <= child['ts'] and child['ts'] + child['dur'] <= parent['ts'] + parent['dur']
        walls = [e for e in events if e['name'] == 'diagnostics.broken_in.wall']
        for event in events:
            require(any(contains(wall, event) for wall in walls), 'profile wall containment mismatch')
            if event['name'] in semantic:
                require(any(parent['name'] == 'diagnostics.semantic_inclusive_asked' and
                            parent['pid'] == event['pid'] and parent['tid'] == event['tid'] and contains(parent, event)
                            for parent in events), 'profile semantic thread containment mismatch')
    require(set(outer + semantic).issubset(positive_labels), 'profile label never positive')
    return dict(status='passed', source_commit='fe8df07f3', files=len(entries),
                native_tests=359, new_native_tests=8, retained_v27_failed_tests=5,
                regression_tests=700, suite_sessions=4, suite_rows=88,
                warm_runs=2, warm_rows=282, profile_rows=12,
                scope='Scoped resident test edit observations only, unknown noise, no general/default promotion')


if __name__ == '__main__':
    directory = Path(sys.argv[1]).resolve() if len(sys.argv) > 1 else Path(__file__).resolve().parent
    try:
        if '--seal' in sys.argv:
            seal(directory)
        else:
            print(json.dumps(validate(directory), sort_keys=True))
    except (ValueError, KeyError, OSError, TypeError, IndexError) as error:
        print(json.dumps(dict(status='failed', error=str(error))))
        raise SystemExit(1)
