#!/usr/bin/env python3
"""Portable offline semantic replay. Never calls Git, engines, builds or original paths."""
import argparse
import hashlib
import importlib.util
import json
import math
from pathlib import Path
import re
import sys

HERE = Path(__file__).resolve().parent
PROVENANCE = {
 'resident_state_matrix.py': 'be64861cbbd8295cc103b219ee9e028daa478f301be91046e8d9d4b0ae6f8358',
 'test_resident_state_matrix.py': 'a744f8f5eec9e363c9ce625b91930cf943852c07c95d9ba42f980fc7e07e21b5',
 'resident_workload_compare.py': 'e47ef667315c0044e2b0072eb43a9db68860f32b50757bdd927b49522de1e92d',
 'resident_engine_compare.py': '7da5b024344f662743b81043f4c200a23b3a49f61356e933b48257b2860c5a80',
 'blaze_runtime_compare.py': 'a7e2af863e2b1ea2878ae5c66a0f01aab1ad7fb9efe2e1027254a1269eefb568',
 'blaze_edit_compare.py': '9c451fdb939d3a8c024ddf7e2b8c2fd3a8badc7c164a429415a7c636eb8c41a8',
 'matrix-typeerror-probe.py': '9792fb4817d996a92a8228d2f6a0a8c00404cc4b6ec4a3c8d1e932245c177f45',
 'matrix-typeerror-probe-v2.py': 'fba695d20620845fdeb9223fb075519d2dc8feade11a179c8f25be8dc0f40a6a'}
PRIVATE_SHA = {
 'rsm01': {'metadata.json': 'e2d1121c9ee04d2641b83a9d4125fe06c3665a9a22f5760d99aa2a13ffd2a67b', 'records.json': '97420fff16e7dac8283668e2e261d572c9845cd8769e58f1bf3a81282f2cdeba'},
 'rstp01': {'metadata.json': '86610625e646e14808a40247488a0b05b2e74f4c4f3e8a9b85d74ae1096df953', 'records.json': '76d337931c579f1dca9eeae84e3e9a25eae8193045927e39f1924fed6e484825'},
 'rstp02': {'metadata.json': 'ba293e6f2ea2131230530e6866effc8ed64593bfff96b3704c1906eb8b939de0', 'records.json': 'e71f2cbe3d72c84f65494b0033916080ff9f25a91e597ed87b860bef8f0f69bd'}}
TOOLS = {'reference': 'fa622aeca09858c050264f701fbbebd1a14aa07d2b02c1eb4de511b684ba0263',
         'engine': '5cd25191a65a6f24452cd0c46bd779a49d6482b6685a7068c60545867708d0da'}
TREE_SHA = 'a36bc6039b80b1f03838e4c6ccf20ac96641fd46bc2b48050d24848c37bd1574'
TC_SHA = '8bdbccdd1ca3fd7f2935cfb5667047eb4c46824928823e96311d8eb5ec0a6371'


def require(ok, message):
    if not ok:
        raise ValueError(message)


def sha(data):
    return hashlib.sha256(data).hexdigest()


def normalized(value):
    return json.loads(json.dumps(value))


def canonical(value):
    return sha(json.dumps(value, sort_keys=True, separators=(',', ':')).encode())


def safe(root, name):
    name = Path(name)
    require(not name.is_absolute() and name.parts and '..' not in name.parts, 'unsafe evidence path')
    target = root / name
    require(not any((root / Path(*name.parts[:n])).is_symlink() for n in range(1, len(name.parts) + 1)), 'symlink evidence')
    return target


def files(root):
    result = {}
    for path in root.rglob('*'):
        require(not path.is_symlink(), 'symlink evidence')
        if path.is_file():
            name = path.relative_to(root).as_posix()
            if '__pycache__' in path.parts and path.suffix == '.pyc':
                continue
            if name not in ('SHA256SUMS', 'validation.json'):
                data = path.read_bytes()
                require(not data.startswith(b'\x7fELF'), 'native binary is not portable evidence')
                result[name] = sha(data)
        else:
            require(path.is_dir(), 'special evidence file')
    return result


def seal(root):
    (root / 'SHA256SUMS').write_text(''.join(f'{value}  {name}\n' for name, value in sorted(files(root).items())))


