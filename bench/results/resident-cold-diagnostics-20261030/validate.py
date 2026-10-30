#!/usr/bin/env python3
"""Portable strict replay of v30 diagnostic correctness, never engines or Git."""
import argparse
import difflib
import hashlib
import importlib.util
import json
import math
from pathlib import Path
import re

HERE = Path(__file__).resolve().parent
BASE_SHA = 'c68f6d4051dba968ddc2623f3dbcd013614ec6db42d2c31a7560cf0c0a688e5f'
RUNNER_SHA = 'e4ee94d7f835dc55544215399829141e4f66646c2e494c54dd4e27a54c4108cd'
OLD_MAIN = 'a76b2fb21419ff9f7dfdf173dbc1c4be64111566220f2b02d7dd282bf9ae510e'
NEW_MAIN = 'a1b78692eeb5fa95b6bb43c40e2c063f82740ae5c2e07f1293b715b3b4b9ab48'
OLD_ENGINE = '5cd25191a65a6f24452cd0c46bd779a49d6482b6685a7068c60545867708d0da'
NEW_ENGINE = 'c2606d25eb820ee515f5c5ae9d42f803916a4e3af974adb872c2fc4ba31c4e8b'
PRIVATE = {
 'rstp03': {'metadata.json': 'c84d01e54617badfaf00a8617b722bc5fb253ac00a9c74faeadfa555e71ddb37', 'records.json': 'a383766d3d66b3ce7a3db05a8f26bb76975420989a795cd66a1801fd2433d0e0', 'summary.json': 'f868217a16e6fedaf4b231aeb6bf5a37bd396304e37330e9f5e3083f90b5b9c8'},
 'p': {'metadata.json': '02e5a38ff03aa569d4617336609d9dd5c42894679cca187ce9234fa8a4041794', 'records.json': 'fd56a9356c7f351306a837addf8d6cd4886868dc252de0b72fe9ef508b5d7d56', 'summary.json': 'b186ced05706f2dad0a2fe912cd900046998c5a4d874db747398577ed78e2854'}}


def require(ok, message):
    if not ok:
        raise ValueError(message)


def sha(data):
    return hashlib.sha256(data).hexdigest()


def base(root):
    path = root / 'provenance/matrix-evidence-validator.py'
    require(sha(path.read_bytes()) == BASE_SHA, 'frozen portable helper changed')
    spec = importlib.util.spec_from_file_location('frozen_matrix_evidence_validator', path)
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module


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


