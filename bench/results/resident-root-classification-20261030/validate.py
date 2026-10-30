#!/usr/bin/env python3
"""Portable offline evidence audit. No builds, network, historical paths, or git required."""
import argparse
import hashlib
import json
import math
from pathlib import Path
import re
import statistics

SESSIONS = ('rsb02', 'rsa07', 'rsa08', 'rsb03', 'rsa09')
SOURCE_HASHES = {
    9: 'e09c8ec5984a73cd32b11d12f4337b6d4d2ea872f536891b4e59ec9700fe77d4',
    10: 'fdddfb3d0bd68f5e304c2e427a48b918a706edb94e76d98b37b4cd88f86bb3b5',
    11: 'aaab4f1312d1d4f35523d8687f8ba1925b7b8f4d1442e267d27d44cc78d1b5fe',
    12: 'ee2ef625d670c5a646acf2f180495a4294123cfb92f0fb83b967418167e2e03c',
}
PRIMITIVE_HASH = '304e148fdf9ef75d77a11f5f4802d81698c1104b88a1caeb0cb5fe2b2fbeb45e'
TEST_NAMES = ['parses_libtest_results', 'shares_overlapping_time',
              'splits_passes_without_double_counting', 'test_failures_are_not_compile_errors']


class InvalidEvidence(ValueError):
    pass


def need(condition, message):
    if not condition:
        raise InvalidEvidence(message)


def sha(data):
    return hashlib.sha256(data).hexdigest()


class Evidence:
    def __init__(self, root, overrides=None):
        self.root = Path(root)
        self.overrides = overrides if overrides is not None else {}

    def read(self, relative):
        path = Path(relative)
        need(not path.is_absolute() and '..' not in path.parts, 'unsafe artifact path')
        if relative in self.overrides:
            return self.overrides[relative]
        try:
            return (self.root / path).read_bytes()
        except OSError as error:
            raise InvalidEvidence(f'missing artifact {relative}') from error

    def text(self, relative):
        return self.read(relative).decode('utf-8')

    def obj(self, relative):
        return json.loads(self.text(relative))


def verify_integrity(e):
    inventory = e.obj('manifest.json')['sha256']
    need(len(inventory) >= 400, 'incomplete artifact inventory')
    excluded = {'manifest.json', 'collect.py', 'validate.py', 'test_validate.py', 'README.md', 'audit.json', 'offline-tests.log'}
    actual = {str(path.relative_to(e.root)) for path in e.root.rglob('*') if path.is_file() and path.name not in excluded and '__pycache__' not in path.parts}
    need(set(inventory) == actual, 'artifact inventory omits or adds a captured file')
    for path, digest in inventory.items():
        need(sha(e.read(path)) == digest, f'checksum mismatch: {path}')
    return len(inventory)


def source_receipt(e, version):
    text = e.text(f'native/candidate-build-v{version}/source-before.sha256')
    return dict((path, digest) for digest, path in
                (line.split(None, 1) for line in text.splitlines()))


def verify_patch(e):
    """Replay only archived unified hunks in memory against archived baseline bytes."""
    mapping = {'toolchain/resident/crates/ra-jit/src/main.rs': 'main.rs',
               'toolchain/resident/crates/hir-ty-codegen/mod.rs': 'hir-ty-codegen-mod.rs'}
    patch = e.text('candidate-v12/candidate.patch').splitlines(keepends=True)
    seen = set()
    i = 0
    while i < len(patch):
        if not patch[i].startswith('--- a/'):
            i += 1
            continue
        old = patch[i][6:].strip()
        i += 1
        need(i < len(patch) and patch[i] == f'+++ b/{old}\n', 'patch renamed source')
        need(old in mapping and old not in seen, 'unexpected patch source')
        seen.add(old)
        name = mapping[old]
        original = e.text(f'baseline-head/{name}').splitlines(keepends=True)
        result, cursor = [], 0
        i += 1
        while i < len(patch) and not patch[i].startswith('diff --git '):
            match = re.match(r'@@ -(\d+)(?:,(\d+))? \+(\d+)(?:,(\d+))? @@', patch[i])
            need(match is not None, 'invalid patch hunk')
            start = int(match[1]) - 1
            need(start >= cursor, 'overlapping patch hunks')
            result.extend(original[cursor:start])
            cursor = start
            i += 1
            old_count = new_count = 0
            while i < len(patch) and patch[i][:1] in (' ', '+', '-'):
                line = patch[i]
                if line[0] in (' ', '-'):
                    need(cursor < len(original) and original[cursor] == line[1:], 'patch baseline context mismatch')
                    cursor += 1
                    old_count += 1
                if line[0] in (' ', '+'):
                    result.append(line[1:])
                    new_count += 1
                i += 1
            need(old_count == int(match[2] or 1) and new_count == int(match[4] or 1), 'patch hunk range mismatch')
        result.extend(original[cursor:])
        need(''.join(result) == e.text(f'candidate-v12/{name}'), 'patch does not reconstruct frozen candidate')
    need(seen == set(mapping), 'patch missing owned source')


