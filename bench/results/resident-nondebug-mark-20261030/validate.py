#!/usr/bin/env python3
"""Standalone offline rejection audit. Never executes native tools or historical paths."""
import argparse
import hashlib
import json
import math
from pathlib import Path
import re
import statistics
import tomllib

EXCLUDED = {'manifest.json', 'audit.json', 'offline-tests.log'}
FIXTURE = 'source-ready/v29b/native-fixtures/'
PATCH_PATH = 'toolchain/resident/crates/hir-ty-codegen/patch.rs'
CARGO_PATH = 'toolchain/resident/crates/hir-ty-codegen/Cargo.toml'
SOURCE_HASHES = {
    'patch-original': 'f5bbdae3988e42aaef61a0d5f1edda759c29775491b800ad4cfde13cc116f9a2',
    'patch-v29': '0bf66d7539d51f03a04551aaeac9122a216030f1597a7d6d4e3ca1a253bb79d4',
    'patch-v29b-initial': '2850a2f5db8d7ec65bd06eb751ecec3b172f71df9a5f823e98f15bc95e40dd4d',
    'patch-v29b-final': '959dacd0e3a6ece56514b98175f828e5ec3e0e41af826ee42ceb600ba141771e',
    'cargo-original': 'f46dec5b2cf06f30456f2379967d4bb1747c9e690ae35a1861d3775c95eab381',
    'cargo-final': '443ba74bc8d7cbdc902c3fba1d1efc2cecc8a1772db9dfb2fd83393de6601774',
    'lock-original': 'bf3307542b2b5c0e7a4b8c5b1a1e650bba9afece80284cc497a3800560a0c2bc',
}
ENGINE_HASHES = {'rsb20': '5cd25191a65a6f24452cd0c46bd779a49d6482b6685a7068c60545867708d0da',
                 'rsa26': '87fbace90bb07285f7d96ef5583fc353743f6c9b01413652e0899070d60b43e9'}
BUILD_IDS = {'rsb20': 'c44702f2cefa696dc5b8df36636c66fcdc08bd27badbbd9dd2850848a2c8ecd1',
             'rsa26': '31aaa3469f9b5ad729591ac662441cb75db29515b3342598436a4460bb200931'}
REFERENCE_HASH = 'fa622aeca09858c050264f701fbbebd1a14aa07d2b02c1eb4de511b684ba0263'
TEST_NAMES = ['parses_libtest_results', 'shares_overlapping_time',
              'splits_passes_without_double_counting', 'test_failures_are_not_compile_errors']
NATIVE = {
    '20261030-161531907-1577564': ('mark', 4, 46, '6e2ba0368f4db57509c51f452c106a2ec06afcf3b254e8e5484102cf267f60da'),
    '20261030-161551972-1579958': ('codegen-full', 50, 0, '73e8bc923a35a1c3ea4f27ce8fddcc55ec2027e66bc80d096f3169a4a606d883'),
    '20261030-161552243-1580059': ('full-native', 359, 0, '947ddd3d7c1da2265fa8cd93a2f60497ed7f17c61fe55fffcda1ff70ba3f5526'),
}


class InvalidEvidence(ValueError):
    pass


def need(value, message):
    if not value:
        raise InvalidEvidence(message)


def sha(data):
    return hashlib.sha256(data).hexdigest()


def canonical(value):
    return sha(json.dumps(value, sort_keys=True, separators=(',', ':')).encode())


class Evidence:
    def __init__(self, root, overrides=None):
        self.root = Path(root)
        self.overrides = overrides or {}

    def read(self, relative):
        p = Path(relative)
        need(not p.is_absolute() and '..' not in p.parts, 'unsafe artifact path')
        if relative in self.overrides:
            need(self.overrides[relative] is not None, f'missing artifact: {relative}')
            return self.overrides[relative]
        target = self.root / p
        need(not any(part.is_symlink() for part in [target, *target.parents] if part != self.root.parent),
             'symlink artifact path')
        try:
            return target.read_bytes()
        except OSError as error:
            raise InvalidEvidence(f'missing artifact: {relative}') from error

    def text(self, path):
        return self.read(path).decode()

    def obj(self, path):
        return json.loads(self.text(path))


