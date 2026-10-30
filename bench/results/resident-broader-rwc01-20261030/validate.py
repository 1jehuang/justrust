#!/usr/bin/env python3
"""Relocatable offline semantic replay. Never runs retained native commands or Git."""
import argparse
import hashlib
import json
import math
from pathlib import Path
import re
import statistics

HERE = Path(__file__).resolve().parent
TREE_SHA = 'a36bc6039b80b1f03838e4c6ccf20ac96641fd46bc2b48050d24848c37bd1574'
TC_SHA = '8bdbccdd1ca3fd7f2935cfb5667047eb4c46824928823e96311d8eb5ec0a6371'
TOOLS = {'reference': 'fa622aeca09858c050264f701fbbebd1a14aa07d2b02c1eb4de511b684ba0263',
         'engine': '5cd25191a65a6f24452cd0c46bd779a49d6482b6685a7068c60545867708d0da'}
PRIVATE_SHA = {
    'rwc01': {'metadata.json': 'de1ae993296cb3cd6b42113e8e78d669346501b3eb9a57fe2272237e3c5e0161',
              'records.json': '97612d3da6e929570629914ed76f8bb23e8a4e89dca634510b7e507d7b648110'},
    'rwc02': {'metadata.json': 'c5abb9e504ef87cbbc83dde76d7b31296bfea893d733c7d32db4eac7dcc66f74',
              'records.json': '4172a4a73f8330a2cedd832eb53e9ca0d154e7640aeb7e8cb9105abc6daa288e'}}
PROVENANCE = {
    '0dba7c33e': {
        'resident_workload_compare.py': '594f29147febbef96ec48ded5548542c823c7f2a21fed953855ced305f18bf4e',
        'resident_engine_compare.py': '7da5b024344f662743b81043f4c200a23b3a49f61356e933b48257b2860c5a80',
        'blaze_runtime_compare.py': 'a7e2af863e2b1ea2878ae5c66a0f01aab1ad7fb9efe2e1027254a1269eefb568',
        'blaze_edit_compare.py': '9c451fdb939d3a8c024ddf7e2b8c2fd3a8badc7c164a429415a7c636eb8c41a8',
        'test_resident_workload_compare.py': '0b21f62976d297aee90aea570f86c8aafb06a76d2a49054509c7013dc351c347'},
    'fb9e3f14c': {
        'resident_workload_compare.py': 'e47ef667315c0044e2b0072eb43a9db68860f32b50757bdd927b49522de1e92d',
        'test_resident_workload_compare.py': '42893d9d5b4893fdf635fb3aec1d5756539ff8fe07fcb7998a3ed8a98f8d9cb4'}}
WORKLOADS = {
    'summary-test': ('src/summary.rs', 'summary::tests::', 'fn shares_overlapping_time() {',
                     'shares_overlapping_time', ['parses_libtest_results', 'shares_overlapping_time',
                     'splits_passes_without_double_counting', 'test_failures_are_not_compile_errors']),
    'shim-test': ('src/shim.rs', 'shim::tests::', 'fn describes_cargo_rustc_args() {',
                  'describes_cargo_rustc_args', ['describes_cargo_rustc_args', 'parses_time_passes_json']),
    'record-test': ('src/record.rs', 'record::tests::', 'fn finds_subcommand_after_global_flags() {',
                    'finds_subcommand_after_global_flags', ['classifies_processes',
                    'finds_subcommand_after_global_flags', 'only_records_build_like_commands',
                    'progress_config_goes_after_toolchain', 'strips_ansi_sequences']),
    'shim-function': ('src/shim.rs', 'shim::tests::', 'fn parse_pass(json: &[u8]) -> Option<Pass> {',
                      'parses_time_passes_json', ['describes_cargo_rustc_args', 'parses_time_passes_json'])}
PROFILES = {f'CARGO_PROFILE_{p}_{k}': v for p in ('DEV', 'TEST')
            for k, v in (('OPT_LEVEL', '0'), ('DEBUG', '0'), ('INCREMENTAL', 'true'))}
ALLOW = frozenset(('HOME', 'JR_ENGINE_HOME', 'RA_JIT_HOME', 'RA_JIT_CACHE', 'CARGO_HOME',
    'CARGO_TARGET_DIR', 'RUSTUP_HOME', 'BLAZE_HOME', 'BLAZE_CACHE', 'BLAZE_BACKEND', 'BLAZE_OPT',
    'BLAZE_REMOTE', 'RA_JIT_REMOTE', 'BLAZE_PROC_MACRO_SERVER', 'RA_JIT_PROC_MACRO_SERVER',
    'XDG_RUNTIME_DIR', 'XDG_CACHE_HOME', 'XDG_CONFIG_HOME', 'XDG_DATA_HOME')) | frozenset(PROFILES)