def verify_native(e):
    for version in [9, 10, 11, 12]:
        ready = e.text(f'provenance/root-classification-source-ready-v{version}.txt')
        need(SOURCE_HASHES[version] in ready, f'v{version} original source receipt changed')
        need(source_receipt(e, version)['crates/ra-jit/src/main.rs'] == SOURCE_HASHES[version], f'v{version} build/source mismatch')
    need(int(e.text('native/candidate-build-v9/check-exit')) != 0, 'v9 compile failure lost')
    compile_error = e.text('native/candidate-build-v9/check.log')
    need('E0308' in compile_error and 'E0277' in compile_error, 'v9 original return-mode errors lost')
    failure = e.text('native/candidate-unit-v10/resident_root_classification.log')
    need(int(e.text('native/candidate-unit-v10/resident_root_classification.exit')) != 0, 'v10 mechanism failure lost')
    need('left: 7' in failure and 'right: 5' in failure and 'FAILED' in failure, 'v10 actual failed counts lost')
    need(int(e.text('provenance/suite-after-v10.exit')) == 2, 'failed native mechanism did not block v10 timing gate')
    for version in [10, 11, 12]:
        for name in ['check-exit', 'compile-exit', 'exit']:
            need(int(e.text(f'native/candidate-build-v{version}/{name}')) == 0, 'native successful build gate changed')
        prefix = f'native/candidate-unit-v{version}/'
        if version == 10:
            prefix += 'frozen-oracle/'
            need(bool(re.match(r'[0-9a-f]{64}', e.text(prefix + 'test-binary.sha256'))), 'v10 frozen test identity lost')
        for name, passed in [('resident_graph_reuse', 1), ('the_roots_of_a_listed_crate', 1), ('telemetry', 3)]:
            need(int(e.text(prefix + name + '.exit')) == 0, 'original classifier/telemetry failed')
            text = e.text(prefix + name + '.log')
            need(re.search(rf'(?:{passed} passed, 0 failed|{passed} passed; 0 failed)', text), 'native oracle pass receipt missing')
        if version >= 11:
            text = e.text(prefix + 'resident_root_classification.log')
            values = re.findall(r'classification revision (\d+): \[export, root, main\] = \[(\d+), (\d+), (\d+)\]', text)
            expected = [('0', '2', '2', '1'), ('1', '2', '2', '1'), ('2', '2', '4' if version == 11 else '2', '1')]
            need(values == expected, f'v{version} exact provider counts changed')
            need(int(e.text(prefix + 'resident_root_classification.exit')) == 0, 'provider-count native test did not pass')
            for source in source_receipt(e, version):
                need(f'{source}: OK' in e.text(prefix + 'source-after.log'), 'post-test source identity changed')
    need(sha(e.read('candidate-v12/main.rs')) == SOURCE_HASHES[12], 'final full source hash mismatch')
    need(sha(e.read('candidate-v12/hir-ty-codegen-mod.rs')) == PRIMITIVE_HASH, 'final primitive hash mismatch')
    need(source_receipt(e, 12)['crates/hir-ty-codegen/mod.rs'] == PRIMITIVE_HASH, 'primitive build source mismatch')
    ready = e.text('provenance/root-classification-source-ready-v12.txt')
    for name in ['main.rs', 'hir-ty-codegen-mod.rs', 'resident-root-classification.md']:
        need(sha(e.read('candidate-v12/' + name)) in ready, 'frozen full-source/ready hash mismatch')
    candidate = e.text('candidate-v12/main.rs')
    oracle = candidate.split('fn assert_original_classification(', 1)[1].split('/// Compare with the uncached traversal', 1)[0]
    need(oracle.count('AttrFlags::has_word(db, it.into(), "rustc_comptime")') == 2, 'oracle no longer uses original raw primitive')
    need('hir_ty_codegen::compile_time_only' not in oracle, 'oracle depends on candidate primitive')
    primitive = e.text('candidate-v12/hir-ty-codegen-mod.rs')
    baseline = e.text('baseline-head/hir-ty-codegen-mod.rs')
    needle = '#[salsa::tracked(returns(copy))]\npub fn compile_time_only'
    need(needle in primitive and primitive.replace(needle, 'pub fn compile_time_only', 1) == baseline, 'primitive body changed beyond tracked annotation')
    verify_patch(e)
    full = 'native/candidate-unit-v12/'
    need(int(e.text(full + 'full-native.exit')) == 0, 'full native suite failed')
    need('347 passed, 0 failed, 0 ignored, 0 filtered out' in e.text(full + 'full-native.log'), 'full347 receipt missing')
    run = full + 'justrust-home/runs/20261030-091611542-269328/'
    lines = [json.loads(line)['l'] for line in e.text(run + 'output.jsonl').splitlines()]
    tests = [line for line in lines if re.match(r'^test .+ \.\.\. ok(?: |$)', line)]
    need(len(tests) == 347 and len(set(line.split(' ... ')[0] for line in tests)) == 347, 'raw347 individual test results incomplete')
    need(any('test result: ok. 347 passed; 0 failed; 0 ignored' in line for line in lines), 'raw full-native result missing')
    for source in source_receipt(e, 12):
        need(f'{source}: OK' in e.text(full + 'full-native-source-after.log'), 'post-full-suite source identity changed')