def verify_inventory(e):
    inventory = e.obj('manifest.json')['sha256']
    actual = {str(p.relative_to(e.root)) for p in e.root.rglob('*')
              if p.is_file() and p.name not in EXCLUDED and '__pycache__' not in p.parts}
    need(set(inventory) == actual and len(inventory) >= 150, 'incomplete artifact inventory')
    for path, digest in inventory.items():
        need(sha(e.read(path)) == digest, f'checksum mismatch: {path}')
    return len(inventory)


def replay_patch(original, patch, source):
    lines = patch.splitlines(keepends=True)
    old = original.splitlines(keepends=True)
    start = lines.index(f'--- a/{source}\n')
    need(lines[start + 1] == f'+++ b/{source}\n', 'patch source mismatch')
    out, cursor, i = [], 0, start + 2
    while i < len(lines):
        hunk = re.match(r'@@ -(\d+)(?:,(\d+))? \+(\d+)(?:,(\d+))? @@', lines[i])
        need(hunk is not None, 'invalid patch hunk')
        at = int(hunk[1]) - 1
        need(at >= cursor, 'overlapping patch hunks')
        out.extend(old[cursor:at])
        cursor = at
        old_count = new_count = 0
        i += 1
        while i < len(lines) and lines[i][:1] in (' ', '+', '-'):
            line = lines[i]
            if line[0] in (' ', '-'):
                need(cursor < len(old) and old[cursor] == line[1:], 'patch original context mismatch')
                cursor += 1
                old_count += 1
            if line[0] in (' ', '+'):
                out.append(line[1:])
                new_count += 1
            i += 1
        need(old_count == int(hunk[2] or 1) and new_count == int(hunk[4] or 1), 'patch hunk count mismatch')
    out.extend(old[cursor:])
    return ''.join(out)


def hash_lines(text):
    result = {}
    for line in text.splitlines():
        digest, path = line.split(None, 1)
        need(path not in result, 'duplicate source hash path')
        result[path] = digest
    return result


