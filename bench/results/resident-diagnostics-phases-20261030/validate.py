#!/usr/bin/env python3
"""Portable stdlib-only validation of retained evidence, never runs native tools."""
import argparse
import hashlib
import json
import re
from pathlib import Path

LABELS = {'diagnostics.broken_in.wall', 'diagnostics.skeleton_key_clean',
          'diagnostics.semantic_inclusive_asked', 'diagnostics.filter_cache_update'}
MAIN_SHA = '56e2045c4f57a04631bdd206b8141e62e3a706abef0a940a59fea1e2b1df818a'
CORRECTED_RUN = '20261030-122938036-2828152'
TEST_RESULT = re.compile(r'test result: ok\. (\d+) passed; (\d+) failed; (\d+) ignored;')


def digest(path):
    return hashlib.sha256(path.read_bytes()).hexdigest()


def local(root, relative):
    path = root / relative
    if Path(relative).is_absolute() or '..' in Path(relative).parts or path.is_symlink():
        raise ValueError(f'nonlocal artifact: {relative}')
    if not path.resolve().is_relative_to(root.resolve()):
        raise ValueError(f'artifact escapes bundle: {relative}')
    return path


def load(root, relative):
    return json.loads(local(root, relative).read_text())


def require(ok, message):
    if not ok:
        raise ValueError(message)


def raw_result(root, home, run):
    directory = local(root, f'journals/{home}/runs/{run}')
    meta = json.loads((directory / 'summary.json').read_text())
    lines = [json.loads(line)['l'] for line in (directory / 'output.jsonl').read_text().splitlines()]
    results = [TEST_RESULT.search(line) for line in lines]
    results = [tuple(map(int, match.groups())) for match in results if match]
    require(len(results) == 1, f'{run}: expected exactly one positive raw libtest result')
    require(results[0][1:] == (0, 0), f'{run}: failed or ignored native tests')
    require(meta.get('exit') == 0, f'{run}: nonzero native exit')
    # Export paths are bundle-local. Never resolve the original JUSTRUST_HOME.
    exported = []
    for path in local(root, f'exports/{home}').rglob('*.jsonl'):
        exported.extend(json.loads(line) for line in path.read_text().splitlines() if line)
    require(any(item.get('id') == run for item in exported), f'{run}: no bundle-local export')
    return results[0][0]