def verify_session(e, name):
    base = f'sessions/{name}/'
    suite, metadata, records, summary = [e.obj(base + part) for part in ['suite.json', 'metadata.json', 'records.json', 'summary.json']]
    need(suite['schema'] == 'native_resident/v1', 'unexpected suite schema')
    need(suite['metadata'] == metadata, 'suite metadata detached from raw receipt')
    for path, digest in suite['artifacts_sha256'].items():
        need('/' not in path and sha(e.read(base + path)) == digest, 'original session artifact hash mismatch')
    for flag in ['source_restored', 'toolchain_links_valid', 'unchanged']:
        need(metadata[flag] is True, f'{name} identity flag {flag} false')
    need(metadata['status'] == 'complete', 'incomplete runtime suite')
    for what in ['source', 'tools', 'toolchain']:
        need(metadata[what + '_before'] == metadata[what + '_after'], 'source/tool identity changed during session')
    need(sha(e.read(base + 'source.original')) == metadata['source_before']['src/summary.rs'], 'source original hash mismatch')
    need(metadata['expected_tests']['passed'] == 4 and metadata['expected_tests']['failed'] == 0, 'expected test contract changed')
    need([test[0] for test in metadata['expected_tests']['tests']] == ['summary::tests::' + test for test in TEST_NAMES], 'original test names changed')
    need(summary['excluded'] == [] and summary['source_built_parity'] is True, 'excluded sample or parity failure')
    need(suite['noise'] == 'unknown', 'noise uncertainty was erased')
    need(len(records) == 22, 'runtime records missing or added')
    medians = {}
    for stack in ['reference', 'engine']:
        version = e.text(base + f'{stack}-version-0.stdout')
        need(metadata['versions'][stack]['valid'] is True and version + '\n' + e.text(base + f'{stack}-version-0.stderr') == metadata['versions'][stack]['text'], 'version receipt mismatch')
        need('Blaze-derived origin' in version if stack == 'engine' else version.startswith('blaze 0.20261027.0'), 'wrong engine family')
        env = metadata['environments'][stack]
        for key, value in {'BLAZE_PROC_MACRO_SERVER': '0', 'RA_JIT_PROC_MACRO_SERVER': '0', 'BLAZE_REMOTE': 'off', 'RA_JIT_REMOTE': 'off'}.items():
            need(env[key] == value, 'launcher compatibility control changed')
        rows = [row for row in records if row['stack'] == stack]
        expected_phases = [('version', 0), ('prepare', 0), ('type-error', 0), ('type-recovery', 0), ('assert-failure', 0), ('assert-recovery', 0)] + [('edit', n) for n in range(5)]
        need(sorted((row['phase'], row['replicate']) for row in rows) == sorted(expected_phases), 'runtime phase/replicate coverage incomplete')
        for row in rows:
            phase, replicate = row['phase'], row['replicate']
            need(not row['timed_out'] and row['error'] is None, 'runtime timeout or launch error')
            need(row['command'][0] == metadata['tools_before'][stack]['path'], 'runtime command binary differs from identity')
            expected_args = ['--version'] if phase == 'version' else ((['cargo'] if stack == 'reference' else []) + ['test', '--lib', '--', 'summary::tests::', '--test-threads=1', '--nocapture'])
            need(row['command'][1:] == expected_args, 'actual direct native runtime contract changed')
            out = e.text(base + f'{stack}-{phase}-{replicate}.stdout')
            err = e.text(base + f'{stack}-{phase}-{replicate}.stderr')
            if phase == 'type-error':
                need(row['status'] == 101 and row.get('gate_passed') is True and row['parsed'] is None, 'type-error negative gate invalid')
                need('error[E0308]' in err and 'JR_RUNTIME_TYPE_ERROR' in err, 'real type diagnostic missing')
            elif phase == 'assert-failure':
                need(row['status'] == 101 and row.get('gate_passed') is True and row['parsed']['failed'] == 1, 'assert negative gate invalid')
                need('JR_RUNTIME_FAILURE' in err and '3 passed; 1 failed' in out and 'shares_overlapping_time ... FAILED' in out, 'real runtime assertion failure missing')
            elif phase != 'version':
                need(row['status'] == 0 and row['parsed'] == metadata['expected_tests'], 'runtime/recovery actual test receipt differs')
                need('test result: ok. 4 passed; 0 failed; 0 ignored' in out and 'FAILED' not in out, 'raw runtime/recovery result missing')
                for test in TEST_NAMES:
                    need(f'test summary::tests::{test} ...' in out, 'raw runtime test name missing')
                if phase == 'edit':
                    need(f'JR_RUNTIME_EXECUTED_{replicate + 1}' in out, 'runtime edit marker missing')
            else:
                need(row['status'] == 0, 'version command failed')
        edits = sorted((row for row in rows if row['phase'] == 'edit'), key=lambda row: row['replicate'])
        samples = [row['elapsed_seconds'] for row in edits]
        need(all(math.isfinite(value) and value > 0 for value in samples), 'invalid sample duration')
        measurement = suite['measurements'][stack]
        median = statistics.median(samples)
        need(measurement['samples_seconds'] == samples, 'timing samples detached from original records')
        need(median == measurement['median_seconds'] == summary['medians_seconds'][stack], 'median not computed from all samples')
        medians[stack] = median
    for phase, replicate in [('prepare', 0), ('type-error', 0), ('type-recovery', 0), ('assert-failure', 0), ('assert-recovery', 0)] + [('edit', n) for n in range(5)]:
        matched = [row['source_sha256'] for row in records if row['phase'] == phase and row['replicate'] == replicate]
        need(len(set(matched)) == 1, 'engine/reference source edit mismatch')
    return {'medians': medians, 'metadata': metadata}


