#!/usr/bin/env python3
"""Author-only read-only export. Explicit roles and redactions, never ambient env.
This script is not imported or executed by portable validation.
"""
import argparse
import hashlib
import json
from pathlib import Path
import re
import subprocess


def sha(data):
    return hashlib.sha256(data).hexdigest()


def main():
    p = argparse.ArgumentParser()
    p.add_argument('--native-root', type=Path, required=True)
    p.add_argument('--abba-root', type=Path, required=True)
    p.add_argument('--repo', type=Path, required=True)
    p.add_argument('--output', type=Path, required=True)
    p.add_argument('--redact', action='append', required=True, help='public-role=exact-private-prefix')
    a = p.parse_args()
    maps = sorted((item.split('=', 1)[::-1] for item in a.redact), key=lambda x: -len(x[0]))
    seen = {}
    inputs = {}
    def clean(text):
        for private, role in maps:
            text = text.replace(private, '<' + role + '>')
        if re.search(r'(?<![A-Za-z0-9_./])/(?:home|Users)/|[A-Za-z]:\\\\', text):
            raise ValueError('unmapped private absolute path in export')
        return text
    def read(role, path):
        data = path.read_bytes()
        before = {'sha256': sha(data), 'bytes': len(data)}
        if role in inputs and inputs[role] != before:
            raise ValueError('input role changed')
        inputs[role] = before
        seen[path] = before
        return data
    def text(role, path):
        return read(role, path).decode()
    def load(role, path):
        return json.loads(read(role, path))
    def emit(name, value):
        data = value if isinstance(value, str) else json.dumps(value, indent=2, sort_keys=True) + '\n'
        data = clean(data)
        path = a.output / name
        path.parent.mkdir(parents=True, exist_ok=True)
        path.write_text(data)
        return name
    def archive(name, role, path):
        return emit(name, text(role, path))
    def process(row):
        result = {key: row[key] for key in ('command', 'status', 'error', 'timed_out', 'elapsed_seconds')}
        if 'noise' in row:
            result['noise'] = {key: row['noise'][key] for key in (
                'before', 'after', 'child_cpu_seconds', 'contamination', 'reason', 'other_cpu_seconds_estimate') if key in row['noise']}
        return result
    root = a.native_root
    src = root / 'balanced-manifest-syntax-v82'
    harness = root / 'parallel-manifest-harness-v81'
    original_paths = {
        'toolchain/resident/crates/blaze-manifest/src/manifest.rs': 'toolchain/resident/crates/blaze-manifest/src/manifest.rs',
        'toolchain/resident/crates/blaze-metadata/src/lib.rs': 'toolchain/resident/crates/blaze-metadata/src/lib.rs',
    }
    source = {'originals': {}, 'candidates': {}}
    for relative, old in original_paths.items():
        source['originals'][relative] = archive('source/originals/' + relative, 'source-freeze/originals/' + old, src / 'originals' / old)
    source_files = list(original_paths) + [
        'toolchain/resident/crates/blaze-manifest/tests/parallel_manifest_syntax/mod.rs',
        'toolchain/resident/crates/blaze-metadata/tests/parallel_manifest_syntax/mod.rs',
        'docs/resident-balanced-manifest-syntax.md',
    ]
    for relative in source_files:
        source['candidates'][relative] = archive('source/candidates/' + relative, 'source-freeze/candidates/' + relative, src / 'candidates' / relative)
        live = read('rollback/live/' + relative, root / 'balanced-manifest-syntax-v82-rollback/live' / relative)
        assert live == (src / 'candidates' / relative).read_bytes()
    for name in ('candidate.patch', 'RELEASE', 'SHA256SUMS', 'checks.json', 'sourcechecks.py', 'source-check-commands.txt', 'STOPEDITS', 'USAGE', 'diff-check.txt'):
        archive('source/freeze/' + name, 'source-freeze/' + name, src / name)
    ancestor_root = root / 'parallel-manifest-syntax-v81'
    ancestor = {'originals': source['originals'], 'candidates': {}}
    for relative, old in zip(original_paths, ('manifest.rs', 'metadata-lib.rs')):
        assert read('ancestor/originals/' + relative, ancestor_root / 'originals' / old) == read('source-freeze/originals/' + relative, src / 'originals' / relative)
    for relative in list(original_paths) + [
        'toolchain/resident/crates/blaze-manifest/tests/parallel_manifest_syntax/mod.rs',
        'toolchain/resident/crates/blaze-metadata/tests/parallel_manifest_syntax/mod.rs',
        'docs/resident-parallel-manifest-syntax.md',
    ]:
        ancestor['candidates'][relative] = archive('ancestor/candidates/' + relative, 'ancestor/candidates/' + relative, ancestor_root / 'candidates' / relative)
    for name in ('candidate.patch', 'RELEASE', 'SHA256SUMS', 'checks.json'):
        archive('ancestor/freeze/' + name, 'ancestor/freeze/' + name, ancestor_root / name)
    restoration = {'scope': 'point-in-time v82 rollback only, not current native checkout',
                   'time': '2026-10-31T17:45:29Z', 'files': {},
                   'restored_native_originals': list(original_paths),
                   'removed_preserved_fixture_modules': source_files[2:4],
                   'native_and_live_harness_head_diff_empty_then': True}
    for relative in source_files:
        frozen = inputs['source-freeze/candidates/' + relative]
        live = inputs['rollback/live/' + relative]
        assert frozen == live
        restoration['files'][relative] = {'frozen_original': frozen, 'preserved_live_original': live}
    emit('restoration/receipt.json', restoration)
    archive('formatting/root-rustfmt-check.log', 'formatting/root-rustfmt-check.log', root / 'balanced-manifest-syntax-v82-root-rustfmt-check.log')
    emit('formatting/receipt.json', {'exit': 1, 'classification': 'formatting-only diff, parser succeeded',
                                   'source_bytes_modified': False, 'native_failure': False})
    harness_data = {'originals': {}, 'candidates': {}}
    for relative in ('bench/resident_suite.py', 'docs/resident-suite.md'):
        harness_data['originals'][relative] = archive('harness/originals/' + relative, 'harness-freeze/originals/' + relative, harness / 'originals' / relative)
    for relative in ('bench/resident_suite.py', 'bench/test_resident_parallel_manifest.py', 'docs/resident-suite.md'):
        harness_data['candidates'][relative] = archive('harness/candidates/' + relative, 'harness-freeze/candidates/' + relative, harness / 'candidates' / relative)
        assert read('harness-restoration/live/' + relative, root / 'parallel-manifest-harness-v81-restoration/live' / relative) == (harness / 'candidates' / relative).read_bytes()
    for name in ('owned.patch', 'RELEASE', 'STOPEDITS', 'SHA256SUMS', 'staticchecks.py', 'staticchecks.log', 'USAGE', 'testlogs/parallel.log', 'testlogs/legacy.log', 'testlogs/final-all.log', 'testlogs/COMMANDS'):
        archive('harness/freeze/' + name, 'harness-freeze/' + name, harness / name)
    # Exact immutable Git text inputs. No compiler, Cargo or engine is invoked.
    for name in ('resident_state_matrix.py', 'resident_workload_compare.py', 'resident_engine_compare.py', 'blaze_runtime_compare.py', 'blaze_edit_compare.py'):
        data = subprocess.check_output(['git', 'show', 'c8829bae02fd234f46012ce6996baa14c09b3a1c:bench/' + name], cwd=a.repo)
        inputs['git-c8829bae0/bench/' + name] = {'sha256': sha(data), 'bytes': len(data)}
        emit('oracle/' + name, data.decode())
    for name in ('LICENSE-MIT', 'LICENSE-APACHE'):
        relative = 'toolchain/resident/' + name
        data = subprocess.check_output(['git', 'show', 'c8829bae02fd234f46012ce6996baa14c09b3a1c:' + relative], cwd=a.repo)
        inputs['git-c8829bae0/' + relative] = {'sha256': sha(data), 'bytes': len(data)}
        emit('source/licenses/' + name, data.decode())
    for name in ('build-test-candidate-v82.sh', 'profile-parallel-manifest-v82.py', 'probe-parallel-manifest-seven-v82.py', 'suite-frozen-parallel-v81.py', 'cleanup-completed-parallel-manifest-v82.py'):
        archive('drivers/' + name, 'drivers/' + name, root / name)
    gate_records = []
    for directory, gates in [('candidate-build-v82', ('check', 'compile')), ('candidate-unit-v82', ('manifest-native', 'metadata-native', 'codegen-native', 'full-native'))]:
        index = [json.loads(line) for line in text(directory + '/recorder-index', root / directory / 'justrust-home/index.jsonl').splitlines()]
        assert len(index) == len(gates)
        for name, recorder in zip(gates, index):
            journal = archive('gates/' + name + '.log', directory + '/' + name + '.log', root / directory / (name + '.log'))
            exit_code = int(text(directory + '/' + name + '.exit', root / directory / (name + '.exit')))
            raw_path = root / directory / 'justrust-home/runs' / recorder['id'] / 'output.jsonl'
            rows = [json.loads(line) for line in text(directory + '/' + name + '/full-output', raw_path).splitlines()]
            assert all(set(row) == {'t', 's', 'l'} for row in rows)
            raw_name = emit('gates/' + name + '-full.json', rows)
            gate_records.append({'name': name, 'exit': exit_code, 'journal': journal, 'raw_journal': raw_name,
                'recorder': {key: recorder[key] for key in ('id', 'start', 'args', 'exit', 'wall', 'cpu_secs', 'compile', 'test_run', 'units_compiled', 'compile_failed')}})
    for directory, names in [('candidate-build-v82', ('lock-before.sha256', 'all-source-before.sha256', 'engine-version.txt', 'engine-identity.sha256', 'orchestration-exit')), ('candidate-unit-v82', ('all-source-after.log', 'lock-after.log', 'exit'))]:
        for name in names:
            archive('gates/' + name, directory + '/' + name, root / directory / name)
    toolchain_identity = None
    def state_common(state):
        nonlocal toolchain_identity
        before = state['toolchain_before']['files']
        after = state['toolchain_after']['files']
        assert before == after
        if toolchain_identity is None:
            toolchain_identity = before
        assert before == toolchain_identity
        emit('inventories/toolchain.json', before)
        return {key: state[key] for key in ('status', 'source_before', 'source_after', 'tool_before', 'tool_after', 'unchanged')}
    profile_root = root / 'resolver-subphases-profile-v82'
    profile_state = load('profile/state.json', profile_root / 'state.json')
    profile = state_common(profile_state)
    profile['prepare'] = process(profile_state['prepare'])
    profile['stop'] = process(profile_state['stop'])
    for name in ('source.original', 'prepare.stdout', 'prepare.stderr', 'stop.stdout', 'stop.stderr'):
        archive('profile/' + name, 'profile/' + name, profile_root / name)
    profile['rows'] = []
    labels = ('resolver ', 'client planning')
    for row in profile_state['rows']:
        stem = str(row['replicate']) + '-' + row['kind']
        out = {key: row[key] for key in ('kind', 'replicate', 'marker', 'source_sha256', 'trace_sha256', 'resolver_labels_observed')}
        out.update(process(row))
        out['events'] = [{key: event[key] for key in ('cat', 'dur', 'name', 'ph', 'pid', 'tid', 'ts')} for event in row['events'] if event['name'].startswith(labels)]
        for suffix in ('stdout', 'stderr'):
            archive('profile/' + stem + '.' + suffix, 'profile/' + stem + '.' + suffix, profile_root / (stem + '.' + suffix))
        original_trace = read('profile/' + stem + '.trace.json', profile_root / (stem + '.trace.json'))
        assert sha(original_trace) == row['trace_sha256']
        trace = json.loads(original_trace)
        events = [{key: event[key] for key in ('cat', 'dur', 'name', 'ph', 'pid', 'tid', 'ts')} for event in trace['traceEvents'] if event.get('ph') == 'X' and event['name'].startswith(labels)]
        assert events == out['events']
        emit('profile/' + stem + '.trace.json', {'traceEvents': events})
        profile['rows'].append(out)
    strict = []
    for mode in ('off', 'on', 'on-repeat', 'off-repeat'):
        directory = root / ('parallel-manifest-seven-v82-' + mode)
        state = load('strict/' + mode + '/state.json', directory / 'state.json')
        out = state_common(state)
        out.update(mode=mode, startup_controls=state['startup_controls'], rows=[])
        for i in range(3):
            archive('strict/' + mode + '/original-' + str(i), 'strict/' + mode + '/original-' + str(i), directory / ('original-' + str(i)))
        for i, row in enumerate(state['rows']):
            item = {key: row[key] for key in ('phase', 'nonce', 'inputs', 'accepted', 'raw_error', 'parsed', 'input_unchanged', 'failures')}
            item.update(process(row))
            item['status_receipt'] = process(row['status_receipt'])
            stem = f'{i:02}-' + row['phase']
            for suffix in ('stdout', 'stderr', 'status.stdout', 'status.stderr', 'worker-0.log', 'daemon.log'):
                separator = '-' if suffix.startswith(('status', 'worker', 'daemon')) else '.'
                name = stem + separator + suffix
                archive('strict/' + mode + '/' + name, 'strict/' + mode + '/' + name, directory / name)
            out['rows'].append(item)
        strict.append(out)
    abba_meta = load('abba/metadata.json', a.abba_root / 'metadata.json')
    abba_rows = load('abba/records.json', a.abba_root / 'records.json')
    abba = {'metadata': {key: abba_meta[key] for key in ('schema', 'pin', 'order', 'samples', 'source_before', 'source_after', 'fixture_source', 'profiles', 'status', 'mode', 'factor', 'run_id', 'rules', 'source_restored', 'toolchain_links_valid')}, 'rows': [], 'stops': []}
    abba['metadata']['tools_before'] = {key: value['sha256'] for key, value in abba_meta['tools_before'].items()}
    abba['metadata']['tools_after'] = {key: value['sha256'] for key, value in abba_meta['tools_after'].items()}
    assert abba_meta['tools_before'] == abba_meta['tools_after']
    abba['metadata']['tool_paths'] = {key: value['path'] for key, value in abba_meta['tools_before'].items()}
    assert abba_meta['toolchain_before'] == abba_meta['toolchain_after']
    assert abba_meta['toolchain_before']['files'] == toolchain_identity
    for name in ('source.original', 'source.fixture', 'Cargo.toml.original', 'Cargo.lock.original'):
        archive('abba/' + name, 'abba/' + name, a.abba_root / name)
    for i, row in enumerate(abba_rows):
        out = {key: row[key] for key in ('group', 'phase', 'replicate', 'flag', 'nonce', 'nonce_environment', 'touches', 'source_before', 'source_after', 'flags', 'schema', 'run_id', 'rules')}
        out.update(process(row))
        if 'worker' in row:
            w = row['worker']
            argv = bytes.fromhex(w['cmdline_hex']).decode().rstrip('\0').split('\0')
            out['worker'] = {'pid': w['pid'], 'stat': w['stat'], 'exe': w['exe'], 'exe_sha256': w['exe_sha256'], 'argv': argv}
            out['worker_status'] = process(row['worker_status'])
            for suffix in ('stdout', 'stderr'):
                archive(f'abba/call-{i:03}-status.' + suffix, f'abba/call-{i:03}-status.' + suffix, a.abba_root / (f'call-{i:03}-status.' + suffix))
        for suffix in ('stdout', 'stderr'):
            archive(f'abba/call-{i:03}.' + suffix, f'abba/call-{i:03}.' + suffix, a.abba_root / (f'call-{i:03}.' + suffix))
        abba['rows'].append(out)
    for stop in abba_meta['stops']:
        abba['stops'].append({'group': stop['group'], 'process': process(stop['process'])})
        for suffix in ('stdout', 'stderr'):
            name = stop['group'] + '-stop.' + suffix
            archive('abba/' + name, 'abba/' + name, a.abba_root / name)
    for arm in ('off', 'on'):
        saved = load('abba/' + arm + '.json', a.abba_root / (arm + '.json'))
        emit('abba/' + arm + '-result.json', {key: saved[key] for key in ('schema', 'arm', 'measurements', 'noise', 'scope')})
    archive('abba/compare.txt', 'abba/compare.txt', root / 'parallel-manifest-v82-compare.txt')
    cleanup_root = root / 'completed-parallel-manifest-v82-stop'
    cleanup_state = load('cleanup/receipt.json', cleanup_root / 'receipt.json')
    cleanup = {'scope': cleanup_state['scope'], 'stores_deleted': cleanup_state['stores_deleted'], 'rows': []}
    for row in cleanup_state['rows']:
        item = process(row)
        item.update(case=row['case'], post_measurement_only=row['post_measurement_only'])
        cleanup['rows'].append(item)
        for suffix in ('stdout', 'stderr'):
            archive('cleanup/' + row['case'] + '.' + suffix, 'cleanup/' + row['case'] + '.' + suffix, cleanup_root / (row['case'] + '.' + suffix))
    observations = {'authority': 'c8829bae02fd234f46012ce6996baa14c09b3a1c', 'contract': {'promotion': 'REJECT', 'floor_percent': 10, 'noise': 'unknown', 'causal_claim': False, 'general_speed_claim': False, 'default_promotion': False, 'native_rerun_by_exporter': False}, 'source': source, 'ancestor': ancestor, 'harness': harness_data, 'gates': gate_records, 'profile': profile, 'strict': strict, 'abba': abba, 'cleanup': cleanup}
    emit('observations.json', observations)
    for path, before in seen.items():
        now = path.read_bytes()
        assert {'sha256': sha(now), 'bytes': len(now)} == before, 'private input changed during read-only export'
    emit('private-inputs.json', {'files': inputs, 'originals_unchanged_after_export': True, 'ambient_environment_exported': False})
    print(json.dumps({'explicit_inputs': len(inputs), 'observations_sha256': sha((a.output / 'observations.json').read_bytes()), 'private_originals_unchanged': True}))


if __name__ == '__main__':
    main()
