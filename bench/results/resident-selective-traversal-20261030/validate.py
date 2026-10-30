#!/usr/bin/env python3
"""Portable stdlib-only rejection audit. Reads only bundle-local text artifacts."""
import argparse
import hashlib
import itertools
import json
import math
import re
import statistics
from pathlib import Path

BASE = 'toolchain/resident/'
LIB = 'vendor/rust-analyzer/crates/ide-diagnostics/src/lib.rs'
TESTS = 'vendor/rust-analyzer/crates/ide-diagnostics/src/tests.rs'
DOC = 'docs/resident-selective-traversal.md'
SOURCE_SHA = {
    BASE + LIB: 'f81d1eadea08ff3dcab0624254a86d75af1a67ea8bff0b067ca105cae42619cb',
    BASE + TESTS: 'ee88c803abfa78f9f894666138fe1f9b0529d001acb2b89f4e5b54d22d1e6f23',
    DOC: '788eaa2b329a6a60ff5a091d43a65af4c68e6ed6f583791ef9d629e83314016a',
}
OLD_SHA = {
    BASE + LIB: 'ada7271fc4dac1356e3aeb510778dd86c891ab0834437a70fcf06ad79e8fe72f',
    BASE + TESTS: '943ca135d394f70b03f29ad4ac290068a59580a65edccd947e489797070a0c24',
}
TOOLS = {'old': '31e916f280feb2e79bea42e1ef3a03b68ca16df09e361f437459b10be8b03219',
         'new': '64b7ae843509c96c4d1c17b85db303d91140e983865de4d63f5327308a35dbf1',
         'blaze': 'fa622aeca09858c050264f701fbbebd1a14aa07d2b02c1eb4de511b684ba0263'}
NAMES = {'summary::tests::' + name for name in (
    'shares_overlapping_time', 'splits_passes_without_double_counting',
    'parses_libtest_results', 'test_failures_are_not_compile_errors')}
API_NAMES = {'tests::resident_selective_traversal_' + name for name in (
    'full_api_preserves_syntax_warnings', 'all_bodies_matches_full_semantic_errors',
    'predicate_keeps_only_requested_body_errors')}
RESULT = re.compile(r'test result: (ok|FAILED)\. (\d+) passed; (\d+) failed; (\d+) ignored;')


def require(ok, message):
    if not ok:
        raise ValueError(message)


def local(root, name):
    path = root / name
    require(not Path(name).is_absolute() and '..' not in Path(name).parts, 'nonlocal artifact')
    require(not path.is_symlink() and path.resolve().is_relative_to(root.resolve()), 'escaping artifact')
    require(path.is_file(), 'missing artifact: ' + name)
    return path


def text(root, name):
    return local(root, name).read_text()


def load(root, name):
    return json.loads(text(root, name))


def sha(data):
    return hashlib.sha256(data).hexdigest()


def digest(root, name):
    return sha(local(root, name).read_bytes())


def same_number(actual, expected, message):
    require(isinstance(actual, (float, int)) and math.isfinite(actual)
            and math.isclose(actual, expected, rel_tol=1e-12, abs_tol=1e-12), message)


def identity(state):
    require(state['source_before'] == state['source_after'], 'source boundary mismatch')
    require(state['toolchain_before'] == state['toolchain_after'], 'toolchain boundary mismatch')


def patch_file(original, patch):
    """Replay exact unified hunks in memory, checking every original context byte."""
    old, result, cursor = original.splitlines(keepends=True), [], 0
    lines = patch.splitlines(keepends=True)
    i = 0
    while i < len(lines):
        match = re.match(r'@@ -(\d+)(?:,\d+)? \+(\d+)(?:,\d+)? @@', lines[i])
        if not match:
            i += 1
            continue
        start = max(int(match[1]) - 1, 0)
        require(start >= cursor, 'patch hunk overlap')
        result.extend(old[cursor:start])
        cursor, i = start, i + 1
        while i < len(lines) and not lines[i].startswith(('@@', 'diff --git')):
            line = lines[i]
            if line.startswith((' ', '-')):
                require(cursor < len(old) and old[cursor] == line[1:], 'patch original context mismatch')
                cursor += 1
            if line.startswith((' ', '+')):
                result.append(line[1:])
            i += 1
    result.extend(old[cursor:])
    return ''.join(result)


