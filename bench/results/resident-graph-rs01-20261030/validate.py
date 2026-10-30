#!/usr/bin/env python3
"""Portable offline replay of native suite gates and comparison, never a workload."""
import ast
import contextlib
import hashlib
import io
import json
import math
from pathlib import Path
import re
import statistics
import sys
import types

ROOT = Path(__file__).resolve().parent
ORIGIN = '5d4f78f7577cf7170993b74f735099dd03f5c6ca'
REFERENCE = 'fa622aeca09858c050264f701fbbebd1a14aa07d2b02c1eb4de511b684ba0263'
ENGINES = {'rsb01': ('726edc785271a1907e5d40619fde5239da031f535fff035b62c6650a8b1aafef', 'resident-v1-bc9689b69a6f40e47b93a11f679daa197eb5c1fbc10b9898a310e9cac3efdfc4', 'candidate-build-v2'), 'rsa01': ('deafad0c76f20059db6e1a07f4ed9665013ee61600db0089aec3667c38069df8', 'resident-v1-c44b9049b4557e5ff69a31664fe9a01e9ed0eb45643aa2fecc81ecb4df2fda18', 'candidate-build-v4')}


def require(condition, message):
    if not condition:
        raise ValueError(message)


def sha(data):
    return hashlib.sha256(data).hexdigest()


def pure_functions(path, names, scope):
    """Load explicitly named original functions only, never imports or entrypoints."""
    tree = ast.parse(path.read_text())
    body = [n for n in tree.body if isinstance(n, ast.FunctionDef) and n.name in names]
    require({n.name for n in body} == set(names), 'missing exact retained helper')
    exec(compile(ast.Module(body=body, type_ignores=[]), str(path), 'exec'), scope)
    return scope


def engine_helpers(root):
    runtime = pure_functions(root / 'harness/blaze_runtime_compare.py', ['edit_bytes', 'test_results'], {'re': re})
    runtime['PROFILES'] = {f'CARGO_PROFILE_{p}_{k}': v for p in ('DEV', 'TEST') for k, v in (('OPT_LEVEL', '0'), ('DEBUG', '0'), ('INCREMENTAL', 'true'))}
    scope = {'Path': Path, 'NAMES': sorted('summary::tests::' + n for n in ('shares_overlapping_time', 'splits_passes_without_double_counting', 'parses_libtest_results', 'test_failures_are_not_compile_errors')), 'EDIT_TEST': 'summary::tests::shares_overlapping_time', 'FILTER': ['--lib', '--', 'summary::tests::', '--test-threads=1', '--nocapture'], 'runtime': types.SimpleNamespace(**runtime)}
    pure_functions(root / 'harness/resident_engine_compare.py', ['command', 'valid_tests'], scope)
    scope.update(PIN='f97fcc2', REFERENCE_VERSION=f'blaze 0.20261027.0 ({ORIGIN})', TOOLCHAIN_MANIFEST='34f3d9bf04640a082e561310f3f258846397f4cf64a2bcfebcea96ebf18e2572')
    return types.SimpleNamespace(**scope)


