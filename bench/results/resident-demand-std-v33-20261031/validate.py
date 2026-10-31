#!/usr/bin/env python3
"""Portable experimental demand-std evidence replay, no native calls or original paths."""
import argparse
import hashlib
import importlib.util
import json
import math
from pathlib import Path
import re

HERE = Path(__file__).resolve().parent
BASE_SHA = 'c68f6d4051dba968ddc2623f3dbcd013614ec6db42d2c31a7560cf0c0a688e5f'
EXPLICIT = {'RA_JIT_DEMAND_STD', 'RA_JIT_STATS'}
PRIVATE_SHA = {'rsm05': {'metadata.json': 'e5053d8a10333626d92300be7ac5de6cf9c3447c2ed4bffda4744bde96fe0cdb', 'records.json': '5d80ddd7d636faf6d89a133eeb4ea8802eb42ff94fd26f1404ea68ef8724eef9', 'summary.json': 'adaa6acd4ac604cf8559b5b735ce3ffffe553c6c8ceaf523395492bba0b6501d'}}
PRIVATE_SHA['rsm08'] = {'metadata.json':'4704d91f231fa272cb49affb78d8ff932399c48a354a7006395a595ec728f1c6', 'records.json':'cc40de76e7e7ac8513e5b4b21dae0d5b707a65df0071b340fb0a5173e8135782', 'summary.json':'670df98f86f2c24a8357cf502b71ce0e4b6f28d5bf88006896614db82236129d'}
TOOLS = {'reference': 'fa622aeca09858c050264f701fbbebd1a14aa07d2b02c1eb4de511b684ba0263', 'engine': '89c9445128b4ece98e9b99b0d399ac102bc68185d52ff96098d944f6f4627a20'}
OLD_MAIN = 'a1b78692eeb5fa95b6bb43c40e2c063f82740ae5c2e07f1293b715b3b4b9ab48'
MAINS = {31:'2ae62e3af2ead1dd7704280345f52f46d34d2a078c1470a3863a902f66c49a4e',32:'438becd5c6790f446a7813e51e5f0e0938a799edeefa649d6a061c75188a69ec',33:'f25af99f976a585766adeb75ee8d21a1983113cfdf3ae46d9c05536975d1ad76'}
ENGINES = {31:'194ef9bf5dd3f59a1b897f535130d46431c5c0f5af0d375b44d4856a8653ed61',32:'f5459182e969c7704d39c9c56a5d191e44802f768aaa4a85c5d75137686efa10',33:TOOLS['engine']}
SCRIPTS = {
 'build-test-candidate-v31.sh':'65053915d1a7915cc3e104c316e91f44f6ad068ebec9481df51ed1f5e6fa9072',
 'build-test-candidate-v32.sh':'38da6e8ca9d7ebb42ac8818170a62c480c7f6c5a74a59aeb191d4ea4eeff9e25',
 'build-test-candidate-v33.sh':'bf7a7c8858da2e7e776dfb9d97f54efd5aeaa511e95f609fb030762f30b4be22',
 'probe-demand-std-v31.py':'ccdbbdd41e7a960c588dff278131a1aeae8e0e676a8bc37a7018ce1656950bad',
 'probe-demand-std-v32.py':'4f857f4e3cb008f75839aab880716c7caf761eb4fb5ac122847e23d0ce7adc7f',
 'probe-demand-std-v33.py':'47ca10279aa923bec904eb39f9e46f5b29649cdb9f215de648bc017742f15904'}
JOURNALS = {31:dict(check='20261030-165501009-2656233',build='20261030-165511792-2657601',test='20261030-165610606-2664453'),
            32:dict(check='20261030-170020421-2731920',build='20261030-170030576-2733236',test='20261030-170129211-2740326'),
            33:dict(check='20261030-170434112-2802987',build='20261030-170444434-2804299',test='20261030-170543070-2811321')}


def require(ok, message):
    if not ok:
        raise ValueError(message)


def sha(data):
    return hashlib.sha256(data).hexdigest()


def safe_environments(value, allow):
    if isinstance(value, list):
        for item in value:
            safe_environments(item, allow)
    elif isinstance(value, dict):
        for key, item in value.items():
            if key in ('environment', 'environments', 'env', 'envs') and isinstance(item, dict):
                def check(env):
                    if all(isinstance(v, str) for v in env.values()):
                        require(set(env) <= allow, 'environment credential/unrecognized key retained')
                        require('RA_JIT_DEMAND_STD' not in env or env['RA_JIT_DEMAND_STD'] in ('0', '1'), 'invalid explicit demand control')
                        require('RA_JIT_STATS' not in env or env['RA_JIT_STATS'] == '1', 'invalid explicit stats control')
                    else:
                        require(all(isinstance(child, dict) for child in env.values()), 'invalid environment receipt shape')
                        for child in env.values():
                            if isinstance(child, dict):
                                check(child)
                check(item)
            else:
                safe_environments(item, allow)