def sources(root):
    for name, expected in SOURCE_SHA.items():
        require(digest(root, 'source-ready/candidate/' + name) == expected, 'candidate source identity')
    for name, expected in OLD_SHA.items():
        require(digest(root, 'source-ready/original/' + name) == expected, 'original source identity')
    patch = text(root, 'source-ready/source.patch')
    chunks = re.split(r'(?=^diff --git )', patch, flags=re.M)
    paths = []
    for chunk in chunks:
        if not chunk:
            continue
        header = chunk.splitlines()[0]
        m = re.fullmatch(r'diff --git a/(\S+) b/(\S+)', header)
        require(m is not None and m[1] == m[2], 'patch path contract')
        name = m[1]
        paths.append(name)
        require(name in SOURCE_SHA, 'patch touches unowned source')
        original = '' if name == DOC else text(root, 'source-ready/original/' + name)
        require(patch_file(original, chunk) == text(root, 'source-ready/candidate/' + name),
                'patch reconstruction mismatch: ' + name)
    require(set(paths) == set(SOURCE_SHA) and len(paths) == 3, 'patch scope mismatch')
    old = text(root, 'source-ready/original/' + BASE + LIB)
    candidate = text(root, 'source-ready/candidate/' + BASE + LIB)
    start = old.index('    for node in parse.syntax().descendants().filter(|_| bodies.is_none()) {')
    end = old.index('\n\n    let module =', start)
    block = old[start:end]
    expected = '    if bodies.is_none() {\n' + '\n'.join(
        '    ' + line for line in block.replace('.filter(|_| bodies.is_none())', '').splitlines()
    ) + '\n    }'
    require(candidate == old[:start] + expected + old[end:], 'nonminimal production change')
    manifests = {}
    for version, count, tool in [('v23', 12, 'old'), ('v24', 14, 'new')]:
        manifest = text(root, f'build/{version}/source-before.sha256').splitlines()
        require(len(manifest) == count, 'build source manifest count')
        manifests[version] = {}
        for line in manifest:
            expected_sha, name = line.split(maxsplit=1)
            manifests[version][name] = expected_sha
            require(digest(root, f'build/{version}/frozen-source/{name}') == expected_sha,
                    'build frozen source hash mismatch')
        after = text(root, f'build/{version}/source-after.log').splitlines()
        require(after == [name + ': OK' for name in manifests[version]], 'build source boundary')
        require(text(root, f'build/{version}/engine-identity.sha256').split()[0] == TOOLS[tool],
                'build binary identity')
        for status in ['check-exit', 'compile-exit', 'exit']:
            require(text(root, f'build/{version}/{status}').strip() == '0', 'check/build failed')
    helper = 'crates/ra-jit/src/diagnostic_hash.rs'
    require(digest(root, 'build/v23/frozen-source/' + helper)
            == digest(root, 'build/v24/frozen-source/' + helper), 'accepted helper changed')
    require(set(manifests['v24']) - set(manifests['v23']) == {LIB, TESTS}, 'selected source additions')
    require(all(manifests['v24'][name] == value for name, value in manifests['v23'].items()),
            'shared selected source changed')
    require(all(manifests['v24'][name] == SOURCE_SHA[BASE + name] for name in [LIB, TESTS]),
            'build/API candidate source mismatch')