def require(ok, message):
    if not ok:
        raise ValueError(message)


def sha(data):
    return hashlib.sha256(data).hexdigest()


def canonical(data):
    return sha(json.dumps(data, sort_keys=True, separators=(',', ':')).encode())


def safe(root, name):
    relative = Path(name)
    require(not relative.is_absolute() and relative.parts and '..' not in relative.parts,
            'unsafe evidence path')
    target = root / relative
    require(not any((root / Path(*relative.parts[:i])).is_symlink()
                    for i in range(1, len(relative.parts) + 1)), 'symlink evidence path')
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
                result[name] = sha(path.read_bytes())
        else:
            require(path.is_dir(), 'special evidence input')
    return result


def seal(root):
    (root / 'SHA256SUMS').write_text(''.join(f'{digest}  {name}\n'
        for name, digest in sorted(files(root).items())))


def manifest(root):
    claimed = {}
    for line in (root / 'SHA256SUMS').read_text().splitlines():
        require(bool(re.fullmatch(r'[0-9a-f]{64}  .+', line)), 'malformed checksum line')
        digest, name = line.split('  ', 1)
        safe(root, name)
        require(name not in claimed, 'duplicate checksum entry')
        claimed[name] = digest
    require(claimed == files(root), 'checksum inventory/content mismatch')


def load(path):
    def unique(pairs):
        obj = {}
        for key, value in pairs:
            require(key not in obj, 'duplicate JSON key')
            obj[key] = value
        return obj
    return json.loads(path.read_text(), object_pairs_hook=unique)


def parse_tests(text):
    # Same libtest surface as historical runtime.test_results, independently checked.
    names = re.findall(r'^test ([^\s]+) \.\.\. (?:(?:[^\n]*\n)*?)(ok|FAILED|ignored)(?: <[0-9]+(?:\.[0-9]+)?s>)?[ \t]*$', text, re.M)
    totals = re.findall(r'test result: (ok|FAILED)\. (\d+) passed; (\d+) failed; (\d+) ignored; (\d+) measured; (\d+) filtered out', text)
    if not totals:
        require(not names, 'tests printed without totals')
        return None
    require(len(totals) == 1 and names and len(dict(names)) == len(names), 'ambiguous libtest output')
    state, passed, failed, ignored, measured, filtered = totals[0]
    require([int(passed), int(failed), int(ignored)] ==
            [sum(s == x for _, s in names) for x in ('ok', 'FAILED', 'ignored')], 'libtest counts mismatch')
    require(int(measured) == 0 and state == ('FAILED' if int(failed) else 'ok'), 'libtest state mismatch')
    return dict(tests=[list(x) for x in sorted(names)], passed=int(passed), failed=int(failed),
                ignored=int(ignored), filtered=int(filtered))


def edited(original, workload, phase, rep):
    anchor = WORKLOADS[workload][2].encode()
    require(original.count(anchor) == 1, 'ambiguous fixture anchor')
    if phase == 'type-error':
        body = 'let _: u64 = "JR_WORKLOAD_TYPE_ERROR";'
    elif phase == 'assert-failure':
        body = 'assert_eq!(std::hint::black_box(1u64), 2u64, "JR_WORKLOAD_FAILURE");'
    elif phase == 'edit':
        marker = f'JR_WORKLOAD_EXECUTED_{workload.replace("-", "_")}_{rep + 1}'
        body = (f'let observed = std::hint::black_box({rep + 1}u64); '
                f'assert_eq!(observed, {rep + 1}u64); println!("{marker}");')
    else:
        return original
    return original.replace(anchor, anchor + b'\n' + body.encode(), 1)


def expected_environment(run, stack):
    base = f'/home/jeremy/{run}/{stack}'
    env = dict(PROFILES, HOME='/home/jeremy', BLAZE_HOME=base + '/blaze-home',
               BLAZE_CACHE=base + '/blaze-cache', RA_JIT_HOME=base + '/ra-home',
               RA_JIT_CACHE=base + '/ra-cache', CARGO_HOME=base + '/cargo-home',
               CARGO_TARGET_DIR=base + '/target', RUSTUP_HOME=base + '/rustup-home',
               BLAZE_BACKEND='cranelift', BLAZE_OPT='0', BLAZE_REMOTE='off', RA_JIT_REMOTE='off',
               BLAZE_PROC_MACRO_SERVER='0', RA_JIT_PROC_MACRO_SERVER='0',
               XDG_RUNTIME_DIR=base + '/runtime', XDG_CACHE_HOME=base + '/cache',
               XDG_CONFIG_HOME=base + '/config', XDG_DATA_HOME=base + '/data')
    if stack == 'engine':
        env['JR_ENGINE_HOME'] = base + '/engine-home'
        env['BLAZE_HOME'] = env['RA_JIT_HOME'] = base + '/engine-home/resident-v1/runtime'
    return env