def verify_source(e):
    variants = [('source-ready/v29/', 'patch-v29'),
                ('source-ready/v29b/', 'patch-v29b-initial'), (FIXTURE, 'patch-v29b-final')]
    for prefix, key in variants:
        for path, digest in hash_lines(e.text(prefix + 'SHA256SUMS')).items():
            need(sha(e.read(prefix + path)) == digest, 'original source-ready inventory mismatch')
        original, candidate = e.text(prefix + 'patch.rs.original'), e.text(prefix + 'patch.rs.candidate')
        need(sha(original.encode()) == SOURCE_HASHES['patch-original'], 'immutable original source identity')
        need(sha(candidate.encode()) == SOURCE_HASHES[key], 'immutable candidate source identity')
        need(replay_patch(original, e.text(prefix + 'patch.diff'), PATCH_PATH) == candidate,
             'patch fails exact candidate reconstruction')
    for name, key in [('Cargo.toml.original', 'cargo-original'), ('Cargo.toml.candidate', 'cargo-final')]:
        need(sha(e.read(FIXTURE + name)) == SOURCE_HASHES[key], 'immutable manifest identity')
    need(replay_patch(e.text(FIXTURE + 'Cargo.toml.original'), e.text(FIXTURE + 'Cargo.diff'), CARGO_PATH)
         == e.text(FIXTURE + 'Cargo.toml.candidate'), 'manifest patch reconstruction')
    candidate = e.text(FIXTURE + 'patch.rs.candidate')
    providers = candidate.split('fn root_mark(', 1)[1].split('/// Something in the base', 1)[0]
    need('lru =' not in providers and candidate.count('#[salsa::tracked(returns(copy))]\nfn root_mark') == 1,
         'final default mark policy changed')
    need('#[salsa::tracked(returns(copy))]\nfn instance_mark' in candidate, 'instance policy changed')
    need('lru = 4096' in e.text('source-ready/v29/patch.rs.candidate') and
         'lru = 32768' in e.text('source-ready/v29/patch.rs.candidate'), 'unbuilt LRU rejection lost')
    ready = e.obj(FIXTURE + 'ready-v29b.json')
    need(sha(e.read('source-ready/v29/READY.json')) == '632a45034f09765e49c4f428220cfda5461269a8d1564a85d993389ffa8e7f5d',
         'immutable unbuilt v29 readiness identity')
    need(ready['native_builds_tests_workloads_run'] is False and ready['provider_avoidance_proven'] is False,
         'source readiness retconned as actual proof')
    need(sha(e.read(FIXTURE + 'ready-v29b.json')) == 'ca727f168d9419891bd355287e2ddac68106075d62f4e56d3a85b7b7a6561f08',
         'immutable final source readiness identity')
    need(ready['candidate_patch_sha256'] == SOURCE_HASHES['patch-v29b-final'], 'ready source identity')
    need(ready['candidate_cargo_sha256'] == SOURCE_HASHES['cargo-final'], 'ready manifest identity')
    need(ready['candidate_doc_sha256'] == sha(e.read(FIXTURE + 'resident-nondebug-mark-cache.md.candidate')),
         'ready documentation identity')
    oracle = candidate.split('fn original_function_mark(', 1)[1].split('fn exact_fixture_marks(', 1)[0]
    need(not any(call in oracle for call in ('Mark::of_', 'Mark::new(', 'root_mark(db,', 'instance_mark(db,')),
         'oracle depends on candidate marks')
    need('source_location(db, *owner, *span).hash' in oracle and 'declaration_location(db, *func).hash' in oracle,
         'oracle lost real location dependencies')
    restore = e.obj('restoration.json')
    need(restore['source_ownership_released'] is True and restore['scoped_git_diff_head_clean'] is True,
         'restoration not completed')
    need(restore['restored_patch_sha256'] == SOURCE_HASHES['patch-original'] and
         restore['restored_cargo_sha256'] == SOURCE_HASHES['cargo-original'], 'restoration identity')
    need(restore['candidate_patch_sha256'] == SOURCE_HASHES['patch-v29b-final'] and
         restore['candidate_cargo_sha256'] == SOURCE_HASHES['cargo-final'], 'restored candidate identity')
    old_lock = e.text('provenance/original-Cargo.lock')
    new_lock = e.text('provenance/rejected-v29b-Cargo.lock')
    need(sha(old_lock.encode()) == SOURCE_HASHES['lock-original'], 'immutable original lock identity')
    need(replay_patch(old_lock, e.text('provenance/rejected-v29b-lock.patch'), 'toolchain/resident/Cargo.lock')
         == new_lock, 'lock patch reconstruction')
    old_packages, new_packages = [tomllib.loads(t)['package'] for t in [old_lock, new_lock]]
    need(len(old_packages) == len(new_packages) == 592, 'package inventory changed')
    changed = []
    for a, b in zip(old_packages, new_packages):
        if a != b:
            changed.append(a['name'])
            copy = dict(b)
            copy['dependencies'] = [n for n in b['dependencies'] if n not in ('hir', 'ide-db', 'test-fixture')]
            need(copy == a and set(b['dependencies']) - set(a['dependencies']) == {'hir', 'ide-db', 'test-fixture'},
                 'lock changed beyond three existing devdep names')
    need(changed == ['hir-ty-codegen'], 'unexpected lock packages changed')