def snapshot(root):
    before = load(root, 'native/v24-snapshot/source-copy-before.json')
    after = load(root, 'native/v24-snapshot/source-copy-after.json')
    require(before['status'] == 'identical' and len(before['files']) == 2444, 'copy admission')
    require(after['status'] == 'passed' and after['original_unchanged'], 'live source copy boundary')
    require(set(before['files']) == set(after['files_after']), 'copy file set changed')
    changed = [name for name in before['files'] if before['files'][name] != after['files_after'][name]]
    require(changed == ['Cargo.lock'] and after['changed_snapshot_files'] == changed, 'copy source delta')
    for phase, table in [('before', before['files']), ('after', after['files_after'])]:
        require(digest(root, 'snapshot/Cargo.lock.' + phase) == table['Cargo.lock']
                == after['private_lock_' + phase], 'private lock identity')
        for name in [LIB, TESTS]:
            relative = name.removeprefix('vendor/rust-analyzer/')
            require(table[relative] == SOURCE_SHA[BASE + name], 'actual API source identity')
        require(digest(root, 'snapshot/Cargo.toml') == table['Cargo.toml'], 'workspace manifest identity')
    for name in ['source-before.log', 'source-after.log', 'full-native-source-after.log']:
        lines = text(root, 'native/v24-snapshot/' + name).splitlines()
        manifest = text(root, 'build/v24/source-before.sha256').splitlines()
        require(lines == [line.split(maxsplit=1)[1] + ': OK' for line in manifest], 'native source boundary')


def journal(root, version, run, count=None, exit_code=0):
    prefix = f'journals/{version}/{run}/'
    meta, summary = load(root, prefix + 'meta.json'), load(root, prefix + 'summary.json')
    require(summary['id'] == run and meta['id'] == run and summary['exit'] == exit_code,
            'journal identity/exit mismatch')
    raw = '\n'.join(json.loads(line)['l'] for line in text(root, prefix + 'output.jsonl').splitlines())
    if count is not None:
        require(RESULT.findall(raw) == [('ok', str(count), '0', '0')], 'native raw count mismatch')
        names = re.findall(r'^test (\S+) \.\.\. ok(?:\s|$)', raw, re.M)
        require(len(names) == count and len(set(names)) == count, 'native raw names mismatch')
        return raw, set(names)
    return raw, set()


def native(root):
    for run, subcommand in [('20261030-125604759-3143422', 'check'),
                            ('20261030-125614991-3144269', 'build')]:
        journal(root, 'build-v24', run)
        meta = load(root, f'journals/build-v24/{run}/meta.json')
        require(meta['subcommand'] == subcommand and '--locked' in meta['args'], 'native build invocation')
    admission = load(root, 'native/v24-snapshot/positive-test-admission.json')
    require(admission['verdict'] == 'passed', 'native admission verdict')
    expected = {'resident_selective_traversal': 3, 'diagnostic_hash::tests': 2,
                'diagnostics_timing_tests': 3, 'resident_graph_reuse': 1,
                'the_roots_of_a_listed_crate': 1, 'telemetry': 3}
    require({r['filter']: r['passed'] for r in admission['native_test_rows']} == expected
            and len(admission['native_test_rows']) == len(expected), 'native admission filters')
    for row in admission['native_test_rows']:
        raw, names = journal(root, 'v24-snapshot', row['run_id'], row['passed'])
        require(text(root, 'native/v24-snapshot/' + row['filter'] + '.exit').strip() == '0', 'native exit')
        if row['filter'] == 'resident_selective_traversal':
            require(names == API_NAMES, 'actual native API names')
            args = load(root, f"journals/v24-snapshot/{row['run_id']}/meta.json")['args']
            require('--manifest-path' in args and '--offline' in args and 'ide-diagnostics' in args,
                    'actual snapshot invocation contract')
    journal(root, 'v24-snapshot', '20261030-130705291-3342136', 351)
    require(text(root, 'native/v24-snapshot/full-native.exit').strip() == '0', 'full native failed')
    for version, run, marker in [
        ('v24', '20261030-125716550-3148215', 'not a member of the workspace'),
        ('v24-nested', '20261030-125849389-3153839', '--locked was passed'),
    ]:
        raw, _ = journal(root, version, run, exit_code=101)
        require(marker in raw and not RESULT.findall(raw), 'original invocation failure was lost')
        require(text(root, f'native/{version}/resident_selective_traversal.exit').strip() == '101',
                'original failure status changed')
    require("unexpected argument '--lockfile-path'" in text(root, 'native/v24-private-lock/metadata.stderr'),
            'unsupported alternate lock path failure missing')
    require(text(root, 'native/v24-private-lock/metadata.exit').strip() == '1', 'metadata failure exit')
    require(text(root, 'provenance/final-v24.exit').strip() == '101', 'initial orchestration failure')
    require(text(root, 'provenance/final-v24-snapshot.exit').strip() == '0', 'corrected orchestration exit')


