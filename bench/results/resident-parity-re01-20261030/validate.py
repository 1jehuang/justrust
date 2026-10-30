#!/usr/bin/env python3
"""Offline receipt audit. Never executes a tool, harness, compiler or workload."""
import ast
import hashlib
import json
from pathlib import Path
import re
import statistics
import sys

ROOT = Path(__file__).resolve().parent
ORIGIN = '5d4f78f7577cf7170993b74f735099dd03f5c6ca'
BUILD = 'resident-v1-bc9689b69a6f40e47b93a11f679daa197eb5c1fbc10b9898a310e9cac3efdfc4'
TOOLS = {'reference': 'fa622aeca09858c050264f701fbbebd1a14aa07d2b02c1eb4de511b684ba0263', 'engine': '726edc785271a1907e5d40619fde5239da031f535fff035b62c6650a8b1aafef'}
NAMES = sorted('summary::tests::' + n for n in ('parses_libtest_results', 'shares_overlapping_time', 'splits_passes_without_double_counting', 'test_failures_are_not_compile_errors'))
CASES = ['version-without-home', 'no-home', 'relative-home', 'shared-home', 'linked-home', 'private-status-hostile-aliases', 'reject-management', 'linked-namespace']

def require(value, message):
    if not value:
        raise ValueError(message)

def sha(data):
    return hashlib.sha256(data).hexdigest()