def base(root):
    path = root / 'provenance/matrix-evidence-validator.py'
    require(sha(path.read_bytes()) == BASE_SHA, 'frozen portable helper drift')
    spec = importlib.util.spec_from_file_location('frozen_demand_base', path)
    b = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(b)
    b.PROVENANCE = dict(b.PROVENANCE, **{'resident_state_matrix.py':'138aa2fb7e1791de92d43c8855c6151abd188e5f57fdb4d28c22e326c54be738', 'test_resident_state_matrix.py':'09a598c8b487f9e3abdaec3750de13d9d476493cfc7595952ad76b79c27b7015'})
    b.run_data = lambda root, run, matrix, source_hashes: run_data(root, run, matrix, source_hashes, b)
    return b


def environment(b, matrix, stack, rep, run, optin):
    env = {k:v.replace('/home/jeremy/rsm01/', '/home/jeremy/' + run + '/') for k,v in b.expected_environment(matrix, stack, rep).items()}
    if optin and stack == 'engine':
        env['RA_JIT_DEMAND_STD'] = '1'
    return env


def run_data(root, run, matrix, source_hashes, b):
    load, safe, files, canonical = b.load, b.safe, b.files, b.canonical
    directory = root / 'runs' / run
    meta, rows, summary = [load(directory / name) for name in (
        'metadata.sanitized.json', 'records.sanitized.json', 'summary.sanitized.json')]
    transform = load(directory / 'transformation.json')
    require(transform['private_original_location'] == '/home/jeremy/' + run, 'original location identity')
    require(set(transform['environment_allowlist']) == (matrix.workload.ENV_RECEIPT_ALLOWLIST | EXPLICIT), 'allowlist drift')
    require({k: transform['private_original_sha256'][k] for k in PRIVATE_SHA[run]} == PRIVATE_SHA[run], 'private original commitment drift')
    require(all(isinstance(v, int) and v >= 0 for v in transform['omitted'].values()), 'invalid omission counts')
    for name, digest in transform['derivative_sha256'].items():
        require(sha(safe(directory, name).read_bytes()) == digest, 'derivative receipt hash mismatch')
        if not name.endswith('.sanitized.json'):
            require(transform['private_original_sha256'][name] == digest, 'raw receipt transformed')
    require(set(files(directory)) == set(transform['derivative_sha256']) | {'transformation.json'}, 'unexpected/missing run receipt')
    for value in (meta, rows, summary):
        safe_environments(value, (matrix.workload.ENV_RECEIPT_ALLOWLIST | EXPLICIT))
    require(meta['source_before'] == meta['source_after'] == source_hashes and meta['source_restored'] is True,
            'source restoration/input identity mismatch')
    require(meta['tools_before'] == meta['tools_after'] and {s: t['sha256'] for s, t in meta['tools_before'].items()} == TOOLS,
            'native tool identity mismatch')
    require(meta['toolchain_before'] == meta['toolchain_after'] and canonical(meta['toolchain_before']) == b.TC_SHA, 'toolchain identity drift')
    require(meta['unchanged'] is True and meta['raw_receipts_unchanged'] is True and 'error' not in meta, 'historical identity gate failed')
    require(set(meta['backups']) == set(matrix.FILES), 'missing durable source/manifest backup')
    for name, receipt in meta['backups'].items():
        local = Path(receipt['path']).relative_to(transform['private_original_location'])
        data = safe(directory, local).read_bytes()
        require(sha(data) == receipt['sha256'] == source_hashes[name], 'backup/source identity mismatch')
    return directory, meta, rows, summary, transform