def check_build(root, b, matrix):
    old, new = [(root / 'proof' / name).read_bytes() for name in ('main.rs.original', 'main.rs.frozen')]
    require(sha(old) == OLD_MAIN and sha(new) == NEW_MAIN, 'exact native source identities')
    patch = (root / 'proof/cold-diagnostics.patch').read_text()
    require(apply_patch(old.decode(), patch).encode() == new, 'source patch replay differs')
    require(patch == ''.join(difflib.unified_diff(old.decode().splitlines(keepends=True), new.decode().splitlines(keepends=True), fromfile='main.rs.original', tofile='main.rs.frozen')), 'noncanonical source patch')
    block = '            if let Some(why) = cold.refused_reached(&args) {\n                bail!("{why}");\n            }\n'
    comment = '            // Prefer the precise source diagnostics above to a generic reached-MIR\n            // refusal. Keep that refusal as a fallback before linking or execution.\n'
    old_text, new_text = old.decode(), new.decode()
    require(old_text.count(block) == new_text.count(block) == 1 and new_text.count(comment) == 1,
            'one fallback moved, not duplicated or deleted')
    require(old_text.replace(block, '', 1) == new_text.replace(comment + block, '', 1), 'native patch changed more than fallback order/comments')
    needle = 'if nothing_of_rustc && written.is_empty()'
    require(old_text.index('cold.refused_reached(&args)') < old_text.index(needle)
            and new_text.index(needle) < new_text.index('errors_for_the_daemon(&args, said)?', new_text.index(needle))
            < new_text.index('cold.refused_reached(&args)') < new_text.index('memory::report(&mut db, "build")?', new_text.index(needle)),
            'detailed check must precede fallback, which stays before linking')
    build, unit = root / 'journals/build', root / 'journals/unit'
    for path in (build / 'check.exit', build / 'compile.exit', build / 'orchestration-exit', unit / 'full-native.exit', unit / 'exit'):
        require(path.read_text().strip() == '0', 'native build/check/full359 gate failed')
    identity = (build / 'engine-identity.sha256').read_text().split()
    require(identity == [NEW_ENGINE, '/home/jeremy/.jcode/scratch/blaze-native-engine-20261030/candidate-v30-bin/jr-engine'], 'build artifact identity')
    version = (build / 'engine-version.txt').read_text()
    require('Blaze-derived origin 5d4f78f7577cf7170993b74f735099dd03f5c6ca' in version and 'build resident-v1-97212fd12b5cdb54922fcec68f797b98348695687cc91b26d75ad852ac77017f)' in version, 'version/source build identity')
    manifest = {}
    for line in (build / 'all-source-before.sha256').read_text().splitlines():
        match = re.fullmatch(r'([0-9a-f]{64})  ((?:crates|vendor)/.+)', line)
        require(match is not None and match[2] not in manifest, 'build source manifest invalid')
        manifest[match[2]] = match[1]
    require(manifest['crates/ra-jit/src/main.rs'] == NEW_MAIN and len(manifest) > 100, 'frozen main/build source manifest')
    require((unit / 'all-source-after.log').read_text().splitlines() == [name + ': OK' for name in manifest], 'all native source identities after gate')
    lock = (build / 'lock-before.sha256').read_text().split()
    require(len(lock) == 2 and re.fullmatch(r'[0-9a-f]{64}', lock[0]) and lock[1] == 'Cargo.lock'
            and (unit / 'lock-after.log').read_text().strip() == 'Cargo.lock: OK', 'lock identity after gate')
    results = {}
    for kind, run, operation in [('build', '20261030-163119597-1821357', 'check'), ('build', '20261030-163136337-1823465', 'build'), ('unit', '20261030-163303318-1833909', 'test')]:
        directory = root / 'journals' / kind / run
        meta, summary = b.load(directory / 'meta.json'), b.load(directory / 'summary.json')
        arguments = [operation, '--release', '--locked', '-p', 'ra-jit'] + (['--tests'] if operation == 'check' else ['--bin', 'jr-engine']) + ['-j', '6']
        require(meta['id'] == summary['id'] == run and meta['subcommand'] == summary['subcommand'] == operation
                and meta['args'] == summary['args'] == arguments and summary['exit'] == 0, 'journal command/status identity')
        require(meta['place'] == 'local' and meta['real_cargo'].endswith('/nightly-2026-10-27-x86_64-unknown-linux-gnu/bin/cargo')
                and meta['real_rustc'].endswith('/nightly-2026-10-27-x86_64-unknown-linux-gnu/bin/rustc'), 'native journal toolchain/route')
        output = [json.loads(line) for line in (directory / 'output.jsonl').read_text().splitlines()]
        text = '\n'.join(row['l'] for row in output)
        if operation == 'test':
            parsed = matrix.runtime.test_results(text)
            require(parsed is not None and len(parsed['tests']) == 359 and len({n for n, _ in parsed['tests']}) == 359
                    and all(status == 'ok' for _, status in parsed['tests']) and parsed['passed'] == 359
                    and parsed['failed'] == parsed['ignored'] == parsed['filtered'] == 0, 'full359 exact raw test results')
            require('359 passed, 0 failed, 0 ignored' in (unit / 'full-native.log').read_text(), 'human359 journal drift')
        results[operation] = run
    return dict(native_gates=results, tests=359, native_source_files=len(manifest), patch='exact fallback relocation, detailed check before fallback before link')