def validate(root=ROOT, integrity=True):
    root = Path(root)
    def read(name):
        return (root / name).read_text()
    def load(name):
        return json.loads(read(name))
    # Extract only these two pure functions from the exact retained helper.
    # Do not import its workload runner or platform-specific measurement modules.
    tree = ast.parse(read('harness/blaze_runtime_compare.py'))
    pure = ast.Module(body=[n for n in tree.body if isinstance(n, ast.FunctionDef) and n.name in ('edit_bytes', 'test_results')], type_ignores=[])
    scope = {'re': re}
    exec(compile(pure, '<retained-pure-helpers>', 'exec'), scope)
    parse, edit = scope['test_results'], scope['edit_bytes']
    m, rows, summary = [load('runtime/' + n + '.json') for n in ('metadata', 'records', 'summary')]
    require(m['status'] == 'complete' and m['unchanged'] and m['source_restored'] and m['toolchain_links_valid'], 'incomplete identity/restoration')
    require(m['pin'] == 'f97fcc2', 'source pin')
    source = {p.relative_to(root / 'source').as_posix(): sha(p.read_bytes()) for p in (root / 'source').rglob('*') if p.is_file()}
    require(len(source) == 63 and source == m['source_before'] == m['source_after'], '63 pristine source hashes')
    blobs = {}
    for line in read('provenance/workload-tree.txt').splitlines():
        info, name = line.split('\t', 1)
        mode, kind, oid = info.split()
        require(kind == 'blob' and mode in ('100644', '100755'), 'pinned tree kind')
        data = (root / 'source' / name).read_bytes()
        require(hashlib.sha1(b'blob ' + str(len(data)).encode() + b'\0' + data).hexdigest() == oid, 'pinned Git blob')
        blobs[name] = oid
    require(set(blobs) == set(source), 'pinned tree complete')
    require(read('provenance/baseline-head.txt').strip() == ORIGIN and read('provenance/baseline-status.txt') == '', 'pristine Blaze compiler origin')
    original = (root / 'runtime/source.original').read_bytes()
    require(original == (root / 'source/src/summary.rs').read_bytes(), 'original source backup')
    for key in ('tools', 'toolchain'):
        require(m[key + '_before'] == m[key + '_after'], key + ' identity changed')
    tc = m['toolchain_before']['files']
    require(tc['lib/rustlib/multirust-channel-manifest.toml']['sha256'] == '34f3d9bf04640a082e561310f3f258846397f4cf64a2bcfebcea96ebf18e2572', 'toolchain manifest pin')
    require(sha((root / 'provenance/channel-manifest.toml').read_bytes()) == tc['lib/rustlib/multirust-channel-manifest.toml']['sha256'], 'retained toolchain manifest')
    require(m['store_scope'] == summary['store_scope'] == 'unseeded private cold stores, no automatic seeding', 'no seed policy')
    require(summary['excluded'] == [] and summary['source_built_parity'] is True, 'parity summary')
    require('Descriptive only' in summary['policy'] and 'No general speed claim or independent optimization proof' in summary['policy'], 'descriptive policy')
    require(len(rows) == 22, 'complete runtime row count')
    keys = [(r['stack'], r['phase'], r['replicate']) for r in rows]
    wanted = {(s, p, i) for s in TOOLS for p, count in [('version', 1), ('prepare', 1), ('type-error', 1), ('type-recovery', 1), ('assert-failure', 1), ('assert-recovery', 1), ('edit', 5)] for i in range(count)}
    require(len(set(keys)) == len(keys) and set(keys) == wanted, 'runtime phase/replicate coverage')
    for r in rows:
        s, p, i = r['stack'], r['phase'], r['replicate']
        prefix = f'runtime/{s}-{p}-{i}'
        out, err = read(prefix + '.stdout'), read(prefix + '.stderr')
        text = out + '\n' + err
        require(r['error'] is None and r['timed_out'] is False, 'execution error')
        require(r['command'][0] == m['tools_before'][s]['path'], 'command tool identity')
        require(m['tools_before'][s]['sha256'] == TOOLS[s], 'tool hash pin')
        require(not any('seed' in a.lower() for a in r['command']), 'seed command')
        env = r['environment']
        require(env['BLAZE_HOME'] == m['environments'][s]['BLAZE_HOME'], 'state root')
        require(env['BLAZE_HOME'].startswith('/home/jeremy/re01/' + s + '/'), 'private stack root')
        require(all(env[k] == v for k, v in m['profiles'].items()), 'matched profiles')
        # These were launcher inputs. Native namespace enforcement is independently receipted below.
        require(m['environments'][s]['BLAZE_REMOTE'] == m['environments'][s]['RA_JIT_REMOTE'] == 'off', 'remote disabled')
        require(m['environments'][s]['BLAZE_PROC_MACRO_SERVER'] == m['environments'][s]['RA_JIT_PROC_MACRO_SERVER'] == '0', 'proc macro control')
        if p == 'version':
            require(r['status'] == 0 and text == m['versions'][s]['text'] and m['versions'][s]['valid'], 'version receipt')
            expected = read('build/' + ('baseline-build' if s == 'reference' else 'candidate-build-v2') + '/engine-version.txt')
            require(out.strip() == expected.strip() and ORIGIN in out, 'source-built version')
            if s == 'engine':
                require(BUILD in out and 'Blaze-derived origin' in out, 'compiled build identity')
            continue
        require(r['command'][1:] == (['cargo'] if s == 'reference' else []) + ['test', '--lib', '--', 'summary::tests::', '--test-threads=1', '--nocapture'], 'actual filtered runtime command')
        contents = edit(original, i) if p == 'edit' else edit(original, kind='type-error' if p == 'type-error' else 'test-failure') if p in ('type-error', 'assert-failure') else original
        require(r['source_sha256'] == sha(contents), 'actual edit source identity')
        if p == 'type-error':
            require(r['status'] == 101 and r['gate_passed'] and 'E0308' in err and 'JR_RUNTIME_TYPE_ERROR' in err and parse(text) is None and r['parsed'] is None, 'type failure gate')
            continue
        parsed = json.loads(json.dumps(parse(text)))
        failed = p == 'assert-failure'
        expected = {'tests': [[n, 'FAILED' if failed and n.endswith('shares_overlapping_time') else 'ok'] for n in NAMES], 'passed': 3 if failed else 4, 'failed': int(failed), 'ignored': 0, 'filtered': 144}
        require(parsed == expected == r['parsed'], 'actual names and libtest counts')
        require(r['status'] == (101 if failed else 0), 'runtime status')
        if failed:
            require(r['gate_passed'] and 'JR_RUNTIME_FAILURE' in err, 'assertion failure gate')
        markers = re.findall(r'JR_RUNTIME_EXECUTED_[^\s]*', text)
        require(markers == ([f'JR_RUNTIME_EXECUTED_{i + 1}'] if p == 'edit' else []), 'actual runtime marker count/value')
        if p == 'edit':
            require(r['accepted'] is True, 'edit not accepted')
    for s in TOOLS:
        median = statistics.median(r['elapsed_seconds'] for r in rows if r['stack'] == s and r['phase'] == 'edit')
        require(median == summary['medians_seconds'][s], 'median replay')
    require(summary['medians_seconds']['engine'] > summary['medians_seconds']['reference'], 'derived result is slower')
    for directory in ('baseline-build', 'candidate-build-v2'):
        require(read('build/' + directory + '/exit').strip() == '0', 'compiler build failed')
        require('justrust: ok: build succeeded, tests not run' in read('build/' + directory + '/build.log'), 'compiler build completion receipt')
    require(read('build/candidate-build-v2/compile-exit').strip() == read('build/runtime-v2.exit').strip() == '0', 'compile/runtime exit')
    require(TOOLS['engine'] in read('build/candidate-build-v2/engine-identity.sha256'), 'built native binary identity')
    for name in ('rustc', 'cargo'):
        require(tc['bin/' + name]['sha256'] in read('build/baseline-build/tool-identities.sha256'), 'compiler tool identity')
    require('nightly' in read('build/baseline-build/rustc-version.txt'), 'compiler version receipt')
    manifest = load('provenance/candidate-source-manifest.json')
    for line in read('build/candidate-build-v2/source-before.sha256').splitlines():
        digest, name = line.split(None, 1)
        require(manifest[name] == digest, 'candidate source build receipt')
        require(name + ': OK' in read('build/candidate-build-v2/source-after.log'), 'candidate source-after receipt')
    for item in load('provenance/candidate-build-inputs.json'):
        require(manifest[item['path']] == item['sha256'] and item['bytes'] >= 0, 'full build input hash receipt')
    entry = load('entry/acceptance.json')
    require(entry['status'] == 'passed' and entry['sha256_before'] == entry['sha256_after'] == TOOLS['engine'], 'namespace engine identity')
    require([r['case'] for r in entry['rows']] == CASES, 'eight native entry cases')
    for row in entry['rows']:
        name = row['case']
        require(row['status'] == (0 if name in ('version-without-home', 'private-status-hostile-aliases') else 1), 'namespace status')
        for suffix in ('stdout', 'stderr'):
            require(read('entry/' + name + '.' + suffix) == row[suffix], 'namespace raw output')
        if name == 'private-status-hostile-aliases':
            require('/private/resident-v1/store/' + BUILD in row['stdout'], 'native private versioned namespace')
    if integrity:
        sealed = load('SHA256.json')
        actual = {p.relative_to(root).as_posix(): sha(p.read_bytes()) for p in root.rglob('*') if p.is_file() and p.name != 'SHA256.json' and '__pycache__' not in p.parts}
        require(actual == sealed, 'bundle allowlist/integrity seal')
    return {'accepted_runtime_edits': 10, 'native_entry_cases': 8, 'source_files': 63, 'medians_seconds': summary['medians_seconds'], 'claim': 'slower, descriptive Blaze-derived parity only'}

if __name__ == '__main__':
    try:
        print(json.dumps(validate(Path(sys.argv[1]) if len(sys.argv) > 1 else ROOT), indent=2))
    except (ValueError, KeyError, OSError, TypeError) as error:
        sys.exit('INVALID: ' + str(error))