def check_matrix(root, matrix, originals, source_hashes, b, run, optin):
    directory, meta, rows, summary, _ = b.run_data(root, run, matrix, source_hashes)
    require(meta.get('engine_demand_std') is optin and meta.get('startup_controls') == {'reference': {}, 'engine': {'RA_JIT_DEMAND_STD': '1'} if optin else {}}, 'explicit engine-only startup control drift')
    require(meta['status'] == 'failed/incomplete' and meta['samples'] == 5 and meta['pin'] == 'f97fcc2'
            and meta['phases'] == list(matrix.PHASES), 'failed matrix scope/status changed')
    require(meta['counts'] == {p: 10 for p in matrix.PHASES} and meta['planned_workload_calls'] == 140
            and meta['planned_version_calls'] == 10 and meta['toolchain_links_valid'] is True, 'phase counts/link gate')
    schedule = [(s, p, rep) for rep in range(5) for p in matrix.PHASES for s in matrix.runtime.order(matrix.STACKS, rep)]
    require(len(rows) == 140 and [(r['stack'], r['phase'], r['replicate']) for r in rows] == schedule, 'matrix exact phase/rotation schedule')
    require(len(meta['versions']) == 10, 'version invocation count')
    for rep in range(5):
        for stack in matrix.STACKS:
            env = environment(b, matrix, stack, rep, run, optin)
            require(meta['environments'][f'{rep}:{stack}'] == env, 'private store/profile isolation drift')
            version = meta['versions'][f'{rep}:{stack}']
            row = version['process']
            text = b.raw_text(directory, run, row)
            require(row['command'] == [meta['tools_before'][stack]['path'], '--version']
                    and row['status'] == 0 and version['valid'] is True and row['environment'] == env, 'native version receipt')
            pattern = re.escape(matrix.shared.REFERENCE_VERSION) + '$' if stack == 'reference' else r'jr-engine\s+\S+'
            require(re.search('^' + pattern, text, re.M), 'native version identity')
    previous, contracts, nonces, replay, intrinsic = {}, {}, set(), [], []
    for rep in range(5):
        expected_summary = None
        for phase in matrix.PHASES:
            pair = {}
            for stack in matrix.runtime.order(matrix.STACKS, rep):
                row = rows[len(replay)]
                nonce = row['nonce']
                require(re.fullmatch(r'[0-9a-f]{32}', nonce) and nonce not in nonces, 'nonce reused/invalid')
                nonces.add(nonce)
                state = matrix.inputs(originals, phase, rep, nonce)
                inputs = dict(source_hashes, **{n: sha(data) for n, data in state.items()})
                key = f'{rep}:{stack}'
                require(row['before_phase_inputs'] == inputs and row['previous_stack_inputs'] == previous.get(key)
                        and row['changed_inputs_since_stack_phase'] == [n for n in inputs if inputs[n] != previous.get(key, {}).get(n)],
                        'exact before-phase source/transition hash mismatch')
                previous[key] = inputs
                require(row['cache_scope'] == key and row['cwd'] == meta['source'] and row['input_unchanged'] is True,
                        'cache/source scope mismatch')
                require(row['environment'] == environment(b, matrix, stack, rep, run, optin), 'process environment drift')
                wanted = matrix.shared.command(meta['tools_before'][stack]['path'], stack, arguments=matrix.arguments(phase))
                require(row['command'] == wanted, 'direct command/filter mismatch')
                require(row['status'] == (101 if phase in ('type-error', 'assert-failure') else 0), 'native exit status mismatch')
                text = b.raw_text(directory, run, row)
                parsed = matrix.runtime.test_results(text)
                require(b.normalized(parsed) == row['parsed'], 'raw libtest parser disagrees with row')
                copied = dict(row, parsed=parsed)
                pair[stack] = copied, text, nonce
                replay.append(copied)
            reference, text, nonce = pair['reference']
            expected, count = expected_summary, None
            if phase in ('cold-prepare', 'ordinary-shim-edit', 'all-lib'):
                expected = matrix.contract(reference['parsed'], phase)
                if phase == 'cold-prepare':
                    expected_summary = expected
            if phase == 'ordinary-shim-edit':
                markers = re.findall(r'JR_WORKLOAD_EXECUTED_[^\s]*', text)
                if markers and markers == [f'JR_WORKLOAD_EXECUTED_shim_function_{rep + 1}'] * len(markers):
                    count = len(markers)
            errors = matrix.oracle(reference, text, expected, phase, nonce, rep, count)
            require(reference['failures'] == errors and reference['accepted'] == (not errors), 'reference oracle admission drift')
            if errors:
                expected = None
            if phase == 'cold-prepare' and errors:
                expected_summary = None
            contracts[f'{rep}:{phase}'] = dict(expected=expected, ordinary_invocations=count, reference_accepted=not errors)
            candidate, text, nonce = pair['engine']
            errors = matrix.oracle(candidate, text, expected, phase, nonce, rep, count)
            if not reference['accepted']:
                errors.append('reference state oracle unavailable/failed')
            require(candidate['failures'] == errors and candidate['accepted'] == (not errors), 'candidate oracle admission drift')
            for row, text, nonce in pair.values():
                if phase == 'type-error':
                    require(row['accepted'] is False and row['parsed'] is None, 'original type-error cross-engine exclusion must remain')
                    if row['stack'] == 'reference':
                        require('HasErrors' in text and 'E0308' not in text and nonce not in text, 'reference diagnostic negatives remain rejected')
                    else:
                        failures = matrix.oracle(row, text, None, phase, nonce, rep, count)
                        require(failures == [] and row['failures'] == ['reference state oracle unavailable/failed'], 'candidate intrinsic oracle must pass without relabeling cross-engine exclusion')
                        intrinsic.append(dict(replicate=rep, stack='engine', phase=phase, nonce=nonce,
                            original_cross_engine_accepted=False, original_failures=row['failures'],
                            original_status=row['status'], raw=row['raw'], intrinsic_failures=failures,
                            intrinsic_accepted=True, summary_source_sha256=row['before_phase_inputs']['src/summary.rs']))
                else:
                    require(row['accepted'] is True, 'unexpected rejection outside ten diagnostic negatives')
    require(b.normalized(contracts) == meta['contracts'], 'actual reference contract drift')
    result = matrix.summarize(replay, 5, True)
    require(summary == dict(states=result, policy=matrix.POLICY, identities_unchanged=True), 'per-state median/exclusion summary drift')
    return dict(rows=140, accepted=130, rejected=10, states=result), dict(policy='Explicit offline intrinsic-only derivative. Original cross-engine exclusions unchanged. Not native re-execution, cross-engine admission, median promotion or a complete passing matrix.', cases=intrinsic)