def manifest(root):
    claimed = {}
    for line in (root / 'SHA256SUMS').read_text().splitlines():
        require(bool(re.fullmatch(r'[0-9a-f]{64}  .+', line)), 'invalid checksum entry')
        value, name = line.split('  ', 1)
        safe(root, name)
        require(name not in claimed, 'duplicate checksum entry')
        claimed[name] = value
    require(claimed == files(root), 'checksum inventory/content mismatch')


def load(path):
    def unique(items):
        result = {}
        for k, v in items:
            require(k not in result, 'duplicate JSON key')
            result[k] = v
        return result
    return json.loads(path.read_text(), object_pairs_hook=unique)


def frozen_modules(root):
    # Executable helper imports happen only after independent immutable SHA checks.
    for name, digest in PROVENANCE.items():
        require(sha((root / 'provenance' / name).read_bytes()) == digest, 'frozen helper source drift: ' + name)
    names = ('blaze_edit_compare', 'blaze_runtime_compare', 'resident_engine_compare',
             'resident_workload_compare', 'resident_state_matrix')
    old = {name: sys.modules.get(name) for name in names}
    try:
        for name in names:
            spec = importlib.util.spec_from_file_location(name, root / 'provenance' / (name + '.py'))
            module = importlib.util.module_from_spec(spec)
            sys.modules[name] = module
            spec.loader.exec_module(module)
        return sys.modules['resident_state_matrix']
    finally:
        for name, module in old.items():
            if module is None:
                sys.modules.pop(name, None)
            else:
                sys.modules[name] = module


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
                    else:
                        for v in env.values():
                            if isinstance(v, dict):
                                check(v)
                check(item)
            else:
                safe_environments(item, allow)


def run_data(root, run, matrix, source_hashes):
    directory = root / 'runs' / run
    meta, rows, summary = [load(directory / name) for name in (
        'metadata.sanitized.json', 'records.sanitized.json', 'summary.sanitized.json')]
    transform = load(directory / 'transformation.json')
    require(transform['private_original_location'] == '/home/jeremy/' + run, 'original location identity')
    require(set(transform['environment_allowlist']) == matrix.workload.ENV_RECEIPT_ALLOWLIST, 'allowlist drift')
    require({k: transform['private_original_sha256'][k] for k in PRIVATE_SHA[run]} == PRIVATE_SHA[run], 'private original commitment drift')
    require(all(isinstance(v, int) and v >= 0 for v in transform['omitted'].values()), 'invalid omission counts')
    for name, digest in transform['derivative_sha256'].items():
        require(sha(safe(directory, name).read_bytes()) == digest, 'derivative receipt hash mismatch')
        if not name.endswith('.sanitized.json'):
            require(transform['private_original_sha256'][name] == digest, 'raw receipt transformed')
    require(set(files(directory)) == set(transform['derivative_sha256']) | {'transformation.json'}, 'unexpected/missing run receipt')
    for value in (meta, rows, summary):
        safe_environments(value, matrix.workload.ENV_RECEIPT_ALLOWLIST)
    require(meta['source_before'] == meta['source_after'] == source_hashes and meta['source_restored'] is True,
            'source restoration/input identity mismatch')
    require(meta['tools_before'] == meta['tools_after'] and {s: t['sha256'] for s, t in meta['tools_before'].items()} == TOOLS,
            'native tool identity mismatch')
    require(meta['toolchain_before'] == meta['toolchain_after'] and canonical(meta['toolchain_before']) == TC_SHA, 'toolchain identity drift')
    require(meta['unchanged'] is True and meta['raw_receipts_unchanged'] is True and 'error' not in meta, 'historical identity gate failed')
    require(set(meta['backups']) == set(matrix.FILES), 'missing durable source/manifest backup')
    for name, receipt in meta['backups'].items():
        local = Path(receipt['path']).relative_to(transform['private_original_location'])
        data = safe(directory, local).read_bytes()
        require(sha(data) == receipt['sha256'] == source_hashes[name], 'backup/source identity mismatch')
    return directory, meta, rows, summary, transform