def verify_native(e):
    for run, args in {
        '20261030-161353145-1565760': ['check', '--release', '--locked', '-p', 'hir-ty-codegen', '-p', 'ra-jit', '--tests', '-j', '6'],
        '20261030-161407940-1567589': ['build', '--release', '--locked', '-p', 'ra-jit', '--bin', 'jr-engine', '-j', '6'],
    }.items():
        prefix = f'native/build/raw-runs/{run}/'
        meta, summary = e.obj(prefix + 'meta.json'), e.obj(prefix + 'summary.json')
        need(meta['id'] == summary['id'] == run and summary['exit'] == 0 and meta['args'] == summary['args'] == args,
             'native raw build gate identity/command/status')
    for name in ('check', 'compile', 'exit', 'orchestration-exit'):
        suffix = '.exit' if name in ('check', 'compile') else ''
        need(e.text(f'native/build/{name}{suffix}').strip() == '0', 'native build gate failed')
    selected_text = e.text('native/build/source-before.sha256')
    need(sha(selected_text.encode()) == 'cbe921f268bdfa92d723d33126a79846aadeff47167e19a3deb43bdfb80bd363',
         'immutable selected source freeze identity')
    selected = hash_lines(selected_text)
    all_text = e.text('native/build/all-source-before.sha256')
    need(sha(all_text.encode()) == '5907c838a30f7b8d0801ffbe1bf1d2ed98bad74f64d8d54d5408787b343aa552',
         'immutable all-source freeze identity')
    all_sources = hash_lines(all_text)
    need(len(selected) == 20 and len(all_sources) == 3757, 'source freeze coverage changed')
    for path, digest in selected.items():
        need(sha(e.read('native/build/frozen-source/' + path)) == digest, 'frozen source/build mismatch')
    need(selected['crates/hir-ty-codegen/patch.rs'] == SOURCE_HASHES['patch-v29b-final'], 'wrong compiled candidate')
    for category in ('build', 'unit'):
        for name, inventory in [('source-after.log', selected), ('all-source-after.log', all_sources)]:
            lines = e.text(f'native/{category}/{name}').splitlines()
            need(lines == [f'{path}: OK' for path in inventory], 'source changed during actual native gate')
    for run, (name, count, filtered, names_hash) in NATIVE.items():
        base = f'native/unit/raw-runs/{run}/'
        meta, summary = e.obj(base + 'meta.json'), e.obj(base + 'summary.json')
        need(meta['id'] == summary['id'] == run and summary['exit'] == 0, 'native raw run identity/status')
        need(meta['args'] == summary['args'] and meta['args'][:3] == ['test', '--release', '--locked'], 'native command changed')
        need('nondebug_' in meta['args'] if name == 'mark' else '--' not in meta['args'], 'native test filter changed')
        wanted = ['test', '--release', '--locked', '-p', 'ra-jit' if name == 'full-native' else 'hir-ty-codegen']
        if name == 'full-native':
            wanted += ['--bin', 'jr-engine']
        wanted += ['-j', '6']
        if name == 'mark':
            wanted += ['--', 'nondebug_', '--nocapture']
        need(meta['args'] == wanted, 'actual native fixture command changed')
        need(summary['tests']['passed'] == count and summary['tests']['failed'] == 0 and
             summary['tests']['filtered_out'] == filtered, 'native raw test count changed')
        lines = [json.loads(line)['l'] for line in e.text(base + 'output.jsonl').splitlines()]
        names = sorted(match[1] for line in lines if (match := re.match(r'^test (.+) \.\.\. ok(?: |$)', line)))
        need(len(names) == len(set(names)) == count and sha('\n'.join(names).encode()) == names_hash,
             'native raw individual test names/counts changed')
        need(any(f'test result: ok. {count} passed; 0 failed; 0 ignored;' in line for line in lines), 'native raw result missing')
        need(e.text(f'native/unit/{name}.exit').strip() == '0', 'native wrapper exit changed')
        need(f'{count} passed, 0 failed' in e.text(f'native/unit/{name}.log'), 'native wrapper test receipt missing')
    need(e.text('native/unit/exit').strip() == '0', 'native unit orchestration failed')
    return [4, 50, 359]