def apply_patch(original, patch):
    """Apply only exact ordinary unified hunks, with no subprocess or fuzzy offset."""
    old = original.splitlines(keepends=True)
    lines = patch.splitlines(keepends=True)
    require(lines[:2] == ['--- main.rs.original\n', '+++ main.rs.frozen\n'], 'patch headers')
    cursor, result, index = 0, [], 2
    while index < len(lines):
        match = re.fullmatch(r'@@ -(\d+)(?:,(\d+))? \+(\d+)(?:,(\d+))? @@\n', lines[index])
        require(match is not None, 'patch hunk header')
        start = int(match[1]) - 1
        require(start >= cursor, 'overlapping patch')
        result.extend(old[cursor:start])
        cursor, index, removed, added = start, index + 1, 0, 0
        require(len(result) == int(match[3]) - 1, 'patch new offset')
        while index < len(lines) and not lines[index].startswith('@@ '):
            line = lines[index]
            require(line[:1] in (' ', '+', '-'), 'unsupported patch line')
            if line[0] in ' -':
                require(cursor < len(old) and old[cursor] == line[1:], 'patch old context mismatch')
                cursor += 1
                removed += 1
            if line[0] in ' +':
                result.append(line[1:])
                added += 1
            index += 1
        require(removed == int(match[2] or 1) and added == int(match[4] or 1), 'patch hunk counts')
    return ''.join(result + old[cursor:])


def git_patch(original, patch):
    lines = patch.splitlines(keepends=True)
    require(lines[0].strip() == 'diff --git a/toolchain/resident/crates/ra-jit/src/main.rs b/toolchain/resident/crates/ra-jit/src/main.rs'
            and lines[1].startswith('index ') and lines[2:4] == ['--- a/toolchain/resident/crates/ra-jit/src/main.rs\n', '+++ b/toolchain/resident/crates/ra-jit/src/main.rs\n'], 'single native source git patch')
    clean = ['--- main.rs.original\n', '+++ main.rs.frozen\n']
    for line in lines[4:]:
        if line.startswith('@@ '):
            match = re.match(r'(@@ -\d+(?:,\d+)? \+\d+(?:,\d+)? @@)', line)
            require(match is not None, 'git patch hunk')
            clean.append(match[1] + '\n')
        else:
            clean.append(line)
    return apply_patch(original, ''.join(clean))