def check_run(root, run, source_hashes):
    directory = root / 'runs' / run
    meta = load(directory / 'metadata.sanitized.json')
    rows = load(directory / 'records.sanitized.json')
    summary = load(directory / 'summary.json')
    transform = load(directory / 'transformation.json')
    require(transform['version'] == 1 and set(transform['environment_allowlist']) == ALLOW,
            'environment transformation allowlist changed')
    require(set(transform['omitted']) == {'metadata_environment_entries', 'process_environment_entries'}
            and transform['omitted']['metadata_environment_entries'] > 0
            and all(isinstance(x, int) and x >= 0 for x in transform['omitted'].values()),
            'missing environment omission accounting')
    require(transform['private_original_location'] == f'/home/jeremy/{run}', 'private original identity')
    require(not (directory / 'metadata.json').exists() and not (directory / 'records.json').exists(),
            'unsanitized raw metadata must not be published')
    originals = transform['private_original_sha256']
    derivatives = transform['derivative_sha256']
    require({name: originals[name] for name in PRIVATE_SHA[run]} == PRIVATE_SHA[run],
            'private original metadata/row SHA commitment changed')
    require(set(originals) - {'metadata.json', 'records.json'} ==
            set(derivatives) - {'metadata.sanitized.json', 'records.sanitized.json'}, 'transformation inventory')
    for name, digest in derivatives.items():
        require(re.fullmatch(r'[0-9a-f]{64}', digest) and sha(safe(directory, name).read_bytes()) == digest,
                'derivative checksum mismatch')
        if name not in ('metadata.sanitized.json', 'records.sanitized.json'):
            require(originals[name] == digest, 'raw bytes were transformed')
    require(all(re.fullmatch(r'[0-9a-f]{64}', x) for x in originals.values()), 'private original SHA missing')
    require({p.name for p in directory.iterdir() if p.is_file()} ==
            set(derivatives) | {'transformation.json'}, 'unexpected run receipt')
    require(meta['status'] == 'complete' and meta['unchanged'] is True and meta['samples'] == 5
            and meta['pin'] == 'f97fcc2' and 'error' not in meta, 'historical run incomplete')
    require('source_restored' not in meta, 'do not manufacture historical restoration field')
    require(meta['source_before'] == meta['source_after'] == source_hashes, 'source identity drift')
    native_root = '/home/jeremy/.jcode/scratch/blaze-native-engine-20261030/'
    require(meta['tools_before']['reference']['path'] == native_root + 'reference-bin/ra-jit'
            and meta['tools_before']['engine']['path'] == native_root + 'candidate-v28-bin/jr-engine',
            'native ELF path identity changed')
    require(meta['tools_before'] == meta['tools_after']
            and {s: t['sha256'] for s, t in meta['tools_before'].items()} == TOOLS, 'engine identity drift')
    require(meta['toolchain_before'] == meta['toolchain_after']
            and canonical(meta['toolchain_before']) == TC_SHA, 'pinned toolchain identity drift')
    require(meta['profiles'] == PROFILES, 'profile drift')
    require(meta['store_policy'] == 'unseeded private stores per stack, reused across workloads'
            and meta['proc_macro_policy'] == 'both server variables explicitly 0, rustc-built proc macros',
            'cache/proc-macro policy mismatch')
    order = list(WORKLOADS) if run == 'rwc01' else list(reversed(WORKLOADS))
    require(list(meta['workloads']) == order, 'workload order mismatch')
    for workload in order:
        path, prefix, anchor, failing, _ = WORKLOADS[workload]
        require(meta['workloads'][workload] == [path, prefix, anchor, failing,
                'ordinary' if workload == 'shim-function' else 'test'], 'workload definition changed')
    require(meta['environments'] == {s: expected_environment(run, s) for s in TOOLS},
            'private cache roots or environment policy changed')
    for name, backup in meta['backups'].items():
        local = Path(backup['path']).name
        require(backup['path'] == f'/home/jeremy/{run}/{local}' and name in source_hashes,
                'backup identity/path mismatch')
        require(sha(safe(directory, local).read_bytes()) == backup['sha256'] == source_hashes[name],
                'backup/source mismatch')
    require(set(meta['backups']) == {WORKLOADS[w][0] for w in order}, 'missing source backup')
    schedule = [(s, order[0], 'version', 0) for s in TOOLS]
    for workload in order:
        schedule += [(s, workload, phase, 0) for s in TOOLS for phase in (
            'prepare', 'type-error', 'type-recovery', 'assert-failure', 'assert-recovery')]
        for rep in range(5):
            stacks = list(TOOLS)
            pivot = rep % 2
            schedule += [(s, workload, 'edit', rep) for s in stacks[pivot:] + stacks[:pivot]]
    require(len(rows) == 82 and [(r['stack'], r['workload'], r['phase'], r['replicate'])
            for r in rows] == schedule, 'exact invocation/gate/order schedule mismatch')
    contracts, measured, raw_names = {}, {}, set()
    for row in rows:
        stack, workload, phase, rep = row['stack'], row['workload'], row['phase'], row['replicate']
        path, prefix, _, failing, names = WORKLOADS[workload]
        raw_name = f'{stack}-{workload}-{phase}-{rep}'
        require(row['raw_prefix'] == f'/home/jeremy/{run}/{raw_name}', 'raw prefix identity')
        text = '\n'.join(safe(directory, raw_name + '.' + suffix).read_text(errors='replace')
                         for suffix in ('stdout', 'stderr'))
        raw_names.update(raw_name + '.' + suffix for suffix in ('stdout', 'stderr'))
        command = [meta['tools_before'][stack]['path']]
        command += ['--version'] if phase == 'version' else (
            (['cargo'] if stack == 'reference' else []) + ['test', '--lib', '--', prefix,
                                                           '--test-threads=1', '--nocapture'])
        require(row['command'] == command, 'native command/filter drift')
        require(row['error'] is None and row['timed_out'] is False and
                isinstance(row['elapsed_seconds'], (int, float)) and
                math.isfinite(row['elapsed_seconds']) and row['elapsed_seconds'] > 0,
                'process error/timeout/duration')
        expected_status = 101 if phase in ('type-error', 'assert-failure') else 0
        require(row['status'] == expected_status, 'process exit status mismatch')
        env = row['environment']
        row_keys = ALLOW - {'JR_ENGINE_HOME', 'BLAZE_REMOTE', 'RA_JIT_REMOTE',
                            'BLAZE_PROC_MACRO_SERVER', 'RA_JIT_PROC_MACRO_SERVER'}
        require(env == {k: meta['environments'][stack][k] for k in row_keys},
                'process environment drift/secret')
        require(env.get('BLAZE_BACKEND') == 'cranelift' and env.get('BLAZE_OPT') == '0', 'process backend')
        source = (root / 'input' / 'source63' / path).read_bytes()
        expected_hash = sha(edited(source, workload, phase, rep))
        require(row['input_sha256'] == row['source_sha256'] == expected_hash
                and row['input_unchanged'] is True, 'edited byte identity/anchor mismatch')
        parsed = parse_tests(text)
        require(row['parsed'] == parsed, 'raw test receipt disagrees with row')
        markers = re.findall(r'JR_WORKLOAD_EXECUTED_[^\s]*', text)
        if phase == 'version':
            version = ('blaze 0.20261027.0 (5d4f78f7577cf7170993b74f735099dd03f5c6ca)' if stack == 'reference'
                       else 'jr-engine experimental 0.20261027.0 (Blaze-derived origin 5d4f78f7577cf7170993b74f735099dd03f5c6ca; build resident-v1-c44702f2cefa696dc5b8df36636c66fcdc08bd27badbbd9dd2850848a2c8ecd1)')
            require(text.strip() == version and parsed is None, 'native version mismatch')
        elif phase == 'type-error':
            require(parsed is None and 'E0308' in text and 'JR_WORKLOAD_TYPE_ERROR' in text,
                    'deliberate type error missing')
            require(row.get('accepted') is True, 'type-error gate not accepted')
        else:
            require(parsed is not None, 'test result missing')
            expected = dict(tests=[[prefix + n, 'FAILED' if phase == 'assert-failure' and n == failing else 'ok']
                                   for n in names], passed=len(names) - (phase == 'assert-failure'),
                            failed=int(phase == 'assert-failure'), ignored=0, filtered=148 - len(names))
            require(parsed == expected, 'exact native test names/counts/failure mismatch')
            if stack == 'reference' and phase == 'prepare':
                contracts[workload] = parsed
            require(workload in contracts, 'candidate has no native reference contract')
            if phase == 'assert-failure':
                require('JR_WORKLOAD_FAILURE' in text and row.get('accepted') is True,
                        'assertion failure oracle/gate missing')
            if phase == 'edit':
                require(markers == [f'JR_WORKLOAD_EXECUTED_{workload.replace("-", "_")}_{rep + 1}']
                        and row.get('accepted') is True, 'executed unique edit marker missing/stale')
                measured.setdefault(workload, {}).setdefault(stack, []).append(row['elapsed_seconds'])
        if phase != 'edit':
            require(not markers, 'unexpected executed marker outside timed edit')
    require(meta['contracts'] == contracts, 'reference-discovered contract mismatch')
    require({p.name for p in directory.glob('*.stdout')} | {p.name for p in directory.glob('*.stderr')}
            == raw_names and len(raw_names) == 164, 'raw stdout/stderr inventory mismatch')
    calculated = {w: {s: statistics.median(times) for s, times in stacks.items()}
                  for w, stacks in measured.items()}
    require(summary['medians_seconds'] == calculated and summary['excluded'] == {w: [] for w in order},
            'median/excluded mismatch')
    return dict(rows=len(rows), timed_edits=40, raw_streams=len(raw_names),
                source_restored_derived=meta['source_before'] == meta['source_after'],
                medians_seconds=calculated, workload_order=order)