def runtime(raw, failing=False):
    expected = [('FAILED', '3', '1', '0')] if failing else [('ok', '4', '0', '0')]
    require(RESULT.findall(raw) == expected, 'runtime raw test counts')
    require('; 0 measured; 144 filtered out;' in raw, 'runtime filtered test contract')
    names = re.findall(r'^test (summary::tests::\w+) \.\.\. (?:JR_RUNTIME_EXECUTED_\d+\n)?(ok|FAILED)', raw, re.M)
    require(len(names) == 4 and {n for n, _ in names} == NAMES, 'runtime raw test names')
    require(sum(v == 'FAILED' for _, v in names) == int(failing), 'runtime raw verdicts')


def edit(original, phase, rep):
    anchor = b'fn shares_overlapping_time() {'
    require(original.count(anchor) == 1, 'source edit anchor')
    n = rep + 1
    body = (f'let observed = std::hint::black_box({n}u64); assert_eq!(observed, {n}u64); '
            f'println!("JR_RUNTIME_EXECUTED_{n}");').encode()
    if phase == 'type-error':
        body = b'let _: u64 = "JR_RUNTIME_TYPE_ERROR";'
    elif phase == 'assert-failure':
        body = b'assert_eq!(std::hint::black_box(1u64), 2u64, "JR_RUNTIME_FAILURE");'
    if phase not in ['edit', 'type-error', 'assert-failure']:
        return original
    return original.replace(anchor, anchor + b'\n' + body, 1)


def row_check(root, prefix, row, original, offset):
    phase, rep = row['phase'], row['replicate']
    require(not row['error'] and not row['timed_out'], 'runtime process error')
    require(row['elapsed_seconds'] > 0 and math.isfinite(row['elapsed_seconds']), 'invalid row time')
    streams = []
    for suffix in ['stdout', 'stderr']:
        name = prefix + '.' + suffix
        streams.append(text(root, name))
        if suffix + '_sha256' in row:
            require(digest(root, name) == row[suffix + '_sha256'], 'raw stream hash')
    raw = '\n'.join(streams)
    if phase in ['type-error', 'assert-failure']:
        require(row['status'] == 101, 'negative control status')
        if phase == 'type-error':
            require('E0308' in raw and 'JR_RUNTIME_TYPE_ERROR' in raw and not RESULT.findall(raw),
                    'negative type-error raw gate')
        else:
            runtime(raw, failing=True)
            require('JR_RUNTIME_FAILURE' in raw, 'negative assertion raw gate')
    else:
        require(row['status'] == 0, 'positive runtime status')
        if phase != 'version':
            runtime(raw)
        if phase == 'edit':
            marker = f'JR_RUNTIME_EXECUTED_{rep + offset + 1}'
            require(re.findall(r'JR_RUNTIME_EXECUTED_\d+', raw) == [marker], 'exact raw runtime marker')
            require(row['accepted'], 'edit admission')
    require(row.get('edit_sha256', row.get('source_sha256')) == sha(edit(original, phase, rep + offset)),
            'edit source reconstruction')