def native_gates(root, version, b, matrix):
    proof = root / 'proof' / f'v{version}'
    old = (root / 'proof/v31/main.rs.original').read_bytes()
    new = (proof / 'main.rs.candidate').read_bytes()
    require(sha(old) == OLD_MAIN and sha(new) == MAINS[version], 'exact experimental source identity')
    require(git_patch(old.decode(), (proof / 'source.patch').read_text()).encode() == new, 'source patch replay differs')
    if version > 31:
        require((proof / f'main.rs.v{version - 1}').read_bytes() == (root / f'proof/v{version - 1}/main.rs.candidate').read_bytes()
                and (proof / f'v{version - 1}.patch').read_bytes() == (root / f'proof/v{version - 1}/source.patch').read_bytes(), 'earlier experimental snapshot/patch drift')
    build, unit = root / f'journals/v{version}/build', root / f'journals/v{version}/unit'
    require((build / 'main.rs.frozen').read_bytes() == new, 'frozen compiler source differs')
    for path in (build / 'check.exit', build / 'compile.exit', build / 'orchestration-exit', unit / 'full-native.exit', unit / 'exit'):
        require(path.read_text().strip() == '0', 'default native check/build/full359 failed')
    identity = (build / 'engine-identity.sha256').read_text().split()
    require(identity == [ENGINES[version], f'/home/jeremy/.jcode/scratch/blaze-native-engine-20261030/candidate-v{version}-bin/jr-engine'], 'exact build artifact identity')
    source = {}
    for line in (build / 'all-source-before.sha256').read_text().splitlines():
        match = re.fullmatch(r'([0-9a-f]{64})  ((?:crates|vendor)/.+)', line)
        require(match is not None and match[2] not in source, 'native source manifest')
        source[match[2]] = match[1]
    require(len(source) == 3757 and source['crates/ra-jit/src/main.rs'] == MAINS[version], 'complete native source identities')
    require((unit / 'all-source-after.log').read_text().splitlines() == [name + ': OK' for name in source], 'native source after gate differs')
    lock = (build / 'lock-before.sha256').read_text().split()
    require(len(lock) == 2 and re.fullmatch(r'[0-9a-f]{64}', lock[0]) and lock[1] == 'Cargo.lock'
            and (unit / 'lock-after.log').read_text().strip() == 'Cargo.lock: OK', 'native lock restoration')
    operations = {}
    for parent in (build, unit):
        for directory in sorted(path for path in parent.iterdir() if path.is_dir()):
            meta, summary = [b.load(directory / name) for name in ('meta.json', 'summary.json')]
            operation = meta['subcommand']
            require(operation in ('check', 'build', 'test') and operation not in operations, 'native operation inventory')
            arguments = [operation, '--release', '--locked', '-p', 'ra-jit'] + (['--tests'] if operation == 'check' else ['--bin', 'jr-engine']) + ['-j', '6']
            require(meta['id'] == summary['id'] == directory.name and summary['subcommand'] == operation and meta['args'] == summary['args'] == arguments
                    and summary['exit'] == 0 and meta['place'] == 'local', 'native journal command/status')
            require(meta['real_cargo'].endswith('/nightly-2026-10-27-x86_64-unknown-linux-gnu/bin/cargo')
                    and meta['real_rustc'].endswith('/nightly-2026-10-27-x86_64-unknown-linux-gnu/bin/rustc'), 'native gate toolchain')
            output = [json.loads(line) for line in (directory / 'output.jsonl').read_text().splitlines()]
            text = '\n'.join(row['l'] for row in output)
            if operation == 'test':
                parsed = matrix.runtime.test_results(text)
                require(parsed is not None and len(parsed['tests']) == 359 and len({n for n, _ in parsed['tests']}) == 359
                        and all(status == 'ok' for _, status in parsed['tests']) and parsed['passed'] == 359
                        and parsed['failed'] == parsed['ignored'] == parsed['filtered'] == 0, 'full359 exact raw results')
                require('359 passed, 0 failed, 0 ignored' in (unit / 'full-native.log').read_text(), 'human full359 gate')
            operations[operation] = directory.name
    require(operations == JOURNALS[version], 'all exact default native gate journals required')
    return dict(default_native_gates=operations, tests=359, source_files=len(source), engine_sha256=ENGINES[version])


def legacy_probe(root, version, b, matrix, source_hashes):
    directory = root / f'probes/v{version}'
    state = b.load(directory / 'state.sanitized.json')
    require(state['source_before'] == state['source_after'] == source_hashes and state['unchanged'] is True, 'legacy probe source restoration')
    require(state['toolchain_before'] == state['toolchain_after'] and b.canonical(state['toolchain_before']) == b.TC_SHA, 'legacy probe toolchain identity')
    require(state['tool_before'] == state['tool_after'] and state['tool_before'] == ENGINES[version], 'legacy probe engine identity')
    require(state['status'] == ('passed' if version == 33 else 'failed'), 'failed opt-in prototype must not be promoted')
    require(len(state['rows']) == 2 and [row['mode'] for row in state['rows']] == ['off', 'on'], 'off/on probe order')
    runner = (root / f'provenance/probe-demand-std-v{version}.py').read_text()
    require(re.search(r"env\.update\(RA_JIT_STATS\s*=\s*'1',\s*RA_JIT_DEMAND_STD\s*=\s*'1' if mode\s*==\s*'on' else '0'\)", runner), 'explicit legacy script control provenance')
    result = []
    for row in state['rows']:
        mode = row['mode']
        require(not row['timed_out'] and row['error'] is None and math.isfinite(row['elapsed_seconds']) and row['elapsed_seconds'] > 0, 'bounded legacy probe')
        # Older helper did not serialize these controls. Do not invent receipt values.
        require(not (EXPLICIT & set(row['environment'])), 'legacy serialized-control absence changed')
        safe_environments({'environment':row['environment']}, matrix.workload.ENV_RECEIPT_ALLOWLIST)
        raw = [(directory / f'{mode}.{suffix}').read_bytes() for suffix in ('stdout','stderr')]
        require(all(len(data) <= matrix.MAX_RAW_BYTES for data in raw), 'legacy raw bound')
        text = '\n'.join(data.decode(errors='replace') for data in raw)
        passed = matrix.shared.valid_tests(matrix.runtime.test_results(text))
        wanted = mode == 'off' or version == 33
        require(row['status'] == (0 if wanted else 101) and row['accepted'] is wanted and passed is wanted, 'legacy native result disagrees with raw')
        workers = sorted(directory.glob(f'{mode}-worker-*.log'))
        require(workers and all(path.stat().st_size <= matrix.MAX_RAW_BYTES for path in workers), 'bounded worker profiles missing')
        worker_text = '\n'.join(path.read_text(errors='replace') for path in workers)
        if mode == 'on' and version == 31:
            require('link' in text.lower() and '__rdl_alloc' in worker_text + text, 'v31 unresolved allocator/link negative missing')
        if mode == 'on' and version == 32:
            require('Duplicate definition of identifier: __rust_panic_cleanup' in text + worker_text, 'v32 duplicate panic cleanup negative missing')
        result.append(dict(mode=mode, status=row['status'], accepted=row['accepted'], elapsed_seconds=row['elapsed_seconds'],
                           control_provenance='Retained script+mode only. Legacy environment receipt omitted demand/std stats keys.'))
    return dict(status=state['status'], rows=result)