def validate(root):
    root = Path(root)
    require(root.is_dir() and not root.is_symlink(), 'invalid evidence root')
    manifest(root)
    evidence = load(root / 'evidence.json')
    require(evidence['version'] == 1 and evidence['source_commit'] ==
            'f97fcc2d7490ec3528a8d67657dc3f8d6d457a7d' and evidence['runs'] == ['rwc01', 'rwc02'],
            'evidence provenance mismatch')
    require(sha((root / 'input/tree.json').read_bytes()) == TREE_SHA, 'pinned source tree changed')
    tree = load(root / 'input/tree.json')
    require(len(tree) == 63 and len({r['path'] for r in tree}) == 63, 'source63 input inventory')
    require(set(files(root / 'input/source63')) == {entry['path'] for entry in tree},
            'unexpected source63 input file')
    source_hashes = {}
    for entry in tree:
        data = safe(root / 'input/source63', entry['path']).read_bytes()
        require(sha(data) == entry['sha256'] and hashlib.sha1(
            b'blob ' + str(len(data)).encode() + b'\0' + data).hexdigest() == entry['git_blob'],
            'pinned source blob mismatch')
        source_hashes[entry['path']] = entry['sha256']
    require({revision: info['commit'] for revision, info in evidence['historical'].items()} == {
        '0dba7c33e': '0dba7c33e453b63c5d353b75ab5b19c68b206705',
        'fb9e3f14c': 'fb9e3f14c7408ae380390fffb161d22a51078191'}, 'historical commit identity changed')
    for revision, names in PROVENANCE.items():
        require(evidence['historical'][revision]['files'] == names, 'historical helper inventory')
        for name, expected in names.items():
            require(sha((root / 'provenance' / revision / name).read_bytes()) == expected,
                    'historical harness/helper bytes changed')
    regression = safe(root, evidence['root_regression']).read_text()
    require('Ran 710 tests' in regression and re.search(r'\[stderr\] OK\s*\n', regression)
            and 'exit code: 0' in regression, 'root 710-test regression receipt missing')
    followup = load(root / 'root/followup-command.json')
    followup_log = safe(root, followup['raw_log']).read_text()
    require(followup['revision'] == 'fb9e3f14c7408ae380390fffb161d22a51078191'
            and followup['status'] == 0 and followup['tests'] == 12
            and 'Ran 12 tests' in followup_log and re.search(r'^OK$', followup_log, re.M),
            'follow-up 12-test source-only receipt missing')
    runs = {run: check_run(root, run, source_hashes) for run in evidence['runs']}
    require(evidence['expected_counts'] == dict(rows=164, timed_edits=80, raw_streams=328, source_files=63),
            'aggregate count claim mismatch')
    return dict(status='PASS', runs=runs, total_rows=164, total_timed_edits=80, raw_streams=328,
                policy='Offline semantic replay only, not a native rerun or cryptographic attestation. Four workloads, no comprehensive/default/significance claim.')


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('root', type=Path, nargs='?', default=HERE)
    parser.add_argument('--seal', action='store_true', help='refresh inventory before semantic validation')
    args = parser.parse_args()
    if args.seal:
        seal(args.root)
    print(json.dumps(validate(args.root), indent=2))


if __name__ == '__main__':
    main()
