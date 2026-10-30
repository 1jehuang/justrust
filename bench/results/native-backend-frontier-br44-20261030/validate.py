#!/usr/bin/env python3
"""Portable, standard-library-only evidence replay. Never invokes a compiler."""
import hashlib
import json
from pathlib import Path
import re
import statistics

ROOT = Path(__file__).resolve().parent
STACKS = ['owned-off', 'all3on', 'blaze', 'blaze-llvm', 'owned-clif', 'owned-threads4', 'owned-clif-threads4']
TESTS = ['summary::tests::' + n for n in ('parses_libtest_results', 'shares_overlapping_time', 'splits_passes_without_double_counting', 'test_failures_are_not_compile_errors')]
FLAGS = ['jr-empty-lint-level-fast-path', 'jr-cache-small-query-values', 'jr-cache-associated-items']
MEDIANS = dict(zip(STACKS, [1.2888577339908807, 1.2924911129957763, .10071881199837662, .11157403100514784, 1.2844904080120614, 1.1983482399955392, 1.186774098998285]))

def digest(raw):
    return hashlib.sha256(raw).hexdigest()

def load(name):
    return json.loads((ROOT / name).read_text())

def parse(output):
    names = re.findall(r'^test ([^\s]+) \.\.\. (?:(?:[^\n]*\n)*?)(ok|FAILED|ignored)(?: <[0-9]+(?:\.[0-9]+)?s>)?[ \t]*$', output, re.M)
    totals = re.findall(r'test result: (ok|FAILED)\. (\d+) passed; (\d+) failed; (\d+) ignored; (\d+) measured; (\d+) filtered out', output)
    if not totals or not names:
        return None
    assert len(totals) == 1 and len(dict(names)) == len(names)
    state, passed, failed, ignored, measured, filtered = totals[0]
    assert [int(passed), int(failed), int(ignored)] == [sum(s == x for _, s in names) for x in ('ok', 'FAILED', 'ignored')]
    assert int(measured) == 0 and state == ('FAILED' if int(failed) else 'ok')
    return dict(tests=sorted([list(x) for x in names]), passed=int(passed), failed=int(failed), ignored=int(ignored), filtered=int(filtered))