def aborted_launches(root, b, matrix, source_hashes):
    directory = root / 'runs/rsm03'
    meta, rows, transform = [b.load(directory / name) for name in ('metadata.sanitized.json','records.sanitized.json','transformation.json')]
    require(meta.get('engine_demand_std') is False and meta['status'] == 'incomplete' and len(rows) == 1
            and rows[0]['stack'] == 'reference' and rows[0]['phase'] == 'cold-prepare' and rows[0].get('status') is None
            and rows[0].get('accepted') is None, 'aborted default-mode observation drift')
    require(not any(name in meta for name in ('source_after','source_restored','unchanged')) and not (directory / 'summary.sanitized.json').exists(), 'fabricated aborted finally/summary')
    require(transform['private_original_sha256']['metadata.json'] == '15081dabe5a168c40860605bbdef3e569775b753d20b7fa33070b6bae15ecbfd'
            and transform['private_original_sha256']['records.json'] == '3c5077fe395b6ba96f1bea99761b2159da98fb57ba500d483a641d859f8188cc', 'aborted original commitment')
    for name, digest in transform['derivative_sha256'].items():
        require(sha(b.safe(directory,name).read_bytes()) == digest, 'aborted derivative hash drift')
        if not name.endswith('.sanitized.json'):
            require(transform['private_original_sha256'][name] == digest, 'aborted raw transformed')
    require(set(b.files(directory)) == set(transform['derivative_sha256']) | {'transformation.json'}, 'aborted receipt inventory')
    require(meta['source_before'] == source_hashes and set(meta['backups']) == set(matrix.FILES), 'aborted durable backups missing')
    for name, receipt in meta['backups'].items():
        relative = Path(receipt['path']).relative_to('/home/jeremy/rsm03')
        require(sha(b.safe(directory,relative).read_bytes()) == receipt['sha256'] == source_hashes[name], 'aborted backup identity')
    for value in (meta, rows):
        safe_environments(value, matrix.workload.ENV_RECEIPT_ALLOWLIST | EXPLICIT)
    text = (root / 'launches/rsm04.log').read_text()
    require("source does not match f97fcc2: b'src/shim.rs'" in text and 'Command finished with exit code: 1' in text, 'pristine admission rejection missing')
    return dict(rsm03='Aborted default-mode, one incomplete call, no original finally/exit attestation.',
                rsm04='Pristine source pin rejection before workload calls. No output directory.')