def raw_text(directory, run, row):
    parts = []
    require(set(row['raw']) == {'stdout', 'stderr'}, 'missing raw stream')
    for suffix in ('stdout', 'stderr'):
        receipt = row['raw'][suffix]
        try:
            relative = Path(receipt['path']).relative_to('/home/jeremy/' + run)
        except ValueError:
            raise ValueError('raw path outside retained run') from None
        require(relative.suffix == '.' + suffix, 'raw stream suffix mismatch')
        data = safe(directory, relative).read_bytes()
        require(sha(data) == receipt['sha256'] and len(data) == receipt['bytes'], 'raw stream identity/size mismatch')
        require(len(data) <= 64 * 1024 * 1024, 'oversized raw stream')
        parts.append(data.decode(errors='replace'))
    require(row['error'] is None and row['timed_out'] is False and row.get('raw_error') is None, 'process timeout/launch/raw failure')
    require(isinstance(row['elapsed_seconds'], (int, float)) and math.isfinite(row['elapsed_seconds'])
            and row['elapsed_seconds'] > 0, 'invalid elapsed time')
    return '\n'.join(parts)


def expected_environment(matrix, stack, rep):
    base = f'/home/jeremy/rsm01/s{rep}/{stack}'
    env = dict(matrix.runtime.PROFILES, HOME='/home/jeremy', BLAZE_HOME=base + '/blaze-home',
        BLAZE_CACHE=base + '/blaze-cache', BLAZE_BACKEND='cranelift', BLAZE_OPT='0',
        RA_JIT_HOME=base + '/ra-home', RA_JIT_CACHE=base + '/ra-cache', CARGO_HOME=base + '/cargo-home',
        CARGO_TARGET_DIR=base + '/target', RUSTUP_HOME=base + '/rustup-home', BLAZE_REMOTE='off',
        RA_JIT_REMOTE='off', BLAZE_PROC_MACRO_SERVER='0', RA_JIT_PROC_MACRO_SERVER='0',
        XDG_RUNTIME_DIR=base + '/runtime', XDG_CACHE_HOME=base + '/cache',
        XDG_CONFIG_HOME=base + '/config', XDG_DATA_HOME=base + '/data')
    if stack == 'engine':
        env['JR_ENGINE_HOME'] = base + '/engine-home'
        env['BLAZE_HOME'] = env['RA_JIT_HOME'] = base + '/engine-home/resident-v1/runtime'
    return env


def check_matrix(root, matrix, originals, source_hashes):
    directory, meta, rows, summary, _ = run_data(root, 'rsm01', matrix, source_hashes)
    require(meta['status'] == 'failed/incomplete' and meta['samples'] == 5 and meta['pin'] == 'f97fcc2'
            and meta['phases'] == list(matrix.PHASES), 'failed matrix scope/status changed')
    require(meta['counts'] == {p: 10 for p in matrix.PHASES} and meta['planned_workload_calls'] == 140
            and meta['planned_version_calls'] == 10 and meta['toolchain_links_valid'] is True, 'phase counts/link gate')
    schedule = [(s, p, rep) for rep in range(5) for p in matrix.PHASES for s in matrix.runtime.order(matrix.STACKS, rep)]
    require(len(rows) == 140 and [(r['stack'], r['phase'], r['replicate']) for r in rows] == schedule, 'matrix exact phase/rotation schedule')
    require(len(meta['versions']) == 10, 'version invocation count')
    for rep in range(5):
        for stack in matrix.STACKS:
            env = expected_environment(matrix, stack, rep)
            require(meta['environments'][f'{rep}:{stack}'] == env, 'private store/profile isolation drift')
            version = meta['versions'][f'{rep}:{stack}']
            row = version['process']
            text = raw_text(directory, 'rsm01', row)
            require(row['command'] == [meta['tools_before'][stack]['path'], '--version']
                    and row['status'] == 0 and version['valid'] is True and row['environment'] == env, 'native version receipt')
            pattern = re.escape(matrix.shared.REFERENCE_VERSION) + '$' if stack == 'reference' else r'jr-engine\s+\S+'
            require(re.search('^' + pattern, text, re.M), 'native version identity')
    previous, contracts, nonces, replay = {}, {}, set(), []
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
                require(row['environment'] == expected_environment(matrix, stack, rep), 'process environment drift')
                wanted = matrix.shared.command(meta['tools_before'][stack]['path'], stack, arguments=matrix.arguments(phase))
                require(row['command'] == wanted, 'direct command/filter mismatch')
                require(row['status'] == (101 if phase in ('type-error', 'assert-failure') else 0), 'native exit status mismatch')
                text = raw_text(directory, 'rsm01', row)
                parsed = matrix.runtime.test_results(text)
                require(normalized(parsed) == row['parsed'], 'raw libtest parser disagrees with row')
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
                    require(row['accepted'] is False and row['parsed'] is None and 'HasErrors' in text
                            and 'E0308' not in text and nonce not in text, 'negative type-error case must remain REJECTED')
                else:
                    require(row['accepted'] is True, 'unexpected rejection outside ten diagnostic negatives')
    require(normalized(contracts) == meta['contracts'], 'actual reference contract drift')
    result = matrix.summarize(replay, 5, True)
    require(summary == dict(states=result, policy=matrix.POLICY, identities_unchanged=True), 'per-state median/exclusion summary drift')
    return dict(rows=140, accepted=130, rejected=10, states=result)