def audit(root=ROOT, integrity=True):
    root = Path(root)
    def read(name):
        return (root / name).read_text()
    def load(name):
        return json.loads(read(name))
    if integrity:
        actual = {p.relative_to(root).as_posix(): sha(p.read_bytes()) for p in root.rglob('*') if p.is_file() and p.name != 'SHA256.json' and '__pycache__' not in p.parts}
        require(actual == load('SHA256.json'), 'bundle allowlist/integrity seal')
    source = {p.relative_to(root / 'source').as_posix(): sha(p.read_bytes()) for p in (root / 'source').rglob('*') if p.is_file()}
    require(len(source) == 63, 'complete source63')
    blobs = {}
    for line in read('provenance/workload-tree.txt').splitlines():
        info, name = line.split('\t', 1)
        mode, kind, oid = info.split()
        require(kind == 'blob' and mode in ('100644', '100755'), 'source tree kind')
        data = (root / 'source' / name).read_bytes()
        require(hashlib.sha1(b'blob ' + str(len(data)).encode() + b'\0' + data).hexdigest() == oid, 'pristine Git source blob')
        blobs[name] = oid
    require(set(source) == set(blobs), 'source63 pinned tree')
    harness = engine_helpers(root)
    # Exact original native replay and compare functions, with only validation's
    # external path/Git lookup replaced by a receipt-root resolver below.
    native = pure_functions(root / 'harness/resident_suite.py', ['require', 'digest', 'replay', 'compare'], {'Path': Path, 'hashlib': hashlib, 'json': json, 'math': math, 'statistics': statistics, 're': re, 'harness': harness, 'SCHEMA': 'native_resident/v1'})
    results = {session: load(session + '/suite.json') for session in ENGINES}
    all_metadata = {}

    def validate(result):
        session = next((name for name in ENGINES if result.get('artifacts_root') == '/home/jeremy/' + name), None)
        require(session is not None, 'suite original root identity')
        require(result['schema'] == 'native_resident/v1', 'native suite schema')
        metadata, samples, receipts = native['replay'](root / session, source)
        require(receipts == result['artifacts_sha256'], 'native raw artifact receipts')
        require(samples == result['measurements'], 'native sample receipts')
        require(metadata == result['metadata'], 'native metadata receipt')
        require(result['noise'] == 'unknown' and result['scope'] == 'scoped diagnostic, not comprehensive or default promotion', 'descriptive/noise scope')
        host = result['host']
        require(all(host.get(k) for k in ('host', 'cpu', 'ncpu', 'machine', 'kernel')) and host['execution'] == 'local native direct', 'host execution identity')
        all_metadata[session] = metadata
        rows = load(session + '/records.json')
        summary = load(session + '/summary.json')
        require(len(rows) == 22 and metadata['status'] == 'complete' and metadata['unchanged'] is True, 'complete 22-row gates')
        require(summary['source_built_parity'] is True and summary['excluded'] == [], 'summary parity')
        require(summary['store_scope'] == metadata['store_scope'] and summary['proc_macro_policy'] == metadata['proc_macro_policy'], 'summary policy consistency')
        require(summary['toolchain'] == metadata['toolchain_before']['path'], 'summary toolchain identity')
        require('Descriptive only' in summary['policy'] and 'No general speed claim or independent optimization proof' in summary['policy'], 'no performance promotion')
        digest, build_id, directory = ENGINES[session]
        require(metadata['tools_before']['reference']['sha256'] == REFERENCE and metadata['tools_before']['engine']['sha256'] == digest, 'immutable tool identity')
        require(metadata['toolchain_before']['files']['lib/rustlib/multirust-channel-manifest.toml']['sha256'] == sha((root / 'provenance/channel-manifest.toml').read_bytes()), 'retained toolchain manifest')
        for row in rows:
            stack, phase, rep = row['stack'], row['phase'], row['replicate']
            text = '\n'.join(read(f'{session}/{stack}-{phase}-{rep}.{suffix}') for suffix in ('stdout', 'stderr'))
            parsed = json.loads(json.dumps(harness.runtime.test_results(text)))
            require(parsed == row['parsed'], 'recorded parsed result matches raw')
            if parsed is not None:
                require(parsed['filtered'] == 144, 'actual runtime filtered count')
            require(not any('seed' in arg.lower() for arg in row['command']), 'seed command absent')
            env = row['environment']
            require(env['BLAZE_HOME'] == metadata['environments'][stack]['BLAZE_HOME'] and env['BLAZE_HOME'].startswith('/home/jeremy/' + session + '/' + stack + '/'), 'private stack state root')
            require(all(env[k] == v for k, v in harness.runtime.PROFILES.items()), 'per-row matched profiles')
            if phase == 'version':
                require(metadata['versions'][stack]['valid'] is True and metadata['versions'][stack]['text'] == text, 'version metadata matches raw')
                expected = read('build/' + ('baseline-build' if stack == 'reference' else directory) + '/engine-version.txt').strip()
                require(text.strip() == expected and ORIGIN in text, 'source-built origin version')
                if stack == 'engine':
                    require(build_id in text and 'Blaze-derived origin' in text, 'compiled source build identity')
            elif phase in ('type-error', 'assert-failure'):
                require(row['gate_passed'] is True, 'accepted failure gate')
            if phase == 'edit':
                require(row['accepted'] is True, 'accepted runtime edit')
            elif phase != 'type-error':
                require(not re.findall(r'JR_RUNTIME_EXECUTED_[^\s]*', text), 'no stale edit marker in recovery/prepare')
        for stack in ('reference', 'engine'):
            require(summary['medians_seconds'][stack] == samples[stack]['median_seconds'], 'summary median replay')
        return metadata, samples

    native['validate'] = validate
    buffer = io.StringIO()
    with contextlib.redirect_stdout(buffer):
        native['compare'](results['rsb01'], results['rsa01'], floor=10.0)
    comparison = buffer.getvalue()
    ma, mb = all_metadata['rsb01'], all_metadata['rsa01']
    require(ma['toolchain_before'] == mb['toolchain_before'] and ma['source_before'] == mb['source_before'], 'immutable paired inputs')
    require(ma['tools_before']['engine']['sha256'] != mb['tools_before']['engine']['sha256'], 'actual candidate binary differs')
    tc = ma['toolchain_before']['files']
    for name in ('rustc', 'cargo'):
        require(tc['bin/' + name]['sha256'] in read('build/baseline-build/tool-identities.sha256'), 'compiler tool receipts')
    for directory in ('baseline-build', 'candidate-build-v2', 'candidate-build-v3', 'candidate-build-v4'):
        require(read(f'build/{directory}/exit').strip() == '0', 'native compiler build exit')
        require('justrust: ok: build succeeded, tests not run' in read(f'build/{directory}/build.log'), 'native compiler build success')
    require(read('build/candidate-build-v4/compile-exit').strip() == '0', 'v4 compile exit')
    require(ENGINES['rsa01'][0] in read('build/candidate-build-v4/engine-identity.sha256'), 'v4 built tool hash')
    require(ENGINES['rsb01'][0] in read('build/candidate-build-v2/engine-identity.sha256'), 'v2 baseline tool hash')
    for version, manifest_name in [('v2', 'baseline-v2-candidate-source-manifest.json'), ('v4', 'candidate-v4-source-manifest.json')]:
        manifest = load('provenance/' + manifest_name)
        for line in read(f'build/candidate-build-{version}/source-before.sha256').splitlines():
            digest, name = line.split(None, 1)
            require(manifest[name] == digest, 'pre-build source hash')
            require(name + ': OK' in read(f'build/candidate-build-{version}/source-after.log'), 'post-build source receipt')
            if version == 'v4':
                require(name + ': OK' in read('build/candidate-unit-v4/source-after.log'), 'post-unit source receipt')
    # Retain the rejected fixture compilation as a rejection, not a test pass.
    require(read('build/candidate-unit-v3/exit').strip() == read('build/candidate-unit-v3/graph-exit').strip() == '101', 'v3 rejection exit preserved')
    rejected = read('build/candidate-unit-v3/graph.log')
    require(all(s in rejected for s in ('E0603', 'worker_named', 'is private', 'E0308', 'expected `&str`, found `String`', 'FAILED to compile')), 'v3 fixture compile errors preserved')
    require(read('build/candidate-unit-v4/exit').strip() == '0', 'v4 unit overall exit')
    wanted = {'resident_graph_reuse': (1, 345), 'the_roots_of_a_listed_crate': (1, 345), 'telemetry': (3, 343)}
    for filt, (count, filtered) in wanted.items():
        require(read(f'build/candidate-unit-v4/{filt}.exit').strip() == '0', 'v4 unit filter exit')
        wrapper = read(f'build/candidate-unit-v4/{filt}.log')
        ids = re.findall(r'justrust log (\d{8}-\d{9}-\d+)', wrapper)
        require(len(ids) == 1, 'v4 raw unit run identity')
        base = f'build/candidate-unit-v4/runs/{ids[0]}/'
        run_meta = load(base + 'meta.json')
        require(run_meta['args'] == ['test', '--release', '--locked', '-p', 'ra-jit', '--bin', 'jr-engine', '-j', '6', '--', filt, '--nocapture'], 'actual native unit filter command')
        for name in ('rustc', 'cargo'):
            require(run_meta['real_' + name] == ma['toolchain_before']['path'] + '/bin/' + name, 'unit compiler identity')
        run_summary = load(base + 'summary.json')
        require(run_summary['exit'] == 0 and run_summary['args'] == run_meta['args'], 'v4 actual unit summary')
        output = '\n'.join(json.loads(line)['l'] for line in read(base + 'output.jsonl').splitlines())
        parsed = harness.runtime.test_results(output)
        require(parsed is not None and parsed['passed'] == count and parsed['failed'] == parsed['ignored'] == 0 and parsed['filtered'] == filtered, 'actual v4 unit test counts')
        require(all(state == 'ok' for name, state in parsed['tests']), 'actual v4 unit result')
        if filt == 'resident_graph_reuse':
            require(parsed['tests'] == [('tests::resident_graph_reuse_tracks_nested_items_and_macro_edits', 'ok')], 'actual graph fixture name')
        elif filt == 'the_roots_of_a_listed_crate':
            require(parsed['tests'] == [('tests::the_roots_of_a_listed_crate_are_its_functions_but_those_only_the_const_evaluator_runs', 'ok')], 'actual listed-root fixture name')
        else:
            require(parsed['tests'] == [(name, 'ok') for name in sorted(('cargo::telemetry_tests::both_telemetry_spellings_are_client_only', 'cargo::telemetry_tests::disabled_phase_needs_no_clock_or_span', 'cargo::cli::tests::telemetry_does_not_select_a_build_or_worker'))], 'actual telemetry fixture names')
    report = {'schema': 'retained_resident_graph/v1', 'accepted_runtime_edits': 20, 'raw_runtime_rows': 44, 'source_files': 63, 'v3_unit': 'rejected fixture compile errors E0603/E0308 retained', 'v4_unit': 'graph/listed/telemetry 1+1+3 passed', 'noise': 'unknown', 'claim': 'Both engine and reference improved. No attributable speedup, faster-than-Blaze conclusion or promotion.', 'native_comparison': comparison}
    return report

if __name__ == '__main__':
    try:
        print(json.dumps(audit(Path(sys.argv[1]) if len(sys.argv) > 1 else ROOT), indent=2))
    except (ValueError, KeyError, OSError, TypeError, StopIteration) as error:
        sys.exit('INVALID: ' + str(error))