def negative_controls(root, b, matrix, source_hashes):
    pins = {
        'rsm06': {'metadata.json':'0a43d423a3369e4b35ba6d80e82366903208fdd8a8ec53c43cf73ed4c5efa221', 'records.json':'bc4d5587a001d876a6a61c135cb9c93593db2724bf6fdcbe4ef770352ab22e5a', 'summary.json':'cb81444460ac67aa4231445a2bba6091201024056068de2d55a9b5bb147b5bce'},
        'rsm07': {'metadata.json':'ac680d214850cdd9d0f86c644bc6a389664273f8cf45504d12aec85d62170baf', 'records.json':'0c68a10c79d1843b014b31487753fa122800bcdcac277549cebc74fcf308ecf5'}}
    result = {}
    for run in ('rsm06', 'rsm07'):
        directory = root / 'runs' / run
        meta, rows, transform = [b.load(directory / name) for name in ('metadata.sanitized.json', 'records.sanitized.json', 'transformation.json')]
        require({name:transform['private_original_sha256'][name] for name in pins[run]} == pins[run], 'negative control original commitment')
        require(meta['engine_demand_std'] is False and meta['source_before'] == source_hashes, 'negative default-control source/scope')
        require(set(transform['environment_allowlist']) == matrix.workload.ENV_RECEIPT_ALLOWLIST | EXPLICIT, 'negative receipt allowlist')
        for name,digest in transform['derivative_sha256'].items():
            path = b.safe(directory,name)
            require(sha(path.read_bytes()) == digest, 'negative receipt hash')
            if not name.endswith('.sanitized.json'):
                require(transform['private_original_sha256'][name] == digest, 'negative raw transformed')
            else:
                safe_environments(b.load(path), matrix.workload.ENV_RECEIPT_ALLOWLIST | EXPLICIT)
        require(set(b.files(directory)) == set(transform['derivative_sha256']) | {'transformation.json'}, 'negative receipt inventory')
        require(set(meta['backups']) == set(matrix.FILES), 'negative original backup missing')
        for name,receipt in meta['backups'].items():
            require(sha(b.safe(directory,Path(receipt['path']).relative_to('/home/jeremy/' + run)).read_bytes()) == receipt['sha256'] == source_hashes[name], 'negative backup identity')
        if run == 'rsm06':
            require(meta['status'] == 'failed/incomplete' and len(rows) == 140 and all(row.get('accepted') is False for row in rows), 'unsupported control must remain fully rejected')
            require(all(tool['sha256'] == ENGINES[33] for tool in meta['tools_before'].values()), 'unsupported same-engine reference identity')
            require(all(not value['valid'] if key.endswith(':reference') else value['valid'] for key,value in meta['versions'].items()), 'unsupported reference version negative')
            require(meta['source_restored'] is True and meta['source_before'] == meta['source_after'] and meta['unchanged'] is True, 'unsupported run restoration')
            require('Command finished with exit code: 1' in (root / 'launches/rsm06.log').read_text(), 'unsupported launch exit')
            result[run] = dict(rows=140,accepted=0,scope='Unsupported same-engine reference identity/command control, never attribution.')
        else:
            require(meta['status'] == 'incomplete' and len(rows) == 128 and sum(row.get('accepted') is None for row in rows) == 2, 'incomplete timeout rows drift')
            require(not any(key in meta for key in ('source_after','source_restored','unchanged')) and not (directory / 'summary.sanitized.json').exists(), 'fabricated timeout finally/medians')
            require({s:t['sha256'] for s,t in meta['tools_before'].items()} == TOOLS and all(v['valid'] for v in meta['versions'].values()), 'proper reference identity, but incomplete run')
            require('Command timed out after 600000ms' in (root / 'launches/rsm07.log').read_text(), 'outer timeout original observation')
            texts = []
            for row in rows:
                for receipt in row.get('raw',{}).values():
                    data = b.safe(directory,Path(receipt['path']).relative_to('/home/jeremy/' + run)).read_bytes()
                    require(len(data) <= matrix.MAX_RAW_BYTES and sha(data) == receipt['sha256'] and len(data) == receipt['bytes'], 'incomplete immutable raw receipt')
                    texts.append(data.decode(errors='replace'))
            require(any('signal: 15' in text or 'signal 15' in text for text in texts), 'resource-pressure/signal observation missing')
            result[run] = dict(rows=128,scope='Incomplete outer timeout and resource-pressure failures. No summary/medians or attribution.')
    stop = b.load(root / 'cleanup/receipt.json')
    require(len(stop['rows']) == 32 and sum(row['status'] == 0 for row in stop['rows']) == 27
            and sum(row['status'] == 1 for row in stop['rows']) == 5, 'scoped native stop outcomes')
    sessions = set()
    for row in stop['rows']:
        run, session = row['matrix'],row['session']
        rep,stack = session.split(':')
        require((run,session) not in sessions and run in ('rsm03','rsm05','rsm06','rsm07') and stack in matrix.STACKS and rep in (['0'] if run == 'rsm03' else ['0','1','2','3','4']), 'stop namespace outside exact completed sessions')
        sessions.add((run,session))
        meta = b.load(root / f'runs/{run}/metadata.sanitized.json')
        require(row['command'] == [meta['tools_before'][stack]['path'],'stop'] and row['namespace'] == meta['environments'][session]['JR_ENGINE_HOME' if stack == 'engine' else 'BLAZE_HOME'], 'scoped stop command/namespace')
        label = f'{run}-{rep}-{stack}'
        for suffix in ('stdout','stderr'):
            require((root / 'cleanup' / (label + '.' + suffix)).is_file(), 'native stop raw missing')
    result['cleanup'] = dict(calls=32,status0=27,status1=5,scope='Native stop outcomes only. No stores deleted, not universal process-exit attestation.')
    return result