def worker_snapshots(directory, run, entries):
    delta_text = []
    for entry in entries:
        values = {}
        for label in ('before', 'after', 'delta'):
            if label not in entry:
                continue
            receipt = entry[label]
            relative = Path(receipt['path']).relative_to('/home/jeremy/' + run)
            data = safe(directory, relative).read_bytes()
            require(sha(data) == receipt['sha256'] and len(data) == receipt['bytes'], 'worker snapshot identity mismatch')
            values[label] = data
        before, after = values.get('before'), values.get('after')
        append = before is not None and after is not None and after.startswith(before)
        require(entry['delta_kind'] == ('append' if append else 'new/replaced/deleted'), 'worker delta provenance kind')
        delta = after[len(before):] if append else after
        require(values.get('delta') == delta, 'worker log delta bytes mismatch')
        if delta is not None:
            require(entry['delta_has_E0308'] == (b'E0308' in delta), 'worker diagnostic observation mismatch')
            delta_text.append(delta.decode(errors='replace'))
    return '\n'.join(delta_text)


def check_probe(root, run, matrix, originals, source_hashes, matrix_meta):
    directory, meta, rows, summary, _ = run_data(root, run, matrix, source_hashes)
    require(meta['status'] == 'complete' and len(rows) == 8 and meta['planned_calls'] == 8
            and meta['original_matrix'] == '/home/jeremy/rsm01' and meta['original_receipts_unchanged'] is True, 'probe identity/count/status')
    require(meta['harness_before'] == meta['harness_after'] and
            {Path(p).name: h for p, h in meta['harness_before'].items()} == {n: h for n, h in PROVENANCE.items()
                if n in ('resident_state_matrix.py', 'resident_workload_compare.py', 'resident_engine_compare.py', 'blaze_runtime_compare.py', 'blaze_edit_compare.py')},
            'probe frozen harness identity mismatch')
    variants = ['helper-present', 'helper-absent'] if run == 'rstp01' else ['canonical-fixed-uninstrumented', 'probe-only-fresh']
    require(meta['variants'] == variants and [(r['variant'], r['stack'], r['phase']) for r in rows] ==
        [(v, s, p) for v in variants for s in matrix.STACKS for p in ('type-error', 'type-recovery')], 'probe variant/control schedule')
    for entries in meta['initial_worker_logs'].values():
        worker_snapshots(directory, run, entries)
    expected = matrix_meta['contracts']['0:cold-prepare']['expected']
    nonces = set()
    for row in rows:
        phase, variant, nonce, stack = row['phase'], row['variant'], row['nonce'], row['stack']
        require(re.fullmatch(r'[0-9a-f]{32}', nonce) and nonce not in nonces, 'probe nonce reused')
        nonces.add(nonce)
        if run == 'rstp01':
            state = matrix.inputs(originals, phase, 0, nonce)
            if variant == 'helper-absent':
                state['src/summary.rs'] = state['src/summary.rs'].replace(matrix.SIGNATURE, b'', 1)
            canonical_source = state['src/summary.rs'].replace(nonce.encode(), b'CANONICAL').replace(matrix.SIGNATURE, b'', 1)
        else:
            state = dict(originals)
            if variant == 'probe-only-fresh':
                state = matrix.baseline(originals)
                state['src/summary.rs'] = state['src/summary.rs'].replace(matrix.SIGNATURE, b'', 1)
            number = (1 << 60) + int(nonce[:15], 16)
            marker = 'JR_RUNTIME_EXECUTED_' + str(number + 1)
            if phase == 'type-error':
                state['src/summary.rs'] = matrix.runtime.edit_bytes(state['src/summary.rs'], kind='type-error')
                if variant == 'probe-only-fresh':
                    state['src/summary.rs'] = state['src/summary.rs'].replace(b'JR_RUNTIME_TYPE_ERROR', ('JR_MATRIX_TYPE_' + nonce).encode(), 1)
                canonical_source = state['src/summary.rs'].replace(matrix.PROBE, b'', 1).replace(('JR_MATRIX_TYPE_' + nonce).encode(), b'JR_RUNTIME_TYPE_ERROR', 1)
            else:
                state['src/summary.rs'] = matrix.runtime.edit_bytes(state['src/summary.rs'], number)
                canonical_source = state['src/summary.rs'].replace(matrix.PROBE, b'', 1).replace(str(number + 1).encode(), b'1')
        inputs = dict(source_hashes, **{n: sha(data) for n, data in state.items()})
        require(row['before_phase_inputs'] == inputs and row['input_unchanged'] is True
                and row['normalized_summary_sha256'] == sha(canonical_source), 'probe exact source/fixture hash mismatch')
        require(row['cwd'] == meta['source'] and row['environment'] == meta['environments'][stack]
                == expected_environment(matrix, stack, 0), 'probe original helper environment drift')
        require(row['command'] == matrix.shared.command(meta['tools_before'][stack]['path'], stack), 'probe native command drift')
        require(row['status'] == (101 if phase == 'type-error' else 0), 'probe process status')
        text = raw_text(directory, run, row)
        parsed = matrix.runtime.test_results(text)
        require(normalized(parsed) == row['parsed'], 'probe raw parser mismatch')
        copied = dict(row, parsed=parsed)
        delta = worker_snapshots(directory, run, row['worker_logs'])
        for entry in row['worker_logs']:
            if 'delta' in entry:
                local = Path(entry['delta']['path']).relative_to('/home/jeremy/' + run)
                require(entry['delta_has_current_nonce'] == (nonce.encode() in safe(directory, local).read_bytes()), 'worker current-nonce observation')
        if phase == 'type-error':
            require(parsed is None and 'HasErrors' in text and 'E0308' not in text and nonce not in text
                    and row['direct_has_E0308'] is False and row['direct_has_current_nonce'] is False
                    and row['accepted'] is False, 'probe diagnostic negatives must remain rejected')
            if run == 'rstp02':
                require('cold: compiled 3988 of 3989' in delta, 'canonical/probe-only error cold-path observation missing')
                control = variant == 'canonical-fixed-uninstrumented'
                require(row['observational_control'] == control and row['direct_has_fixed_original_literal'] is False, 'control observation flags')
                if control:
                    require(row['observation_passed'] is False and row['failures'] == ['fixed canonical control is observational, not freshness-admitted'], 'fixed control must never be freshness-admitted')
                    continue
            errors = matrix.oracle(copied, text, expected, phase, nonce, 0)
            require(row['failures'] == errors, 'negative probe oracle changed')
        else:
            require(row['accepted'] is True and row['failures'] == [] and matrix.runtime.base.succeeded(row)
                    and matrix.workload.valid(parsed, expected), 'probe recovery exact test/status contract')
            if run == 'rstp01':
                require(matrix.oracle(copied, text, expected, phase, nonce, 0) == [], 'probe recovery current nonce missing')
            else:
                require(re.findall(r'JR_RUNTIME_EXECUTED_[^\s]*', text) == [marker]
                        and row['expected_recovery_marker'] == marker, 'fresh imported recovery marker')
                wanted = ['JR_MATRIX_RUNTIME_' + nonce] if variant == 'probe-only-fresh' else []
                require(re.findall(r'JR_MATRIX_RUNTIME_[^\s]*', text) == wanted, 'variant recovery probe count')
    require(summary['unchanged'] is True and summary['calls'] == 8 and len(summary['results']) == 8, 'probe summary count')
    for row, result in zip(rows, summary['results']):
        for key in ('variant', 'stack', 'phase', 'status', 'accepted', 'failures', 'direct_has_E0308', 'direct_has_current_nonce'):
            require(result[key] == row[key], 'probe summary receipt drift')
        if run == 'rstp02':
            for key in ('observational_control', 'observation_passed', 'direct_has_fixed_original_literal'):
                require(result.get(key) == row.get(key), 'observational control summary drift')
    return dict(rows=8, errors_rejected=4, recoveries_accepted=4,
                observational_controls=2 if run == 'rstp02' else 0)