def validate(e):
    count = verify_integrity(e)
    provenance = e.obj('provenance.json')
    need(provenance['decision'] == 'REJECTED' and provenance['promotion'] is False, 'negative decision changed')
    need(provenance['source_reconstruction_claim'] is False, 'unsupported source reconstruction claim')
    need(provenance['older_full_source'] == {f'v{v}': 'unknown/not captured' for v in [9, 10, 11]}, 'older full-source availability retconned')
    need(provenance['sessions'] == list(SESSIONS) and provenance['final_pair'] == ['rsb03', 'rsa09'], 'comparison provenance changed')
    restoration = e.obj('restoration.json')
    need(restoration['head_at_capture'] == provenance['capture_head'] and restoration['source_ownership_released'] is True, 'restoration provenance mismatch')
    need(sha(e.read('candidate-v12/candidate.patch')) == restoration['candidate_patch_sha256'], 'restored candidate patch identity mismatch')
    for source, name in [('toolchain/resident/crates/ra-jit/src/main.rs', 'main.rs'), ('toolchain/resident/crates/hir-ty-codegen/mod.rs', 'hir-ty-codegen-mod.rs')]:
        need(sha(e.read('baseline-head/' + name)) == restoration['restored_sha256'][source], 'restored source not byte-exact archived HEAD')
    need(sha(e.read('baseline-head/main.rs')) == source_receipt(e, 4)['crates/ra-jit/src/main.rs'], 'baseline archived source differs from actual v4 build')
    verify_native(e)
    results = {name: verify_session(e, name) for name in SESSIONS}
    first = results['rsb02']['metadata']
    versions = {'rsb02': 4, 'rsb03': 4, 'rsa07': 11, 'rsa08': 12, 'rsa09': 12}
    for name, result in results.items():
        meta = result['metadata']
        need(meta['source_before'] == first['source_before'] and meta['toolchain_before'] == first['toolchain_before'], 'cross-session source/toolchain mismatch')
        need(meta['tools_before']['reference'] == first['tools_before']['reference'], 'reference binary mismatch across sessions')
        version = versions[name]
        build = f'native/candidate-build-v{version}/'
        need(meta['tools_before']['engine']['sha256'] == e.text(build + 'engine-identity.sha256').split()[0], 'timed engine identity differs from built engine')
        need(meta['versions']['engine']['text'].strip() == e.text(build + 'engine-version.txt').strip(), 'timed build version mismatch')
    comparisons = {}
    for before, after, log in [('rsb02', 'rsa07', 'suite-v4-v11-compare.log'), ('rsb02', 'rsa08', 'suite-v4-v12-compare.log'), ('rsb03', 'rsa09', 'suite-v12-repeat-compare.log')]:
        a, b = results[before]['medians'], results[after]['medians']
        raw = (b['engine'] / a['engine'] - 1) * 100
        normalized = ((b['engine'] / b['reference']) / (a['engine'] / a['reference']) - 1) * 100
        original = e.text('provenance/' + log)
        need(f'({raw:+.2f}%)' in original and f'{normalized:+.2f}%' in original, 'original suite comparison disagrees with raw medians')
        comparisons[before + '/' + after] = {'raw_percent': raw, 'reference_normalized_percent': normalized}
    final = comparisons['rsb03/rsa09']
    need(final['raw_percent'] > 10 and final['reference_normalized_percent'] > 10, 'final replication no longer supports rejection for regression')
    return {'decision': 'REJECTED', 'promotion': False, 'immutable_files': count,
            'native_tests_passed': 347, 'v11_counts': [[2, 2, 1], [2, 2, 1], [2, 4, 1]],
            'v12_counts': [[2, 2, 1]] * 3,
            'sessions': {name: result['medians'] for name, result in results.items()},
            'comparisons': comparisons, 'claim': 'mechanism proven, total optimization rejected'}


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('root', nargs='?', type=Path, default=Path(__file__).resolve().parent)
    args = parser.parse_args()
    try:
        report = validate(Evidence(args.root))
    except (InvalidEvidence, KeyError, ValueError, TypeError) as error:
        parser.exit(1, f'INVALID EVIDENCE: {error}\n')
    print(json.dumps(report, indent=2))


if __name__ == '__main__':
    main()
