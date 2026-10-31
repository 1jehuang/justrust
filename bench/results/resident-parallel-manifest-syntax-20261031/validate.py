#!/usr/bin/env python3
"""Standalone stdlib replay of preserved v81 evidence. Never invokes native tools."""
import argparse
import ast
import hashlib
import json
import math
from pathlib import Path
import re
import statistics
from types import SimpleNamespace as NS

AUTHORITY = 'c8829bae02fd234f46012ce6996baa14c09b3a1c'
OBSERVATIONS_SHA256 = 'c1a3453c7e4e71f7e8af9c45ab7c9fb03b6dfeaa31345291280ba08e39aebcd2'
HISTORICAL_SHA256 = '3e0d588f755a65828174c2825034ecfdd3e8683ed888eaac0f089662c78dfdc0'
MARKER = 'ra-jit: parallel-manifest-parse-v81 batch'
FACTOR = 'RA_JIT_PARALLEL_MANIFEST_PARSE'
GENERATION = 'RA_JIT_MANIFEST_CONTENT_GENERATION'
WATCHER = 'RA_JIT_WATCHED_MANIFEST_BYTES'
PHASES = ('cold-prepare', 'repeated-noop-1', 'manifest-feature-add', 'type-error', 'type-recovery', 'assert-failure', 'assert-recovery')
PROCESS = {'command', 'status', 'error', 'timed_out', 'elapsed_seconds', 'noise'}
LABELS = ('client planning rustc host', 'client planning target cfg', 'client planning asked setup', 'client planning owned resolver', 'client planning', 'resolver workspace/config', 'resolver sources.fetch', 'resolver fetched manifest syntax', 'resolver fetched manifest normalize', 'resolver package loading', 'resolver summary preparation', 'resolver version resolve', 'resolver finalization', 'resolver feature graph/export', 'resolver program projection')
ORACLE_SHA256 = {
    'oracle/blaze_edit_compare.py': '9c451fdb939d3a8c024ddf7e2b8c2fd3a8badc7c164a429415a7c636eb8c41a8',
    'oracle/blaze_runtime_compare.py': 'a7e2af863e2b1ea2878ae5c66a0f01aab1ad7fb9efe2e1027254a1269eefb568',
    'oracle/resident_engine_compare.py': '7da5b024344f662743b81043f4c200a23b3a49f61356e933b48257b2860c5a80',
    'oracle/resident_state_matrix.py': '138aa2fb7e1791de92d43c8855c6151abd188e5f57fdb4d28c22e326c54be738',
    'oracle/resident_workload_compare.py': 'e47ef667315c0044e2b0072eb43a9db68860f32b50757bdd927b49522de1e92d',
    'harness/candidates/bench/resident_suite.py': '548d9dc3fd6c9d7075f576b920c833e36715bcba22f9656bfbd2a5270e5cec66',
}


def require(condition, message):
    if not condition:
        raise ValueError(message)


def sha(data):
    return hashlib.sha256(data).hexdigest()


def read(root, name):
    require(isinstance(name, str) and not Path(name).is_absolute() and '..' not in Path(name).parts, 'unsafe bundle path')
    path = root / name
    require(path.resolve().is_relative_to(root) and not path.is_symlink() and path.is_file(), 'missing or linked bundle file: ' + name)
    return path.read_bytes()


def load(root, name):
    return json.loads(read(root, name))


def exact(value, keys, label):
    require(isinstance(value, dict) and set(value) == set(keys), 'unexpected fields: ' + label)


def process(row, status=0):
    require(row['status'] == status and row['error'] is None and row['timed_out'] is False, 'process status/error/timeout')
    require(type(row['elapsed_seconds']) in (int, float) and math.isfinite(row['elapsed_seconds']) and row['elapsed_seconds'] > 0, 'invalid elapsed time')
    if 'noise' in row:
        require(row['noise']['contamination'] == 'unknown', 'noise must remain unknown')
        require(set(row['noise']) <= {'before', 'after', 'child_cpu_seconds', 'contamination', 'reason', 'other_cpu_seconds_estimate'}, 'unknown noise field')