def verify_session(e, name):
    prefix = f'sessions/{name}/'
    suite, meta, rows, summary = [e.obj(prefix + f) for f in ('suite.json', 'metadata.json', 'records.json', 'summary.json')]
    need(suite['schema'] == 'native_resident/v1' and suite['metadata'] == meta, 'suite metadata mismatch')
    for path, digest in suite['artifacts_sha256'].items():
        need('/' not in path and sha(e.read(prefix + path)) == digest, 'session artifact integrity')
    need(len(suite['artifacts_sha256']) == 48, 'original runtime artifact coverage changed')
    need(meta['status'] == 'complete' and all(meta[k] is True for k in ('source_restored', 'toolchain_links_valid', 'unchanged')),
         'suite incomplete or source/tool identity drift')
    for what, digest in [('source', '79c53ff6e87a22c84c9bc7bac6917c69b3f8d0fbc29b107f734cc00cc39a64df'),
                         ('toolchain', '8bdbccdd1ca3fd7f2935cfb5667047eb4c46824928823e96311d8eb5ec0a6371')]:
        need(meta[what + '_before'] == meta[what + '_after'] and canonical(meta[what + '_before']) == digest,
             f'immutable {what} identity drift')
    need(meta['tools_before'] == meta['tools_after'], 'tool identity changed during run')
    need(meta['tools_before']['engine']['sha256'] == ENGINE_HASHES[name] and
         meta['tools_before']['reference']['sha256'] == REFERENCE_HASH, 'immutable binary identity drift')
    build = 'native/build/' if name == 'rsa26' else 'provenance/baseline-v28/'
    need(e.text(build + 'engine-identity.sha256').split()[0] == ENGINE_HASHES[name], 'built/timed binary identity mismatch')
    need(f'build resident-v1-{BUILD_IDS[name]})' in e.text(build + 'engine-version.txt'), 'immutable engine build identity drift')
    need(sha(e.read(prefix + 'source.original')) == meta['source_before']['src/summary.rs'], 'runtime source snapshot mismatch')
    need(summary['excluded'] == [] and summary['source_built_parity'] is True and suite['noise'] == 'unknown',
         'sample exclusion/parity/noise policy changed')
    expected = {'tests': [['summary::tests::' + n, 'ok'] for n in TEST_NAMES], 'passed': 4, 'failed': 0, 'ignored': 0, 'filtered': 144}
    need(meta['expected_tests'] == expected and len(rows) == 22, 'runtime test/phase count changed')
    medians = {}
    phases = [('version', 0), ('prepare', 0), ('type-error', 0), ('type-recovery', 0), ('assert-failure', 0), ('assert-recovery', 0)] + [('edit', n) for n in range(5)]
    for stack in ('reference', 'engine'):
        entries = [row for row in rows if row['stack'] == stack]
        need(sorted((r['phase'], r['replicate']) for r in entries) == sorted(phases), 'runtime gate coverage changed')
        env = meta['environments'][stack]
        for key, value in {'BLAZE_REMOTE': 'off', 'RA_JIT_REMOTE': 'off', 'BLAZE_PROC_MACRO_SERVER': '0', 'RA_JIT_PROC_MACRO_SERVER': '0',
                           'BLAZE_BACKEND': 'cranelift', 'BLAZE_OPT': '0'}.items():
            need(env[key] == value, 'runtime launcher controls changed')
        for profile in ('DEV', 'TEST'):
            for key, value in [('OPT_LEVEL', '0'), ('DEBUG', '0'), ('INCREMENTAL', 'true')]:
                need(env[f'CARGO_PROFILE_{profile}_{key}'] == value, 'runtime profile changed')
        version = e.text(prefix + f'{stack}-version-0.stdout')
        need(meta['versions'][stack]['valid'] is True and meta['versions'][stack]['text'] == version + '\n' + e.text(prefix + f'{stack}-version-0.stderr'),
             'version receipt mismatch')
        need(f'resident-v1-{BUILD_IDS[name]}' in version if stack == 'engine' else version.startswith('blaze 0.20261027.0'), 'runtime build version drift')
        for row in entries:
            phase, replicate = row['phase'], row['replicate']
            need(row['error'] is None and row['timed_out'] is False, 'runtime launch error/timeout')
            arguments = ['--version'] if phase == 'version' else (['cargo'] if stack == 'reference' else []) + ['test', '--lib', '--', 'summary::tests::', '--test-threads=1', '--nocapture']
            need(row['command'] == [meta['tools_before'][stack]['path']] + arguments, 'actual runtime command changed')
            out, err = [e.text(prefix + f'{stack}-{phase}-{replicate}.{ext}') for ext in ('stdout', 'stderr')]
            if phase == 'type-error':
                need(row['status'] == 101 and row['gate_passed'] is True and row['parsed'] is None and
                     'error[E0308]' in err and 'JR_RUNTIME_TYPE_ERROR' in err, 'type-error gate lost')
            elif phase == 'assert-failure':
                need(row['status'] == 101 and row['gate_passed'] is True and row['parsed']['failed'] == 1 and
                     'JR_RUNTIME_FAILURE' in err and '3 passed; 1 failed' in out and 'shares_overlapping_time ... FAILED' in out,
                     'assertion negative gate lost')
            elif phase == 'version':
                need(row['status'] == 0, 'version execution failed')
            else:
                need(row['status'] == 0 and row['parsed'] == expected and 'test result: ok. 4 passed; 0 failed; 0 ignored;' in out and 'FAILED' not in out,
                     'runtime/recovery gate lost')
                need(all(f'test summary::tests::{n} ...' in out for n in TEST_NAMES), 'runtime individual test lost')
                if phase == 'edit':
                    need(f'JR_RUNTIME_EXECUTED_{replicate + 1}' in out, 'actual runtime edit marker lost')
        edits = sorted((r for r in entries if r['phase'] == 'edit'), key=lambda r: r['replicate'])
        samples = [r['elapsed_seconds'] for r in edits]
        need(all(math.isfinite(v) and v > 0 for v in samples), 'invalid elapsed duration')
        measure = suite['measurements'][stack]
        median = statistics.median(samples)
        need(measure['samples_seconds'] == samples and median == measure['median_seconds'] == summary['medians_seconds'][stack],
             'all-sample median changed')
        medians[stack] = median
    for phase, replicate in phases[1:]:
        hashes = [r['source_sha256'] for r in rows if r['phase'] == phase and r['replicate'] == replicate]
        need(len(hashes) == 2 and len(set(hashes)) == 1, 'engine/reference edited source mismatch')
    return medians