def check_auxiliary(root, matrix, source_hashes, matrix_meta):
    receipts = root / 'root'
    old = load(receipts / 'full-lib-original-state.json')
    admission = load(receipts / 'full-lib-corrected-admission.json')
    require(old['status'] == 'failed/support-not-established' and old['unchanged'] is True,
            'original full-lib failure must remain historical failure')
    for key in ('source', 'toolchain', 'tools'):
        require(old[key + '_before'] == old[key + '_after'], 'full-lib identities changed')
    require(old['source_before'] == source_hashes and old['tools_before'] == matrix_meta['tools_before']
            and old['toolchain_before'] == matrix_meta['toolchain_before'], 'full-lib frozen inputs differ')
    require(admission['status'] == 'passed' and admission['original_probe_status'] == old['status']
            and admission['scope'] == 'Offline semantic admission only. Not new native execution or cold speed claim.'
            and admission['original_state_sha256'] == 'c0804f5b8c8903e9d356e00cbcd904c3b768bf1235a595efa081e4e7ae141c73'
            and admission['ignored_source_sha256'] == source_hashes['src/split_index.rs'], 'corrected admission provenance')
    ignored_source = (root / 'input/source63/src/split_index.rs').read_text()
    require(re.search(r'#\[ignore[^\n]*\]\s*fn attach_cost', ignored_source) is not None, 'pinned ignored microbenchmark declaration')
    expected = None
    require(len(old['rows']) == len(admission['rows']) == 2, 'full-lib pair missing')
    for index, stack in enumerate(matrix.STACKS):
        row, corrected = old['rows'][index], admission['rows'][index]
        streams = [(receipts / ('full-lib-' + stack + '.' + name)).read_bytes() for name in ('stdout', 'stderr')]
        for name, data in zip(('stdout', 'stderr'), streams):
            require(sha(data) == row[name + '_sha256'] == corrected[name + '_sha256'], 'full-lib raw hash mismatch')
        parsed = matrix.runtime.test_results(b'\n'.join(streams).decode(errors='replace'))
        require(normalized(parsed) == row['parsed'] and row['stack'] == corrected['stack'] == stack
                and row['status'] == corrected['status'] == 0 and row['error'] is None and not row['timed_out']
                and corrected['passed'] == 147 and corrected['ignored'] == 1, 'full-lib exact statuses/results')
        contract = matrix.contract(parsed, 'all-lib')
        require(contract is not None, 'full-lib exact names/counts/one declared ignore')
        if expected is None:
            expected = contract
        require(contract == expected, 'full-lib candidate/reference drift')
    require((receipts / 'full-lib-original.exit').read_text().strip() == '1', 'original probe failure exit changed')
    require((receipts / 'bench-728.exit').read_text().strip() == '0'
            and re.search(r'Ran 728 tests in [0-9.]+s\s+OK\s*$', (receipts / 'bench-728.log').read_text()), 'root regression log')
    cleanup = load(receipts / 'cleanup-receipt.json')
    require(cleanup['signal_or_native_stop_receipts_only'] is True and len(cleanup['rows']) == 8,
            'scoped cleanup count/type')
    for row, (rep, stack) in zip(cleanup['rows'], ((r, s) for r in range(1, 5) for s in matrix.STACKS)):
        env = matrix_meta['environments'][str(rep) + ':' + stack]
        require(row['session'] == str(rep) + ':' + stack and row['namespace'] == env['JR_ENGINE_HOME' if stack == 'engine' else 'BLAZE_HOME']
                and row['command'] == [matrix_meta['tools_before'][stack]['path'], 'stop']
                and row['status'] == 0 and row['error'] is None and row['timed_out'] is False,
                'cleanup must only stop original completed namespaces1-4')
        require(all(env.get(k) == v for k, v in row['environment'].items()), 'cleanup namespace environment drift')
        for stream in ('stdout', 'stderr'):
            require((receipts / 'cleanup' / (str(rep) + '-' + stack + '.' + stream)).is_file(), 'cleanup raw missing')
    proof = (root / 'proof/ra-jit-main-5ec494e60.rs').read_bytes()
    require(sha(proof) == 'a76b2fb21419ff9f7dfdf173dbc1c4be64111566220f2b02d7dd282bf9ae510e', 'historical native source proof changed')
    text = proof.decode()
    cold = text.index('if let Some(why) = cold.refused_reached(&args)')
    whole = text.index('if nothing_of_rustc && written.is_empty()', cold)
    detailed = text.index('errors_for_the_daemon(&args, said)?', whole)
    require(cold < text.index('bail!', cold) < whole < detailed, 'cold refusal precedes detailed whole-file check')
    return dict(full_lib_original='REJECTED original blanket-ignore guard', full_lib_corrected='offline exact148/147passed/1declared-ignore', regression_tests=728, scoped_stop_receipts=8, cold_ordering='source-order observation, not a native repair proof')