def validate(root):
    collection = load(root, 'collection.json')
    for item in collection['files']:
        require(digest(local(root, item['artifact'])) == item['sha256'],
                f'copied evidence changed: {item["artifact"]}')
    manifest = load(root, 'manifest.json')
    files = {str(path.relative_to(root)) for path in root.rglob('*') if path.is_file()
             and '__pycache__' not in path.parts and path.name != 'manifest.json'}
    require(files == set(manifest['files']), 'sealed artifact inventory changed')
    for name, sha in manifest['files'].items():
        require(digest(local(root, name)) == sha, f'sealed artifact hash changed: {name}')
    require(digest(local(root, 'frozen/v22/main.rs')) == MAIN_SHA, 'wrong accepted main source')
    require(digest(local(root, 'build/v22/frozen-source/crates/ra-jit/src/main.rs')) == MAIN_SHA,
            'native build source differs from accepted instrumentation')
    for version in ('v21', 'v22'):
        hashes = local(root, f'build/{version}/source-before.sha256').read_text().splitlines()
        require(len(hashes) == 11, f'{version}: expected eleven selected source identities')
        for line in hashes:
            sha, name = line.split(maxsplit=1)
            require(digest(local(root, f'build/{version}/frozen-source/{name}')) == sha,
                    f'{version}: frozen selected source mismatch: {name}')
    admissions = load(root, 'native/v22/positive-test-admission.json')
    expected = {'diagnostics_timing_tests': 3, 'resident_graph_reuse': 1,
                'the_roots_of_a_listed_crate': 1, 'telemetry': 3}
    require(admissions['verdict'] == 'passed', 'focused native admission failed')
    require({row['filter']: row['passed'] for row in admissions['native_test_rows']} == expected,
            'focused native admission counts changed')
    for row in admissions['native_test_rows']:
        require(raw_result(root, 'unit-v22', row['run_id']) == expected[row['filter']],
                f'{row["filter"]}: raw journal count mismatch')
    require(raw_result(root, 'unit-v22-restored', CORRECTED_RUN) == 349,
            'corrected native full pass must have exactly349 tests')
    first_index = [json.loads(line) for line in local(root, 'journals/unit-v22/index.jsonl').read_text().splitlines()]
    first = [row for row in first_index if row['args'] ==
             ['test', '--release', '--locked', '-p', 'ra-jit', '--bin', 'jr-engine', '-j', '6']]
    require(len(first) == 1, 'first full-native pass must remain distinct')
    require(raw_result(root, 'unit-v22', first[0]['id']) == 349, 'first raw full-native count mismatch')
    for boundary in ('before', 'after'):
        lines = local(root, f'native/v22-restored/source-{boundary}.log').read_text().splitlines()
        require(len(lines) == 11 and all(line.endswith(': OK') for line in lines),
                f'corrected {boundary} selected-source identity check failed')
    restored_script = local(root, 'provenance/full-native-v22-restored.sh').read_text()
    require(restored_script.count('test ! -e crates/ra-jit/src/diagnostic_hash.rs') == 2,
            'corrected pass did not check helper absence at both boundaries')
    incident = local(root, 'race/original-source-ready.txt').read_text()
    race_sha = digest(local(root, 'race/diagnostic_hash.pre-refinement.rs'))
    require(race_sha in incident and 'Freeze incident:' in incident,
            'missing original worker-produced race receipt/bytes')
    for name in ('rsb12', 'rsa18', 'rsb13', 'rsa19'):
        meta = load(root, f'suites/{name}/metadata.json')
        require(meta['status'] == 'complete' and meta['source_restored'] and meta['unchanged'],
                f'{name}: incomplete or identity-changing suite')
        require(meta['source_before'] == meta['source_after'], f'{name}: source restoration mismatch')
        require(meta['tools_before'] == meta['tools_after'], f'{name}: tool identity mismatch')
        require(meta['toolchain_before'] == meta['toolchain_after'], f'{name}: toolchain identity mismatch')
        records = load(root, f'suites/{name}/records.json')
        require(len(records) == 22, f'{name}: missing suite rows')
        for row in records:
            stack, phase, rep = row['stack'], row['phase'], row['replicate']
            require(not row['timed_out'] and not row['error'], f'{name}: suite process failed')
            require(row['environment'].get('RA_JIT_DIAGNOSTICS_STATS') != '1',
                    f'{name}: expected disabled instrumentation suite')
            text = '\n'.join(local(root, f'suites/{name}/{stack}-{phase}-{rep}.{suffix}').read_text()
                             for suffix in ('stdout', 'stderr'))
            if phase in ('type-error', 'assert-failure'):
                require(row['status'] != 0, f'{name}: expected failure became success')
                if phase == 'type-error':
                    require('E0308' in text, f'{name}: missing actual type error')
                else:
                    require('test result: FAILED. 3 passed; 1 failed; 0 ignored;' in text,
                            f'{name}: missing actual failing assertion')
            else:
                require(row['status'] == 0, f'{name}: unexpected suite exit')
                if phase != 'version':
                    require('test result: ok. 4 passed; 0 failed; 0 ignored;' in text,
                            f'{name}: missing positive actual four-test runtime')
                    for test, verdict in meta['expected_tests']['tests']:
                        require(text.count(test) == 1 and verdict == 'ok',
                                f'{name}: wrong actual test names/count')
                if phase == 'edit':
                    require(row['accepted'] and text.count(f'JR_RUNTIME_EXECUTED_{rep + 1}') == 1,
                            f'{name}: missing exact edit execution marker')
    for version in ('v21', 'v21c'):
        state = load(root, f'profiles/{version}/state.json')
        require(state['status'] == 'failed' and state['source_restored'], f'{version}: failure control missing')
        require('missing positive actual native diagnostics intervals' in state['error'],
                f'{version}: unexpected failure-control reason')
        require(state['source_before'] == state['source_after'], f'{version}: failed profile not restored')
    state = load(root, 'profiles/v22/state.json')
    require(state['status'] == 'passed' and state['source_restored'], 'positive profile failed')
    for field in ('source', 'tool', 'toolchain'):
        require(state[f'{field}_before'] == state[f'{field}_after'], f'profile {field} identity changed')
    require([row['mode'] for row in state['rows']] ==
            ['off', 'on', 'off-again', 'edit-one', 'edit-two', 'edit-three'], 'profile mode coverage changed')
    observations = []
    for row in state['rows']:
        require(row['status'] == 0 and not row['timed_out'] and not row['error'], 'profile runtime failed')
        mode = row['mode']
        text = ''.join(local(root, f'profiles/v22/{mode}.{suffix}').read_text()
                       for suffix in ('stdout', 'stderr'))
        require(text.count(row['marker']) == 1, f'{mode}: exact runtime marker missing/duplicated')
        if mode in ('off', 'off-again'):
            require(not local(root, f'profiles/v22/{mode}.trace.json').exists(), f'{mode}: unexpected trace')
            continue
        trace_path = local(root, f'profiles/v22/{mode}.trace.json')
        require(digest(trace_path) == row['trace_sha256'], f'{mode}: trace SHA mismatch')
        trace = json.loads(trace_path.read_text())
        require(trace == row['trace'], f'{mode}: embedded and standalone trace disagree')
        events = [event for event in trace['traceEvents'] if event.get('ph') == 'X'
                  and event.get('name') in LABELS]
        if mode.startswith('edit-'):
            require({event['name'] for event in events} == LABELS and len(events) == 4,
                    f'{mode}: four actual phase events required')
            require(all(event['cat'] == 'detail' and event['pid'] > 0 and event['tid'] > 0
                        and event['dur'] >= 0 for event in events), f'{mode}: invalid detail attribution')
            observations.append({'mode': mode, 'durations_us': {event['name']: event['dur'] for event in events}})
    logs = []
    for row in state['worker_logs']:
        path = local(root, f'profiles/v22/{row["artifact"]}')
        require(digest(path) == row['sha256'], 'worker log identity mismatch')
        logs.append(path.read_text())
    text = '\n'.join(logs)
    receipts = [tuple(map(int, match)) for match in re.findall(
        r'diagnostics transport: retained=(\d+) stale=(\d+) dropped=(\d+) cap=4000', text)]
    require(receipts == [(4, 94, 0), (4, 0, 0), (4, 0, 0)], 'actual transport receipt counts changed')
    callbacks = [tuple(map(int, match)) for match in re.findall(
        r'asked_callback_calls=(\d+) asked_callback_cumulative_ns=(\d+)', text)]
    require(callbacks[-3:] == [(373, 7969758), (373, 7676528), (373, 8332561)],
            'actual edit callback receipt counts changed')
    return {'verdict': 'passed', 'source_commit': collection['source_commit'],
            'corrected_full_native_run': CORRECTED_RUN, 'native_passed': 349,
            'first_full_native_run_provenance_limited': first[0]['id'],
            'positive_profile_rows': len(state['rows']), 'actual_edits': observations,
            'transport_receipts': receipts, 'edit_callback_receipts': callbacks[-3:],
            'claim': 'instrumentation evidence only, inclusive concurrent timings, no speedup claim'}


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument('--root', type=Path, default=Path(__file__).resolve().parent)
    args = parser.parse_args()
    print(json.dumps(validate(args.root), indent=2))


if __name__ == '__main__':
    main()