def validate(e):
    count = verify_inventory(e)
    decision = e.obj('decision.json')
    need(decision['decision'] == 'REJECTED' and decision['promotion'] is False and decision['speedup_claimed'] is False,
         'rejection/no-promotion decision changed')
    verify_source(e)
    native = verify_native(e)
    medians = {name: verify_session(e, name) for name in ('rsb20', 'rsa26')}
    a, b = medians.values()
    need(decision['matched_sessions'] == ['rsb20', 'rsa26'] and
         decision['observed_engine_medians_seconds'] == [a['engine'], b['engine']] and
         decision['observed_reference_medians_seconds'] == [a['reference'], b['reference']],
         'decision receipt detached from all-sample medians')
    raw = (b['engine'] / a['engine'] - 1) * 100
    normalized = ((b['engine'] / b['reference']) / (a['engine'] / a['reference']) - 1) * 100
    comparison = e.text('provenance/suite-v29b-compare.log')
    need(f'({raw:+.2f}%)' in comparison and f'{normalized:+.2f}%' in comparison and
         f'{a["engine"]:.6f}s -> {b["engine"]:.6f}s' in comparison, 'raw comparison disagrees with medians')
    need(raw > 0 and normalized > 0 and abs(raw - 6.62) < .005 and abs(normalized - 19.41) < .005,
         'matched observations no longer support recorded rejection')
    need('noise unknown' in comparison and 'No significance or speedup verdict' in comparison and 'Not a comprehensive workload' in comparison,
         'measurement caveats lost')
    return {'decision': 'REJECTED', 'promotion': False, 'speedup_claimed': False, 'immutable_files': count,
            'native_test_counts': native, 'sessions': medians, 'raw_percent': raw,
            'reference_normalized_percent': normalized, 'noise': 'unknown',
            'scope': 'scoped native diagnostic only, not comprehensive default-promotion evidence',
            'mechanism': 'four native direct-oracle/provider-counter fixtures passed as written',
            'restoration': 'two owned source paths exact HEAD; root separately restored lock'}


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('root', nargs='?', type=Path, default=Path(__file__).resolve().parent)
    args = parser.parse_args()
    try:
        report = validate(Evidence(args.root))
    except (InvalidEvidence, OSError, KeyError, ValueError, TypeError) as error:
        parser.exit(1, f'INVALID EVIDENCE: {error}\n')
    print(json.dumps(report, indent=2))


if __name__ == '__main__':
    main()