def validate(root):
    root = Path(root)
    require(root.is_dir() and not root.is_symlink(), 'invalid bundle root')
    manifest(root)
    evidence = load(root / 'evidence.json')
    require(evidence['version'] == 1 and evidence['source_commit'] == 'f97fcc2d7490ec3528a8d67657dc3f8d6d457a7d'
            and evidence['harness_commit'].startswith('5ec494e60') and evidence['runs'] == ['rsm01', 'rstp01', 'rstp02'], 'historical provenance drift')
    require(evidence['provenance'] == PROVENANCE, 'frozen provenance inventory')
    require(sha((root / 'input/tree.json').read_bytes()) == TREE_SHA, 'pinned source63 tree inventory drift')
    tree = load(root / 'input/tree.json')
    require(len(tree) == 63 and len({e['path'] for e in tree}) == 63, 'source63 file count')
    require(set(files(root / 'input/source63')) == {e['path'] for e in tree}, 'source63 unexpected file')
    source_hashes = {}
    for item in tree:
        data = safe(root / 'input/source63', item['path']).read_bytes()
        require(sha(data) == item['sha256'] and hashlib.sha1(b'blob ' + str(len(data)).encode() + b'\0' + data).hexdigest() == item['git_blob'], 'pinned source blob drift')
        source_hashes[item['path']] = item['sha256']
    matrix = frozen_modules(root)
    originals = {n: (root / 'input/source63' / n).read_bytes() for n in matrix.FILES}
    result = check_matrix(root, matrix, originals, source_hashes)
    meta = load(root / 'runs/rsm01/metadata.sanitized.json')
    probes = {run: check_probe(root, run, matrix, originals, source_hashes, meta) for run in ('rstp01', 'rstp02')}
    for run in evidence['runs']:
        transform = load(root / 'runs' / run / 'transformation.json')
        require(evidence['private_commitments'][run] == transform['private_original_sha256'], 'private commitments mismatch')
    require(evidence['counts'] == dict(matrix_rows=140, matrix_accepted=130, matrix_rejected=10, probe_rows=16, source_files=63), 'aggregate count drift')
    for label, receipt in evidence['root_receipts'].items():
        path = safe(root / 'root', label)
        require(sha(path.read_bytes()) == receipt['retained_sha256'], 'root receipt identity mismatch')
        if path.suffix == '.json':
            safe_environments(load(path), matrix.workload.ENV_RECEIPT_ALLOWLIST)
    auxiliary = check_auxiliary(root, matrix, source_hashes, meta)
    return dict(status='PASS', matrix=result, probes=probes, auxiliary=auxiliary, raw_streams=332,
        policy='Offline replay of a FAILED native matrix. Ten negative type-error cases remain rejected. Canonical controls observational only. Not a native rerun, general speedup, comprehensive/default gate or significance claim.')


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('root', type=Path, nargs='?', default=HERE)
    parser.add_argument('--seal', action='store_true')
    args = parser.parse_args()
    if args.seal:
        seal(args.root)
    print(json.dumps(validate(args.root), indent=2))


if __name__ == '__main__':
    main()