def fixed(root, folder, tool):
    path = 'fixed-five/' + folder
    rows, meta = load(root, path + '/records.json'), load(root, path + '/metadata.json')
    suite, summary = load(root, path + '/suite.json'), load(root, path + '/summary.json')
    require(meta['status'] == 'complete' and meta['unchanged'] and meta['source_restored'], 'fixed suite admission')
    identity(meta)
    require(meta['tools_before'] == meta['tools_after'], 'fixed tools boundary')
    require(meta['tools_before']['engine']['sha256'] == TOOLS[tool]
            and meta['tools_before']['reference']['sha256'] == TOOLS['blaze'], 'fixed tools identity')
    require(suite['metadata'] == meta, 'suite/metadata disagreement')
    for name, expected in suite['artifacts_sha256'].items():
        require(digest(root, path + '/' + name) == expected, 'suite artifact hash')
    expected_keys = {(stack, phase, 0) for stack in ['engine', 'reference']
                     for phase in ['version', 'prepare', 'type-error', 'type-recovery', 'assert-failure', 'assert-recovery']}
    expected_keys |= {(stack, 'edit', rep) for stack in ['engine', 'reference'] for rep in range(5)}
    require(len(rows) == 22 and {(r['stack'], r['phase'], r['replicate']) for r in rows} == expected_keys,
            'fixed complete rows')
    original = local(root, path + '/source.original').read_bytes()
    require(meta['source_before']['src/summary.rs'] == sha(original), 'fixed original source identity')
    for row in rows:
        expected_command = [meta['tools_before'][row['stack']]['path']]
        if row['phase'] == 'version':
            expected_command += ['--version']
        else:
            if row['stack'] == 'reference':
                expected_command += ['cargo']
            expected_command += ['test', '--lib', '--', 'summary::tests::', '--test-threads=1', '--nocapture']
        require(row['command'] == expected_command, 'fixed command mismatch')
        row_check(root, f"{path}/{row['stack']}-{row['phase']}-{row['replicate']}", row, original, 0)
    medians = {}
    for stack in ['engine', 'reference']:
        values = [r['elapsed_seconds'] for r in rows if r['stack'] == stack and r['phase'] == 'edit']
        require(suite['measurements'][stack]['samples_seconds'] == values, 'suite samples mismatch')
        median = statistics.median(values)
        same_number(summary['medians_seconds'][stack], median, 'fixed median mismatch')
        same_number(suite['measurements'][stack]['median_seconds'], median, 'suite median mismatch')
        medians[stack] = median
    require(summary['excluded'] == [], 'sample exclusion')
    return medians