def extract(root, file, names, namespace=None):
    """Execute only explicitly named pure original definitions, never archived imports/drivers."""
    namespace = {} if namespace is None else namespace
    tree = ast.parse(read(root, file).decode(), filename=file)
    nodes = []
    for node in tree.body:
        name = node.name if isinstance(node, ast.FunctionDef) else node.targets[0].id if isinstance(node, ast.Assign) and len(node.targets) == 1 and isinstance(node.targets[0], ast.Name) else None
        if name in names:
            nodes.append(node)
    require(len(nodes) == len(names), 'missing original oracle definitions')
    exec(compile(ast.Module(body=nodes, type_ignores=[]), file, 'exec'), namespace)
    return NS(**{name: namespace[name] for name in names})


def original_oracle(root):
    for file, digest in ORACLE_SHA256.items():
        require(sha(read(root, file)) == digest, 'original oracle code changed: ' + file)
    base = extract(root, 'oracle/blaze_edit_compare.py', ['succeeded'])
    runtime = extract(root, 'oracle/blaze_runtime_compare.py', ['test_results', 'edit_bytes'], {'re': re})
    runtime.base = base
    shared = extract(root, 'oracle/resident_engine_compare.py', ['NAMES', 'EDIT_TEST'])
    shared.runtime = runtime
    workload = extract(root, 'oracle/resident_workload_compare.py', ['WORKLOADS', 'valid', 'discover', 'edited'], {'re': re})
    matrix = extract(root, 'oracle/resident_state_matrix.py', ['FILES', 'ANCHOR', 'SHIM_ANCHOR', 'PROBE', 'SIGNATURE', 'SIGNATURE_CHANGED', 'baseline', 'inputs', 'contract', 'oracle'], {'runtime': runtime, 'shared': shared, 'workload': workload, 're': re})
    marker = extract(root, 'harness/candidates/bench/resident_suite.py', ['require', 'check_parallel_activation', 'retouch_fixture'], {'PARALLEL_MARKER': MARKER})
    return runtime, shared, workload, matrix, marker


def patch_join(patch, originals):
    """Strict in-memory unified-patch replay. No git/subprocess/external file access."""
    lines = patch.splitlines(keepends=True)
    result = dict(originals)
    i = 0
    seen = set()
    while i < len(lines):
        if not lines[i].startswith('diff --git '):
            i += 1
            continue
        i += 1
        while i < len(lines) and not lines[i].startswith('--- '):
            i += 1
        require(i + 1 < len(lines), 'patch missing file header')
        old = lines[i][4:].strip()
        new = lines[i + 1][4:].strip()
        require(old == '/dev/null' or old.startswith('a/'), 'bad old patch path')
        require(new.startswith('b/'), 'bad new patch path')
        name = new[2:]
        require(name not in seen and '..' not in Path(name).parts, 'duplicate/unsafe patch path')
        seen.add(name)
        current = [] if old == '/dev/null' else result.pop(old[2:]).splitlines(keepends=True)
        output, at = [], 0
        i += 2
        while i < len(lines) and not lines[i].startswith('diff --git '):
            header = re.match(r'@@ -(\d+)(?:,(\d+))? \+(\d+)(?:,(\d+))? @@', lines[i])
            require(header is not None, 'malformed patch hunk')
            start, old_count, new_start, new_count = [int(value) if value is not None else 1 for value in header.groups()]
            target = start - 1 if start else 0
            require(target >= at, 'out-of-order patch hunk')
            output.extend(current[at:target])
            at = target
            i += 1
            removed = added = 0
            while i < len(lines) and not lines[i].startswith(('@@ ', 'diff --git ')):
                line = lines[i]
                require(line[:1] in (' ', '-', '+'), 'unsupported patch content')
                if line[0] in (' ', '-'):
                    require(at < len(current) and current[at] == line[1:], 'patch context/deletion mismatch')
                    at += 1
                    removed += 1
                if line[0] in (' ', '+'):
                    output.append(line[1:])
                    added += 1
                i += 1
            require((removed, added) == (old_count, new_count), 'patch hunk length mismatch')
        output.extend(current[at:])
        result[name] = ''.join(output)
    return result