def check_run(root, run, b, matrix, source_hashes, baseline, global_nonces):
    directory = root / 'runs' / run
    meta, rows, summary = [b.load(directory / name) for name in ('metadata.sanitized.json', 'records.sanitized.json', 'summary.sanitized.json')]
    transform = b.load(directory / 'transformation.json')
    require(transform['private_original_location'] == '/home/jeremy/' + run and set(transform['environment_allowlist']) == matrix.workload.ENV_RECEIPT_ALLOWLIST, 'original run/environment scope')
    require({k: transform['private_original_sha256'][k] for k in PRIVATE[run]} == PRIVATE[run], 'original private commitments changed')
    require(set(b.files(directory)) == set(transform['derivative_sha256']) | {'transformation.json'}, 'supplemental run inventory')
    for name, digest in transform['derivative_sha256'].items():
        require(sha(b.safe(directory, name).read_bytes()) == digest, 'supplemental derivative hash')
        if not name.endswith('.sanitized.json'):
            require(transform['private_original_sha256'][name] == digest, 'raw diagnostic/log/backups transformed')
    for value in (meta, rows, summary):
        b.safe_environments(value, matrix.workload.ENV_RECEIPT_ALLOWLIST)
    require(meta['source_before'] == meta['source_after'] == source_hashes and meta['source_restored'] is True
            and meta['unchanged'] is True and meta['status'] == 'complete' and meta['raw_receipts_unchanged'] is True
            and meta['original_receipts_unchanged'] is True, 'complete source/manifest restoration/raw gates')
    require(meta['tools_before'] == meta['tools_after'] and {s: t['sha256'] for s, t in meta['tools_before'].items()} == {'reference': OLD_ENGINE, 'engine': NEW_ENGINE}, 'v28/v30 tool identity')
    require(meta['original_tools'] == baseline['tools_before'] and meta['toolchain_before'] == meta['toolchain_after'] == baseline['toolchain_before'] and b.canonical(meta['toolchain_before']) == b.TC_SHA, 'same original pinned TC/tool controls')
    require(meta['harness_before'] == meta['harness_after'], 'frozen harness after')
    require({Path(path).name: digest for path, digest in meta['harness_before'].items()} == {name: digest for name, digest in b.PROVENANCE.items() if name in ('resident_state_matrix.py', 'resident_engine_compare.py', 'resident_workload_compare.py', 'blaze_runtime_compare.py', 'blaze_edit_compare.py')}, 'original harness identities')
    require(meta['planned_calls'] == 8 and meta['variants'] == ['v28-v30'] and len(rows) == 8
            and meta['stack_meaning'] == {'reference': 'frozen v28 jr-engine', 'engine': 'candidate v30 jr-engine'}, 'v30 explicit plan/stack meanings')
    originals = {name: (root / 'input/source63' / name).read_bytes() for name in matrix.FILES}
    for name, receipt in meta['backups'].items():
        path = Path(receipt['path']).relative_to('/home/jeremy/' + run)
        require(sha(b.safe(directory, path).read_bytes()) == receipt['sha256'] == source_hashes[name], 'source/manifest durable backup')
    require(set(meta['backups']) == set(matrix.FILES), 'all durable backups including manifest')
    expected = baseline['contracts']['0:cold-prepare']['expected']
    require(expected is not None, 'original four-test contract unavailable')
    require(meta['initial_worker_logs'] == {'reference': [], 'engine': []}, 'fresh stores have no initial worker logs')
    accepted = []
    phases = ('type-error', 'type-recovery', 'type-error', 'type-recovery')
    transitions = ('initial-cold-error', 'first-recovery', 'warm-error', 'second-recovery')
    for index, row in enumerate(rows):
        stack, invocation = matrix.STACKS[index // 4], index % 4
        phase = phases[invocation]
        require(row['variant'] == 'v28-v30' and row['stack'] == stack and row['invocation'] == invocation and row['phase'] == phase and row['transition'] == transitions[invocation], 'ordered cold/recovery/warm/recovery schedule')
        nonce = row['nonce']
        require(re.fullmatch('[0-9a-f]{32}', nonce) and nonce not in global_nonces, 'unique diagnostic/runtime invocation nonce')
        global_nonces.add(nonce)
        state = matrix.inputs(originals, phase, 0, nonce)
        wanted = dict(source_hashes, **{name: sha(data) for name, data in state.items()})
        require(row['before_phase_inputs'] == wanted and row['input_unchanged'] is True and row['cwd'] == meta['source'], 'exact pinned mutation inputs')
        require(row['normalized_summary_sha256'] == sha(state['src/summary.rs'].replace(nonce.encode(), b'CANONICAL')), 'exact frozen summary recipe')
        env = row['environment']
        require(env == meta['environments'][stack], 'perstack stores reused through ordered session')
        prefix = '/home/jeremy/' + run + '/private-' + stack + '/engine/'
        original_prefix = '/home/jeremy/rsm01/s0/engine/'
        wanted_env = {key: prefix + value[len(original_prefix):] if value.startswith(original_prefix) else value
                      for key, value in baseline['environments']['0:engine'].items()}
        require(env == wanted_env, 'full same-helper private engine environment controls')
        for key in ('BLAZE_CACHE', 'BLAZE_HOME', 'JR_ENGINE_HOME', 'RA_JIT_HOME', 'RA_JIT_CACHE', 'CARGO_HOME', 'CARGO_TARGET_DIR', 'RUSTUP_HOME', 'XDG_RUNTIME_DIR', 'XDG_CACHE_HOME', 'XDG_CONFIG_HOME', 'XDG_DATA_HOME'):
            require(env[key].startswith(prefix), 'fresh disjoint private native namespaces')
        require(all(env[k] == value for k, value in matrix.runtime.PROFILES.items())
                and env['BLAZE_PROC_MACRO_SERVER'] == env['RA_JIT_PROC_MACRO_SERVER'] == '0'
                and env['BLAZE_REMOTE'] == env['RA_JIT_REMOTE'] == 'off', 'same engine profiles/native controls')
        require(row['command'] == matrix.shared.command(Path(meta['tools_before'][stack]['path']), 'engine')
                and not row['timed_out'] and row['error'] is None and row['raw_error'] is None
                and math.isfinite(row['elapsed_seconds']) and row['elapsed_seconds'] > 0, 'bounded direct engine invocation')
        text = b.raw_text(directory, run, row)
        parsed = matrix.runtime.test_results(text)
        require(b.normalized(parsed) == row['parsed'], 'strict raw parser result')
        copied = dict(row, parsed=parsed)
        failures = matrix.oracle(copied, text, expected, phase, nonce, 0)
        require(row['failures'] == failures and row['accepted'] == (not failures), 'strict nonce E0308/recovery oracle drift')
        require(row['direct_has_E0308'] == ('E0308' in text) and row['direct_has_current_nonce'] == (('JR_MATRIX_TYPE_' if phase == 'type-error' else 'JR_MATRIX_RUNTIME_') + nonce in text), 'direct diagnostic observations')
        b.worker_snapshots(directory, run, row['worker_logs'])
        for entry in row['worker_logs']:
            if 'delta' in entry:
                path = Path(entry['delta']['path']).relative_to('/home/jeremy/' + run)
                require(entry['delta_has_current_nonce'] == (nonce.encode() in b.safe(directory, path).read_bytes()), 'worker current-nonce observation')
        if phase == 'type-error':
            require(row['status'] == 101 and parsed is None and 'JR_MATRIX_RUNTIME_' not in text, 'invalid source refused without test execution')
        if run == 'rstp03' and phase == 'type-recovery':
            require(row['status'] == 101 and parsed is None and 'SUN_LEN' in text and ('is 112 bytes' if stack == 'reference' else 'is 109 bytes') in text and not row['accepted'], 'original socket path-limit failures must remain rejected')
        accepted.append(row['accepted'])
    wanted_acceptance = [False, False, False, False, True, False, True, False] if run == 'rstp03' else [False, True, True, True, True, True, True, True]
    require(accepted == wanted_acceptance, 'baseline cold rejection and candidate acceptance vector')
    require(summary['calls'] == 8 and summary['unchanged'] is True and len(summary['results']) == 8, 'summary calls/restoration')
    for row, result in zip(rows, summary['results']):
        for key in ('variant', 'stack', 'phase', 'status', 'accepted', 'failures', 'direct_has_E0308', 'direct_has_current_nonce'):
            require(row[key] == result[key], 'summary percall oracle mismatch')
    return dict(calls=8, accepted=sum(accepted), acceptance=accepted, historical_path_limit_failure=run == 'rstp03', candidate_all_four_accepted=run == 'p')


def validate(root):
    root = Path(root)
    b = base(root)
    b.manifest(root)
    evidence = b.load(root / 'evidence.json')
    require(evidence['version'] == 1 and evidence['native_fix_commit'] == '91220ff43'
            and evidence['original_matrix_bundle_commit'] == '99049bcf1' and evidence['runs'] == ['rstp03', 'p'], 'separate historical evidence provenance')
    require(sha((root / 'provenance/matrix-typeerror-v30-probe.py').read_bytes()) == RUNNER_SHA, 'same exact runner both output roots')
    require(sha((root / 'input/rsm01-metadata.sanitized.json').read_bytes()) == 'e2d1121c9ee04d2641b83a9d4125fe06c3665a9a22f5760d99aa2a13ffd2a67b', 'original matrix baseline receipt')
    matrix = b.frozen_modules(root)
    require(sha((root / 'input/tree.json').read_bytes()) == b.TREE_SHA, 'source63 pinned tree')
    tree = b.load(root / 'input/tree.json')
    require(len(tree) == 63 and set(b.files(root / 'input/source63')) == {item['path'] for item in tree}, 'complete pinned source63')
    source_hashes = {}
    for item in tree:
        data = b.safe(root / 'input/source63', item['path']).read_bytes()
        require(sha(data) == item['sha256'] and hashlib.sha1(b'blob ' + str(len(data)).encode() + b'\0' + data).hexdigest() == item['git_blob'], 'pinned source blob identity')
        source_hashes[item['path']] = item['sha256']
    for relative, receipt in evidence['receipts'].items():
        path = b.safe(root, relative)
        require(sha(path.read_bytes()) == receipt['retained_sha256'], 'supplemental receipt checksum')
        if path.suffix == '.json':
            b.safe_environments(b.load(path), matrix.workload.ENV_RECEIPT_ALLOWLIST)
        elif path.suffix == '.jsonl':
            for line in path.read_text().splitlines():
                b.safe_environments(json.loads(line), matrix.workload.ENV_RECEIPT_ALLOWLIST)
    baseline = b.load(root / 'input/rsm01-metadata.sanitized.json')
    nonces = set()
    runs = {run: check_run(root, run, b, matrix, source_hashes, baseline, nonces) for run in ('rstp03', 'p')}
    for run in runs:
        require(evidence['private_commitments'][run] == b.load(root / 'runs' / run / 'transformation.json')['private_original_sha256'], 'private original commitment inventory')
    gates = check_build(root, b, matrix)
    return dict(status='PASS', runs=runs, native_gates=gates,
        scope='Bounded initial-cold diagnostic correctness only. Original socket-limit failure and v28 initial cold rejection retained. No performance, general superiority, full state-matrix or default-promotion claim.')


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('root', type=Path, nargs='?', default=HERE)
    parser.add_argument('--seal', action='store_true')
    args = parser.parse_args()
    if args.seal:
        base(args.root).seal(args.root)
    print(json.dumps(validate(args.root), indent=2))


if __name__ == '__main__':
    main()