def validate(root, write_derivative=False):
    root = Path(root)
    b = base(root)
    b.manifest(root)
    evidence = b.load(root / 'evidence.json')
    require(evidence['version'] == 1 and evidence['source_commit'] == 'f97fcc2' and evidence['harness_commit'] == '9e6f21120', 'frozen harness provenance')
    require(evidence['rsm04_private_output_exists'] is False, 'pristine rejected launch output fabrication')
    publication = evidence['native_source_publication']
    require(publication['commit'] == '9f3bab61e' and publication['main_git_blob'] == 'ce0f6796092661bd2bfc51fca8c77b23e266c8da', 'later source publication identity')
    published = (root / 'proof/v33/main.rs.candidate').read_bytes()
    require(hashlib.sha1(b'blob ' + str(len(published)).encode() + b'\0' + published).hexdigest() == publication['main_git_blob'], 'published source blob differs from frozen build')
    require(sha((root / 'input/tree.json').read_bytes()) == b.TREE_SHA, 'pinned source tree identity')
    tree = b.load(root / 'input/tree.json')
    require(len(tree) == 63 and set(b.files(root / 'input/source63')) == {item['path'] for item in tree}, 'complete pinned source63 inventory')
    source_hashes = {}
    for item in tree:
        data = b.safe(root / 'input/source63', item['path']).read_bytes()
        require(sha(data) == item['sha256'] and hashlib.sha1(b'blob ' + str(len(data)).encode() + b'\0' + data).hexdigest() == item['git_blob'], 'pinned source63 blob drift')
        source_hashes[item['path']] = item['sha256']
    matrix = b.frozen_modules(root)
    for name,digest in SCRIPTS.items():
        require(sha((root / 'provenance' / name).read_bytes()) == digest, 'frozen build/probe script drift')
    allow = matrix.workload.ENV_RECEIPT_ALLOWLIST | EXPLICIT
    for name, receipt in evidence['receipts'].items():
        path = b.safe(root, name)
        require(sha(path.read_bytes()) == receipt['retained_sha256'], 'supplemental receipt checksum')
        if path.suffix not in ('.json','.jsonl'):
            require(receipt['original_sha256'] == receipt['retained_sha256'], 'selected raw/source transformed')
        if path.suffix == '.json':
            safe_environments(b.load(path), allow)
        elif path.suffix == '.jsonl':
            for line in path.read_text().splitlines():
                safe_environments(json.loads(line), allow)
    originals = {name:(root / 'input/source63' / name).read_bytes() for name in matrix.FILES}
    results, derivatives = {}, {}
    for run, optin in [('rsm05',True),('rsm08',False)]:
        result, derivative = check_matrix(root,matrix,originals,source_hashes,b,run,optin)
        require(len(derivative['cases']) == 5, 'all five intrinsic diagnostic observations required')
        require(evidence['private_commitments'][run] == b.load(root / 'runs' / run / 'transformation.json')['private_original_sha256'], 'private original commitment inventory')
        results[run], derivatives[run] = result, derivative
    path = root / 'intrinsic-typeerror.json'
    if write_derivative:
        path.write_text(json.dumps(derivatives,indent=2)+'\n')
    else:
        require(b.load(path) == derivatives, 'separate unchanged-oracle intrinsic derivative drift')
    gates = {str(v):native_gates(root,v,b,matrix) for v in (31,32,33)}
    probes = {str(v):legacy_probe(root,v,b,matrix,source_hashes) for v in (31,32,33)}
    launches = aborted_launches(root,b,matrix,source_hashes)
    launches.update(negative_controls(root,b,matrix,source_hashes))
    slim = {}
    for version in (31,32,33):
        for kind, operations in [('build',('check','build')),('unit',('test',))]:
            path = root / f'slim/native-v{version}-{kind}/2026-10.jsonl'
            rows = [json.loads(line) for line in path.read_text().splitlines()]
            require(len(rows) == len(operations) and {r['id'] for r in rows} == {JOURNALS[version][op] for op in operations}, 'slim native gate ID inventory')
            for row in rows:
                require(row['exit'] == 0 and row['repo'] == 'private' and row['args'] == [row['subcommand']]
                        and row['machine'] == f'native-v{version}-{kind}' and row['route'] == 'JUSTRUST_REMOTE=off', 'slim private gate command/status/route')
                require(row['id'] == JOURNALS[version][row['subcommand']], 'slim gate operation identity')
                if row['subcommand'] == 'test':
                    require(row['tests']['passed'] == 359 and row['tests']['failed'] == row['tests']['ignored'] == row['tests']['filtered_out'] == 0, 'slim full359 result')
                slim[row['id']] = row['subcommand']
    require(len(slim) == 9, 'all nine slim exports required')
    regression = (root / 'regression/root733.log').read_text()
    require(re.search(r'\[stderr\] Ran 733 tests in 18\.485s\n\[stderr\] \n\[stderr\] OK', regression)
            and 'Command finished with exit code: 0' in regression, 'actual separate Python733 regression receipt')
    return dict(status='PASS', matrices=results, intrinsic_only={run:dict(cases=5,accepted=5,original_cross_engine_admitted=0,typeerror_medians_excluded=True) for run in results},
        native_gates=gates, legacy_probes=probes, unsuccessful_launches=launches, slim_native_gates=slim, root_python_regression=733,
        scope='Experimental opt-in only. Original failures/exclusions unchanged. Mixed per-state medians, no aggregate weighting, significance, general faster-than-Blaze, comprehensive synthetic gate or default promotion.')


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('root',type=Path,nargs='?',default=HERE)
    parser.add_argument('--seal',action='store_true')
    parser.add_argument('--derive',action='store_true')
    args = parser.parse_args()
    b = base(args.root)
    if args.derive:
        b.seal(args.root)
        validate(args.root,write_derivative=True)
        b.seal(args.root)
    elif args.seal:
        b.seal(args.root)
    print(json.dumps(validate(args.root),indent=2))


if __name__ == '__main__':
    main()