def main():
    manifest = load('sha256.json')
    files = {str(p.relative_to(ROOT)) for p in ROOT.rglob('*') if p.is_file() and p.name not in ('sha256.json', 'validate.py') and '__pycache__' not in p.parts}
    assert files == set(manifest), 'artifact inventory changed'
    for name, sha in manifest.items():
        assert digest((ROOT / name).read_bytes()) == sha, name
    m, rows, summary = load('metadata.json'), load('records.json'), load('summary.json')
    assert m['stacks'] == STACKS and m['samples'] == 5
    assert m['unchanged'] and m['source_restored']
    assert m['source_before'] == m['source_after']
    assert m['identities_before'] == m['identities_after']
    assert m['identities_before']['rustc_driver_0']['sha256'] == 'ed21691a9db267ffb71cc3b8975a8cd89768397a7a7f82c9c839083485424efd'
    native_key = 'native_codegen:lib/rustlib/x86_64-unknown-linux-gnu/codegen-backends/librustc_codegen_cranelift-1.101.0-nightly.so'
    assert m['identities_before'][native_key]['sha256'] == '4d5659e20a9d943765e0a7eceb88301aaf9b8dcfbf6adfe6357938299023d2c7'
    assert m['seed_source_before'] == m['seed_source_after']
    assert m['store_scope'] == summary['store_scope'] == 'warm-store seeded'
    assert set(m['blaze_seed_manifests']) == {'blaze', 'blaze-llvm'}
    for seed in m['blaze_seed_manifests'].values():
        assert seed['files'] == m['seed_source_before'] and seed['files']
    version = load('rustc-version.json')
    assert version == m['compiler_version'] and version['nightly']
    assert version['sha256_before'] == version['sha256_after'] == m['identities_before']['rustc']['sha256']
    original = (ROOT / 'source.original').read_bytes()
    assert digest(original) == m['source_before']['src/summary.rs']
    expected = dict(tests=[[n, 'ok'] for n in TESTS], passed=4, failed=0, ignored=0, filtered=144)
    assert m['expected_tests'] == expected
    assert len(rows) == 70
    phases = ('prepare', 'type-error', 'type-recovery', 'test-failure', 'test-recovery')
    assert [(r['stack'], r['phase'], r['replicate']) for r in rows[:35]] == [(s, p, 0) for s in STACKS for p in phases]
    keys = {(r['stack'], r['phase'], r['replicate']) for r in rows}
    assert len(keys) == 70
    assert keys == {(s, p, i) for s in STACKS for p in ('prepare', 'type-error', 'type-recovery', 'test-failure', 'test-recovery', 'edit') for i in (range(5) if p == 'edit' else [0])}
    owned_ids = {s: set() for s in STACKS if not s.startswith('blaze')}
    anchor = b'fn shares_overlapping_time() {'
    assert original.count(anchor) == 1
    for r in rows:
        s, phase, rep = r['stack'], r['phase'], r['replicate']
        prefix = f'{phase}-{rep:03d}-{s}'
        env = r['environment']
        assert not r['error'] and not r['timed_out'] and r['elapsed_seconds'] > 0
        assert all(env[k] == v for k, v in m['profiles'].items())
        assert r['command'][1:] == ['test', '--lib', '--', 'summary::tests::', '--test-threads=1', '--nocapture']
        if s.startswith('blaze'):
            assert r['command'][0] == m['identities_before']['blaze']['path']
            assert env['BLAZE_BACKEND'] == ('llvm' if s == 'blaze-llvm' else 'cranelift') and env['BLAZE_OPT'] == '0'
            output = ''.join((ROOT / (prefix + '.' + suffix)).read_text() for suffix in ('stdout', 'stderr'))
        else:
            assert r['command'][0] == m['identities_before']['justrust']['path']
            assert env['RUSTC'] == m['identities_before']['rustc']['path']
            assert env['CARGO'] == m['identities_before']['cargo']['path']
            flags = ' '.join('-Z' + f + '=' + ('yes' if s == 'all3on' else 'no') for f in FLAGS)
            if s in ('owned-clif', 'owned-clif-threads4'):
                flags += ' -Zcodegen-backend=cranelift'
            if s in ('owned-threads4', 'owned-clif-threads4'):
                flags += ' -Zthreads=4'
            assert env['RUSTFLAGS'] == flags
            for k in ('JUSTRUST_REMOTE', 'JUSTRUST_PREBUILT'):
                assert env[k] == 'off'
            assert env['JUSTRUST_CACHE_PROC_MACROS'] == '0'
            raw = r['raw_output']
            assert raw['artifact'] == prefix + '.raw-output.jsonl'
            data = (ROOT / raw['artifact']).read_bytes()
            assert digest(data) == raw['sha256']
            entries = [json.loads(line) for line in data.splitlines()]
            assert entries and all(e['s'] in ('o', 'e') and isinstance(e['l'], str) for e in entries)
            output = '\n'.join(e['l'] for e in entries) + '\n'
            assert raw['run_id'] not in owned_ids[s]
            owned_ids[s].add(raw['run_id'])
            assert not raw['hotpatch']['present']
        contents = original
        if phase in ('type-error', 'test-failure', 'edit'):
            n = rep + 1
            body = (f'let observed = std::hint::black_box({n}u64); assert_eq!(observed, {n}u64); println!("JR_RUNTIME_EXECUTED_{n}");').encode()
            if phase == 'type-error':
                body = b'let _: u64 = "JR_RUNTIME_TYPE_ERROR";'
            elif phase == 'test-failure':
                body = b'assert_eq!(std::hint::black_box(1u64), 2u64, "JR_RUNTIME_FAILURE");'
            contents = original.replace(anchor, anchor + b'\n' + body, 1)
        assert digest(contents) == r['source_sha256']
        parsed = parse(output)
        assert parsed == r['parsed']
        if phase == 'type-error':
            assert r['status'] == 101 and r['gate_passed'] and 'E0308' in output and 'JR_RUNTIME_TYPE_ERROR' in output and parsed is None
        elif phase == 'test-failure':
            failed = dict(expected, passed=3, failed=1, tests=[[n, 'FAILED' if n.endswith('shares_overlapping_time') else 'ok'] for n in TESTS])
            assert r['status'] == 101 and r['gate_passed'] and parsed == failed and 'JR_RUNTIME_FAILURE' in output
        else:
            assert r['status'] == 0 and parsed == expected
        markers = re.findall(r'JR_RUNTIME_EXECUTED_\d+', output)
        assert markers == ([f'JR_RUNTIME_EXECUTED_{rep + 1}'] if phase == 'edit' else [])
        if phase == 'edit':
            assert r['accepted']
    edits = [r for r in rows if r['phase'] == 'edit']
    assert len(edits) == 35
    assert [r['stack'] for r in edits] == [s for i in range(5) for s in STACKS[i:] + STACKS[:i]]
    medians = {s: statistics.median(r['elapsed_seconds'] for r in edits if r['stack'] == s) for s in STACKS}
    assert medians == summary['medians_seconds'] == MEDIANS and not summary['excluded']
    for s, ids in owned_ids.items():
        name = 'owned-on' if s == 'all3on' else s
        paths = sorted((ROOT / 'exports' / f'xps14-runtime-br44-{name}-20261030').glob('*.jsonl'))
        exports = [json.loads(line) for p in paths for line in p.read_text().splitlines()]
        assert len(exports) == 10 and {e['id'] for e in exports} == ids
        assert all(e['machine'] == f'xps14-runtime-br44-{name}-20261030' for e in exports)
        repository_exports = ROOT.parents[2] / 'data' / 'runs' / f'xps14-runtime-br44-{name}-20261030'
        if repository_exports.is_dir():
            for path in paths:
                assert (repository_exports / path.name).read_bytes() == path.read_bytes()
    print('PASS: 7 stacks, 35 edits, 70 rows, exact tests/markers, negative/recovery gates, identities, backends, medians, hashes, 50 exports')

if __name__ == '__main__':
    main()