def interleaved(root, folder, offset):
    state = load(root, folder + '/state.json')
    require(state['status'] == 'passed' and state['identities_unchanged']
            and state['source_restored'] and state['toolchain_unchanged'], 'interleaved admission')
    identity(state)
    require(set(state['configs']) == set(TOOLS), 'interleaved configurations')
    for stack in TOOLS:
        require(state['configs'][stack]['sha256'] == TOOLS[stack], 'interleaved binary identity')
    rows = state['rows']
    require(len(rows) == 141, 'interleaved exact row count')
    keys = {(s, p, 0) for s in TOOLS for p in ['prepare', 'type-error', 'type-recovery', 'assert-failure', 'assert-recovery']}
    keys |= {(s, 'edit', n) for s in TOOLS for n in range(42)}
    require({(r['stack'], r['phase'], r['replicate']) for r in rows} == keys, 'interleaved coverage')
    original = local(root, folder + '/original.rs').read_bytes()
    require(state['source_before']['src/summary.rs'] == sha(original), 'diagnostic original identity')
    for row in rows:
        config = state['configs'][row['stack']]
        command = [config['tool']] + (['cargo'] if config['kind'] == 'reference' else [])
        command += ['test', '--lib', '--', 'summary::tests::', '--test-threads=1', '--nocapture']
        require(row['command'] == command, 'interleaved command mismatch')
        require(all(row['environment'].get(k) == v for k, v in config['environment'].items()),
                'interleaved environment mismatch')
        require('RA_JIT_DIAGNOSTICS_STATS' not in row['environment'], 'enabled telemetry in latency run')
        row_check(root, f"{folder}/{row['phase']}-{row['replicate']}-{row['stack']}", row, original, offset)
    permutations = list(itertools.permutations(state['configs']))
    edits = [r for r in rows if r['phase'] == 'edit']
    for rep in range(42):
        triplet = edits[rep * 3:rep * 3 + 3]
        require(tuple(r['stack'] for r in triplet) == permutations[rep % 6], 'balanced permutation order')
        require(len({r['edit_sha256'] for r in triplet}) == 1, 'identical edited bytes across stacks')
    medians = {}
    for stack in TOOLS:
        median = statistics.median(r['elapsed_seconds'] for r in edits if r['stack'] == stack)
        same_number(state['medians_seconds'][stack], median, 'interleaved median mismatch')
        medians[stack] = median
    return medians


def validate(root):
    collection = load(root, 'collection.json')
    require(collection['baseline_commit'] == '8a04340343a67e0152b8c5b4d7d52d18dd8cf13a', 'baseline commit')
    seen = set()
    require(len(collection['files']) == 825, 'complete collected artifact inventory')
    for item in collection['files']:
        name = item['artifact']
        require(name not in seen, 'duplicate collected artifact')
        seen.add(name)
        require(digest(root, name) == item['sha256'], 'collected artifact hash: ' + name)
    sources(root)
    snapshot(root)
    native(root)
    old, new = fixed(root, 'rsb15', 'old'), fixed(root, 'rsa21', 'new')
    reference = load(root, 'fixed-five/rsb15/metadata.json')
    for path in ['fixed-five/rsa21/metadata.json', 'interleaved/state.json', 'interleaved-reverse/state.json']:
        state = load(root, path)
        require(state['source_before'] == reference['source_before']
                and state['toolchain_before'] == reference['toolchain_before'], 'cross-run source/toolchain drift')
    raw_delta = (new['engine'] / old['engine'] - 1) * 100
    normalized = ((new['engine'] / new['reference']) / (old['engine'] / old['reference']) - 1) * 100
    require(abs(raw_delta) < 10 and normalized > 0, 'fixed rejection interpretation')
    compare = text(root, 'provenance/suite-v24-snapshot-compare.log')
    require(f'({raw_delta:+.2f}%)' in compare and f'{normalized:+.2f}%' in compare
            and 'does not exceed requested 10% floor' in compare, 'comparison math disagreement')
    first = interleaved(root, 'interleaved', 1800)
    reverse = interleaved(root, 'interleaved-reverse', 2000)
    require(first['new'] < first['old'] and reverse['new'] > reverse['old'], 'independent benefit did replicate')
    return dict(verdict='passed', decision='reject, production restored', artifacts=len(seen),
                native_api_tests=3, full_resident_tests=351, raw_runtime_rows=326,
                fixed_raw_delta_percent=raw_delta, fixed_reference_normalized_delta_percent=normalized,
                interleaved_medians_seconds=first, reverse_medians_seconds=reverse,
                reverse_candidate_delta_percent=(reverse['new'] / reverse['old'] - 1) * 100,
                limitation='actual API tests used exact source snapshot with private lock resolution, not original nested --locked acceptance')


if __name__ == '__main__':
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--root', type=Path, default=Path(__file__).resolve().parent)
    args = parser.parse_args()
    try:
        print(json.dumps(validate(args.root), indent=2))
    except (ValueError, KeyError, OSError, TypeError) as error:
        parser.exit(1, f'FAIL: {error}\n')