def validate(root=None, manifest=False, historical=True):
    root = Path(root or __file__).resolve()
    if root.is_file():
        root = root.parent
    data = load(root, 'observations.json')
    exact(data, ('authority', 'contract', 'source', 'harness', 'gates', 'profile', 'strict', 'abba', 'cleanup'), 'observations')
    require(data['authority'] == AUTHORITY, 'wrong authority')
    require(data['contract'] == {'promotion': 'REJECT', 'floor_percent': 10, 'noise': 'unknown', 'causal_claim': False, 'general_speed_claim': False, 'default_promotion': False, 'native_rerun_by_exporter': False}, 'rejection/scope contract changed')
    # Hash-only authenticity is not semantic proof. All semantic checks below run
    # before the final fixed-history commitment, and negatives reseal manifests.
    runtime, shared, workload, matrix, marker = original_oracle(root)
    for group, patch in [('source', 'source/freeze/candidate.patch'), ('harness', 'harness/freeze/owned.patch')]:
        old = {name: read(root, path).decode() for name, path in data[group]['originals'].items()}
        new = {name: read(root, path).decode() for name, path in data[group]['candidates'].items()}
        require(patch_join(read(root, patch).decode(), old) == new, group + ' original/candidate patch join failed')
    source_names = data['source']['candidates']
    manifest_path = 'toolchain/resident/crates/blaze-manifest/src/manifest.rs'
    old = read(root, data['source']['originals'][manifest_path]).decode()
    new = read(root, source_names[manifest_path]).decode()
    old_body = old.split('pub fn read_manifest(', 1)[1].split('\nfn to_workspace_config(', 1)[0].split(') -> anyhow::Result<EitherManifest> {\n', 1)[1]
    semantic = '    let mut warnings' + old_body.split('    let mut warnings', 1)[1]
    new_semantic = new.split('fn normalize_manifest(', 1)[1].split('\nfn to_workspace_config(', 1)[0].split(') -> anyhow::Result<EitherManifest> {\n', 1)[1]
    require(semantic == new_semantic, 'common semantic body changed')
    prefix = old_body.split('    let mut warnings', 1)[0].rstrip()
    syntax = new.split('fn read_manifest_syntax(', 1)[1].split('\n}\n', 1)[0].split(') -> anyhow::Result<TomlManifest> {\n', 1)[1]
    require(syntax.removesuffix('\n    Ok(original_toml)').rstrip() == prefix, 'original sequential syntax prefix changed')
    metadata_path = 'toolchain/resident/crates/blaze-metadata/src/lib.rs'
    old_metadata = read(root, data['source']['originals'][metadata_path]).decode()
    metadata = read(root, source_names[metadata_path]).decode()
    old_fetch = old_metadata.split('\nfn fetch(', 1)[1].split('\n/// The registries', 1)[0]
    new_fetch = metadata.split('\nfn fetch(', 1)[1].split('\nfn parallel_manifest_parse_enabled', 1)[0]
    require(old_fetch.split('    let mut read = Vec::with_capacity(texts.len());', 1)[1] == new_fetch.split('    let mut read = Vec::with_capacity(texts.len());', 1)[1], 'OFF ordered fetch tail changed')
    oracle = read(root, source_names['toolchain/resident/crates/blaze-manifest/tests/parallel_manifest_syntax/mod.rs']).decode()
    oracle_body = oracle.split('fn original_read_manifest(', 1)[1].split('\nstruct Fixture', 1)[0].split(') -> anyhow::Result<EitherManifest> {\n', 1)[1]
    require(oracle_body.rstrip() == old_body.rstrip(), 'frozen original native oracle changed')
    native = {}
    for line in read(root, 'gates/all-source-before.sha256').decode().splitlines():
        match = re.fullmatch(r'([0-9a-f]{64})  (.+)', line)
        require(match is not None, 'invalid native source inventory')
        digest, name = match.groups()
        require(name not in native and not Path(name).is_absolute() and '..' not in Path(name).parts, 'unsafe native inventory')
        native[name] = digest
    require(len(native) == 3759, 'actual native source count changed')
    after = read(root, 'gates/all-source-after.log').decode().splitlines()
    require(after == [name + ': OK' for name in native], 'native source post-check mismatch')
    for name, bundled in source_names.items():
        if name.endswith('.rs'):
            require(native[name.removeprefix('toolchain/resident/')] == sha(read(root, bundled)), 'candidate native inventory join failed')
    require(read(root, 'gates/lock-after.log') == b'Cargo.lock: OK\n', 'native lock changed')
    require(read(root, 'gates/orchestration-exit').strip() == read(root, 'gates/exit').strip() == b'0', 'native orchestration failed')
    identity = read(root, 'gates/engine-identity.sha256').decode().split()[0]
    require(re.fullmatch(r'[0-9a-f]{64}', identity), 'bad native executable identity')
    require(re.search(r'Blaze-derived origin [0-9a-f]{40}; build resident-v1-[0-9a-f]{64}\)', read(root, 'gates/engine-version.txt').decode()), 'missing build/version lineage')
    require([gate['name'] for gate in data['gates']] == ['check', 'compile', 'manifest-native', 'metadata-native', 'codegen-native', 'full-native'], 'native gate set changed')
    native_counts = {}
    for gate in data['gates']:
        require(gate['exit'] == gate['recorder']['exit'] == 0 and gate['recorder']['compile_failed'] is False, 'native gate failed')
        lines = load(root, gate['raw_journal'])
        require(all(set(line) == {'t', 's', 'l'} and line['s'] in ('e', 'o') for line in lines), 'raw journal fields changed')
        named = [re.match(r'^test (\S+) \.\.\. (ok|FAILED|ignored)(?: |$)', line['l']) for line in lines]
        named = [item.groups() for item in named if item]
        if gate['name'].endswith('-native'):
            expected_count = {'manifest-native': 28, 'metadata-native': 6, 'codegen-native': 53, 'full-native': 373}[gate['name']]
            require(len(named) == expected_count and all(status == 'ok' for _, status in named), 'native named test count/status mismatch')
            require(len(set(name for name, _ in named)) == expected_count, 'duplicate native named test')
            native_counts[gate['name']] = expected_count
    tc = load(root, 'inventories/toolchain.json')
    require(len(tc) == 4020 and {'bin/rustc', 'bin/cargo', 'lib/rustlib/multirust-channel-manifest.toml'} <= set(tc), 'toolchain inventory incomplete')
    require(all(set(value) == {'sha256', 'link'} and re.fullmatch('[0-9a-f]{64}', value['sha256']) for value in tc.values()), 'toolchain identity malformed')
    profile = data['profile']
    def common(state):
        require(state['status'] == 'passed' and state['unchanged'] is True, 'runtime state failed or changed')
        require(len(state['source_before']) == 63 and state['source_before'] == state['source_after'], 'pinned source63 changed')
        require(state['tool_before'] == state['tool_after'] == identity, 'runtime tool identity changed')
    common(profile)
    for action in ('prepare', 'stop'):
        process(profile[action])
    require(len(profile['rows']) == 8, 'profile actual row count')
    ranges = {name: [] for name in LABELS}
    for i, row in enumerate(profile['rows']):
        require((row['replicate'], row['kind']) == (i // 2, 'edit' if i % 2 == 0 else 'noop'), 'profile row order')
        process(row)
        stem = f"profile/{row['replicate']}-{row['kind']}"
        stdout, stderr = (read(root, stem + '.' + suffix).decode() for suffix in ('stdout', 'stderr'))
        expected_marker = f'JR_RUNTIME_EXECUTED_{8101 + i // 2}'
        require(row['marker'] == expected_marker and re.findall(r'JR_RUNTIME_EXECUTED_\d+', stdout + stderr) == [expected_marker], 'profile runtime marker')
        marker.check_parallel_activation(stdout, stderr, '1', 'retouch')
        parsed = runtime.test_results(stdout + '\n' + stderr)
        require(matrix.contract(parsed, 'cold-prepare') is not None, 'profile original test contract')
        edited = runtime.edit_bytes(read(root, 'profile/source.original'), 8100 + i // 2)
        require(row['source_sha256'] == sha(edited), 'profile source edit recipe mismatch')
        require(row['events'] == load(root, stem + '.trace.json')['traceEvents'], 'profile trace join')
        require([event['name'] for event in row['events']] == list(LABELS), 'profile missing/duplicate/out-of-order labels')
        for event in row['events']:
            exact(event, ('cat', 'dur', 'name', 'ph', 'pid', 'tid', 'ts'), 'trace event')
            require(event['cat'] in ('step', 'detail') and event['ph'] == 'X' and all(type(event[key]) is int and event[key] > 0 for key in ('dur', 'pid', 'tid', 'ts')), 'bad complete trace event')
            ranges[event['name']].append(event['dur'])
        by_name = {event['name']: event for event in row['events']}
        parent = by_name['resolver package loading']
        for name in ('resolver sources.fetch', 'resolver fetched manifest syntax', 'resolver fetched manifest normalize'):
            event = by_name[name]
            require(event['pid'] == parent['pid'] and event['tid'] == parent['tid'] and parent['ts'] <= event['ts'] and event['ts'] + event['dur'] <= parent['ts'] + parent['dur'], 'manifest subspan outside package loading')
    require([state['mode'] for state in data['strict']] == ['off', 'on', 'on-repeat', 'off-repeat'], 'strict mode order')
    strict_nonces = set()
    for state in data['strict']:
        common(state)
        mode = state['mode']
        require(len(state['rows']) == 7 and [row['phase'] for row in state['rows']] == list(PHASES), 'strict original seven phases')
        controls = state['startup_controls']
        expected_flag = '0' if mode.startswith('off') else '1'
        require(controls == {'RA_JIT_DEMAND_STD': '1', 'RA_JIT_CACHE_MISS_STATS': '1', 'RA_JIT_STATS': '1', 'RA_JIT_WHOLE_RELOAD': '0', WATCHER: '1', GENERATION: '1', FACTOR: expected_flag}, 'strict fixed startup controls')
        originals = {name: read(root, 'strict/' + mode + '/original-' + str(i)) for i, name in enumerate(matrix.FILES)}
        expected = None
        for i, row in enumerate(state['rows']):
            nonce, phase = row['nonce'], row['phase']
            require(re.fullmatch('[0-9a-f]{32}', nonce) and nonce not in strict_nonces, 'strict stale nonce')
            strict_nonces.add(nonce)
            process(row, 101 if phase in ('type-error', 'assert-failure') else 0)
            process(row['status_receipt'])
            stem = f'strict/{mode}/{i:02}-{phase}'
            stdout, stderr = (read(root, stem + '.' + suffix).decode() for suffix in ('stdout', 'stderr'))
            parsed = runtime.test_results(stdout + '\n' + stderr)
            if expected is None:
                expected = matrix.contract(parsed, phase)
                require(expected is not None, 'strict missing cold contract')
            replay_row = dict(row, parsed=parsed)
            failures = matrix.oracle(replay_row, stdout + '\n' + stderr, expected, phase, nonce, 0)
            require(not failures and row['accepted'] is True and row['failures'] == [] and row['input_unchanged'] is True and row['raw_error'] is None, 'original strict oracle: ' + repr(failures))
            require(json.loads(json.dumps(parsed)) == row['parsed'], 'strict parsed receipt differs from raw')
            wanted = dict(state['source_before'])
            wanted.update({name: sha(contents) for name, contents in matrix.inputs(originals, phase, 0, nonce).items()})
            require(wanted == row['inputs'], 'strict source edit recipe mismatch')
            marker.check_parallel_activation(stdout, stderr, expected_flag, phase)
    abba = data['abba']
    meta = abba['metadata']
    require(meta['schema'] == 'native_resident_parallel_manifest_retouch/v1' and meta['pin'] == 'f97fcc2' and meta['order'] == ['off', 'on', 'on', 'off'] and meta['samples'] == 5, 'ABBA schema/order/samples')
    require(meta['status'] == 'complete' and meta['source_restored'] is True and meta['toolchain_links_valid'] is True, 'ABBA restoration/status')
    require(meta['source_before'] == meta['source_after'] == profile['source_before'] and len(meta['source_before']) == 63, 'ABBA source63 identity')
    require(meta['tools_before'] == meta['tools_after'] and meta['tools_before']['engine'] == identity, 'ABBA tool identity')
    original = read(root, 'abba/source.original')
    overlay = marker.retouch_fixture(original)
    require(sha(original) == meta['source_before']['src/summary.rs'] and read(root, 'abba/source.fixture') == overlay, 'ABBA exact runtime overlay')
    require(meta['fixture_source'] == dict(meta['source_before'], **{'src/summary.rs': sha(overlay)}), 'ABBA fixture inventory')
    phases = ('version', 'prepare', 'type-error', 'type-recovery', 'assert-failure', 'assert-recovery')
    wanted_order = [('reference', phase, 0) for phase in phases]
    for segment in range(4):
        wanted_order += [(f'segment-{segment}', phase, 0) for phase in phases] + [(f'segment-{segment}', 'warmup', 0)] + [(f'segment-{segment}', 'retouch', n) for n in range(5)]
    require([(row['group'], row['phase'], row['replicate']) for row in abba['rows']] == wanted_order, 'ABBA 54-call exact layout')
    samples, workers, nonces = {'off': [], 'on': []}, {}, set()
    for i, row in enumerate(abba['rows']):
        group, phase = row['group'], row['phase']
        arm = 'off' if group == 'reference' else meta['order'][int(group[-1])]
        flag = '1' if arm == 'on' else '0'
        fixed = '0' if group == 'reference' else '1'
        require(row['flag'] == fixed and row['flags'] == {GENERATION: fixed, WATCHER: fixed, FACTOR: flag}, 'ABBA exact per-call controls')
        require(row['schema'] == meta['schema'] and row['run_id'] == meta['run_id'] and row['rules'] == meta['rules'], 'ABBA cross-run/rules mixture')
        process(row, 101 if phase in ('type-error', 'assert-failure') else 0)
        stdout, stderr = (read(root, f'abba/call-{i:03}.' + suffix).decode() for suffix in ('stdout', 'stderr'))
        marker.check_parallel_activation(stdout, stderr, flag, phase)
        contents = runtime.edit_bytes(original, kind='type-error' if phase == 'type-error' else 'test-failure') if phase in ('type-error', 'assert-failure') else overlay if phase in ('warmup', 'retouch') else original
        require(row['source_before'] == row['source_after'] == dict(meta['source_before'], **{'src/summary.rs': sha(contents)}), 'ABBA per-call source identity')
        text = stdout + '\n' + stderr
        parsed = runtime.test_results(text)
        if phase == 'version':
            require(('blaze 0.20261027.0' if group == 'reference' else 'jr-engine experimental') in stdout, 'ABBA version')
        elif phase == 'type-error':
            require(parsed is None and 'E0308' in text and 'JR_RUNTIME_TYPE_ERROR' in text, 'ABBA real type diagnostic')
        else:
            expected = {'tests': [(name, 'ok') for name in shared.NAMES], 'passed': 4, 'failed': 0, 'ignored': 0, 'filtered': 144}
            require(workload.valid(parsed, expected, shared.EDIT_TEST if phase == 'assert-failure' else None), 'ABBA original exact libtest contract')
            if phase == 'assert-failure':
                require('JR_RUNTIME_FAILURE' in text, 'ABBA assertion diagnostic')
        if phase in ('warmup', 'retouch'):
            nonce = row['nonce']
            require(re.fullmatch('[0-9a-f]{32}', nonce) and nonce not in nonces and row['nonce_environment'] == {'JR_MANIFEST_RETOUCH_NONCE': nonce}, 'ABBA nonce controls')
            nonces.add(nonce)
            require(re.findall(r'JR_MANIFEST_RETOUCH_[^\s]+', text) == ['JR_MANIFEST_RETOUCH_' + nonce], 'ABBA actual runtime nonce')
            require(text.splitlines().count('ra-jit: manifest-content-generation-v1 entered') == 1, 'ABBA generation activation')
            process(row['worker_status'])
            w = row['worker']
            fields = w['stat'].split(') ', 1)[1].split()
            require(int(w['stat'].split()[0]) == w['pid'] and len(fields) >= 20, 'ABBA proc PID/stat')
            require(w['exe_sha256'] == identity and w['exe'] == w['argv'][0], 'ABBA worker executable')
            require(w['argv'].count('--socket') == 1, 'ABBA worker socket argument')
            socket = w['argv'][w['argv'].index('--socket') + 1]
            status = read(root, f'abba/call-{i:03}-status.stdout').decode()
            require(re.findall(r'worker: pid (\d+)', status) == [str(w['pid'])], 'ABBA status/proc PID join')
            worker = (w['pid'], fields[19], w['exe'], socket, tuple(w['argv']))
            if phase == 'retouch':
                require(group in workers and workers[group] == worker, 'ABBA worker PID/start/exe/socket/argv changed')
                require([touch['file'] for touch in row['touches']] == ['Cargo.toml', 'Cargo.lock'], 'ABBA combined touch set')
                for touch in row['touches']:
                    name = touch['file']
                    before, after = touch['before'], touch['after']
                    backup = read(root, 'abba/' + name + '.original')
                    require(before['sha256'] == after['sha256'] == sha(backup) == meta['source_before'][name] and before['size'] == after['size'] == len(backup), 'ABBA touch changed bytes/size')
                    require(type(before['mtime_ns']) is int and type(after['mtime_ns']) is int and after['mtime_ns'] > before['mtime_ns'], 'ABBA mtime not increasing')
                samples[arm].append(row['elapsed_seconds'])
            else:
                require(row['touches'] == [], 'ABBA warmup touched inputs')
            workers[group] = worker
        else:
            require(row['nonce'] is None and row['nonce_environment'] == {} and row['touches'] == [], 'ABBA gate edited timed inputs')
    require(all(len(values) == 10 for values in samples.values()) and len(workers) == 4, 'ABBA dropped first touches or worker groups')
    require(len({value[0] for value in workers.values()}) == 4 and len({value[3] for value in workers.values()}) == 4, 'ABBA aliased private workers')
    measurements = {arm: {'samples_seconds': values, 'median_seconds': statistics.median(values)} for arm, values in samples.items()}
    for arm in ('off', 'on'):
        saved = load(root, 'abba/' + arm + '-result.json')
        require(saved['arm'] == arm and saved['schema'] == meta['schema'] and saved['measurements'] == measurements and saved['noise'] == 'unknown', 'ABBA result/sample/median replay mismatch')
    improvement = 100 * (measurements['off']['median_seconds'] - measurements['on']['median_seconds']) / measurements['off']['median_seconds']
    require(improvement <= data['contract']['floor_percent'], 'rejected comparison exceeds promotion floor')
    compare = read(root, 'abba/compare.txt').decode()
    require('-9.58%' in compare and 'does not exceed requested 10% floor' in compare and 'noise unknown' in compare, 'original comparison text changed')
    require([stop['group'] for stop in abba['stops']] == ['reference'] + [f'segment-{i}' for i in range(4)], 'ABBA stops missing')
    for stop in abba['stops']:
        process(stop['process'])
    cleanup = data['cleanup']
    require(cleanup['stores_deleted'] is False and [row['case'] for row in cleanup['rows']] == ['parallel-manifest-seven-v81-' + state['mode'] for state in data['strict']], 'cleanup scope/stores')
    for row in cleanup['rows']:
        process(row)
        require(row['post_measurement_only'] is True and read(root, 'cleanup/' + row['case'] + '.stdout') == b'daemon: stopped\n' and read(root, 'cleanup/' + row['case'] + '.stderr') == b'', 'cleanup raw receipt')
    receipt = load(root, 'private-inputs.json')
    require(receipt['originals_unchanged_after_export'] is True and receipt['ambient_environment_exported'] is False, 'private original preservation contract')
    for path in root.rglob('*'):
        require(not path.is_symlink(), 'bundle symlink')
        if path.is_file():
            contents = path.read_bytes()
            require(not contents.startswith(b'\x7fELF') and b'\0' not in contents, 'binary payload not allowed')
            require(re.search(rb'(?<![A-Za-z0-9_./])/(?:home|Users)/|[A-Za-z]:\\\\', contents) is None, 'private absolute path leaked')
    if historical:
        require(sha(read(root, 'observations.json')) == OBSERVATIONS_SHA256, 'fixed historical observations changed')
        history = load(root, 'HISTORICAL.json')
        require(sha(read(root, 'HISTORICAL.json')) == HISTORICAL_SHA256, 'fixed historical inventory changed')
        for name, digest in history.items():
            require(sha(read(root, name)) == digest, 'fixed historical artifact changed: ' + name)
    if manifest:
        sealed = load(root, 'SHA256.json')
        actual = {path.relative_to(root).as_posix(): {'sha256': sha(path.read_bytes()), 'bytes': path.stat().st_size} for path in sorted(root.rglob('*')) if path.is_file() and path.name != 'SHA256.json'}
        require(actual == sealed, 'bundle SHA manifest mismatch')
    return {'native_source_files': len(native), 'native_named_tests': native_counts, 'profile_rows': 8, 'strict_rows': 28, 'abba_calls': 54, 'timed_samples': 20, 'off_median_seconds': measurements['off']['median_seconds'], 'on_median_seconds': measurements['on']['median_seconds'], 'observed_reduction_percent': improvement, 'floor_percent': 10, 'promotion': 'REJECT', 'noise': 'unknown', 'profile_ranges_us': {name: [min(values), max(values)] for name, values in ranges.items()}}


if __name__ == '__main__':
    parser = argparse.ArgumentParser()
    parser.add_argument('--manifest', action='store_true')
    args = parser.parse_args()
    print(json.dumps(validate(manifest=args.manifest), indent=2, sort_keys=True))
