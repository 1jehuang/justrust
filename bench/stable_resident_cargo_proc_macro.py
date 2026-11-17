#!/usr/bin/env python3
"""Actual-client proc-macro lifecycle acceptance, not a performance benchmark.

Execution requires a frozen candidate and explicit idle authorization. Baseline
fallback is labeled separately from required root RPC and same-worker isolation.
"""
import argparse
import hashlib
import json
import math
from pathlib import Path
import re
import uuid

import stable_resident_cargo_dependency as dependency

shared = dependency.shared
safety = dependency.safety
SCHEMA = 'stable-resident-cargo-proc-macro/v1'
MANIFEST = dependency.MANIFEST.replace('fixture_dep', 'fixture_macro')
DEP_MANIFEST = dependency.DEP_MANIFEST.replace('fixture_dep', 'fixture_macro') + '\n[lib]\nproc-macro=true\n'
MACRO = br'''extern crate proc_macro;
use proc_macro::TokenStream;
use std::sync::atomic::{AtomicUsize, Ordering};
static COUNTER: AtomicUsize = AtomicUsize::new(0);
#[proc_macro]
pub fn probe(input: TokenStream) -> TokenStream {
    let count = COUNTER.fetch_add(1, Ordering::SeqCst) + 1;
    let name = input.to_string();
    let nonce = std::env::var("JR_PM_NONCE").expect("request nonce");
    let value = match std::env::var("JR_PM_VALUE") {
        Ok(value) => value,
        Err(_) => return format!("compile_error!({:?});", format!("JR_PM_ERROR={}:env", nonce)).parse().unwrap(),
    };
    let path = std::env::var("JR_PM_FILE").expect("input path");
    let file = match std::fs::read_to_string(path) {
        Ok(value) => value,
        Err(_) => return format!("compile_error!({:?});", format!("JR_PM_ERROR={}:file", nonce)).parse().unwrap(),
    };
    format!("const {}: (usize, &str, &str, &str) = ({}, {:?}, {:?}, {:?});",
            name, count, value, file, nonce).parse().unwrap()
}
'''


def stages(nonce):
    rows = []
    specs = [('initial', 'env-a', 'file-a', None),
             ('root-edit', 'env-a', 'file-a', None),
             ('env-edit', 'env-b', 'file-a', None),
             ('file-edit', 'env-b', 'file-b', None),
             ('env-error', None, 'file-b', 'env'),
             ('env-recovery', 'env-c', 'file-b', None),
             ('file-error', 'env-c', None, 'file'),
             ('file-recovery', 'env-c', 'file-c', None),
             ('root-final', 'env-c', 'file-c', None)]
    for name, env, file, error in specs:
        token = f'{nonce}-{name}'
        root = f'''// forced actual root edit: {token}
fixture_macro::probe!(A);
fixture_macro::probe!(B);
#[test]
fn proc_macro_nonce_probe() {{
    let mut counts = [A.0, B.0];
    counts.sort();
    assert_eq!(counts, [1, 2]);
    assert_eq!((A.1, A.2, A.3), ({json.dumps(env or '')}, {json.dumps(file or '')}, {json.dumps(token)}));
    assert_eq!((B.1, B.2, B.3), (A.1, A.2, A.3));
    eprintln!("JR_PM_RUNTIME={{}}:1,2:{{}}:{{}}", A.3, A.1, A.2);
}}
'''.encode()
        if error:
            root = (f'// forced actual root edit: {token}\n'
                    'fixture_macro::probe!(A);\nfixture_macro::probe!(B);\n'
                    '#[test]\nfn proc_macro_nonce_probe() {}\n').encode()
        rows.append(dict(name=name, root=root, env=env, file=file, token=token,
                         expectation='error' if error else 'pass', error=error,
                         nonce=None if error else f'{token}:1,2:{env}:{file}', root_only=name != 'initial'))
    return rows


def semantics(record, stage):
    stdout, stderr, cargo = dependency.messages(record)
    if record.get('timed_out'):
        raise AssertionError('Cargo timed out')
    markers = re.findall(r'^JR_PM_RUNTIME=(.*)$', stderr, re.MULTILINE)
    diagnostics = [m['message'] for m in cargo if m.get('reason') == 'compiler-message']
    errors = [m for m in diagnostics if m.get('level') == 'error']
    if stage['error']:
        expected = f"JR_PM_ERROR={stage['token']}:{stage['error']}"
        # Both expansions must observe the same current request input failure.
        observed = [m.get('message') for m in errors]
        if (record['status'] != 101 or markers or observed != [expected, expected]
                or re.search(r'^test proc_macro_nonce_probe \.\.\.', stdout, re.MULTILINE)):
            raise AssertionError('Exact current-request diagnostic nonce mismatch or stale test execution')
        return dict(status=101, markers=[], errors=observed)
    tests = re.findall(r'^test (\S+) \.\.\. (ok|FAILED|ignored)(?: <\d+(?:\.\d+)?s>)?$', stdout, re.MULTILINE)
    if (record['status'] != 0 or errors or markers != [stage['nonce']]
            or tests != [('proc_macro_nonce_probe', 'ok')]
            or 'test result: ok. 1 passed; 0 failed;' not in stdout):
        raise AssertionError('Proc macro counter/input/runtime nonce mismatch')
    return dict(status=0, markers=markers, errors=[])


def validate_artifacts(observations, previous):
    roots = [r for r in observations if r.get('target', {}).get('name') == 'dependency_fallback_root']
    if any(r.get('fresh') is not False for r in roots):
        raise AssertionError('Forced actual root edit was silently Cargo-fresh')
    macros = [r for r in observations if r.get('target', {}).get('name') == 'fixture_macro']
    if not macros or (previous is not None and any(r.get('fresh') is not True for r in macros)):
        raise AssertionError('Proc macro dependency rebuilt instead of exercising request isolation')
    current = {a['path']: a['sha256'] for r in macros for a in r['artifacts']}
    if not current or any(not digest for digest in current.values()):
        raise AssertionError('Missing proc macro artifact identity')
    if previous is not None and current != previous:
        raise AssertionError('Proc macro artifact changed across root requests')
    return current


def capture_arm(compiler, cargo, fixture, arm, mode, workload, timeout, runner,
                require_resident_root=False):
    arm.mkdir(mode=0o700)
    trace = arm / 'trace.jsonl'
    env = safety.safe_environment(shared.environment(
        compiler, fixture, mode, shared.private_directory(arm / 'workers'), trace))
    if require_resident_root:
        env['JUSTRUST_CARGO_RESIDENT_EXTERNAL'] = '1'
    report = dict(mode=mode, verdict='incomplete', observations=[])
    previous_macro = previous_root = None
    try:
        for index, stage in enumerate(workload):
            (fixture / 'src/lib.rs').write_bytes(stage['root'])
            input_path = fixture / 'macro-input.txt'
            if stage['file'] is None:
                input_path.unlink(missing_ok=True)
            else:
                input_path.write_text(stage['file'])
            env['JR_PM_NONCE'] = stage['token']
            env['JR_PM_FILE'] = str(input_path)
            env.pop('JR_PM_VALUE', None)
            if stage['env'] is not None:
                env['JR_PM_VALUE'] = stage['env']
            (arm / f'{index:02d}-root.rs').write_bytes(stage['root'])
            before = len(shared.trace_events(trace))
            row = dict(name=stage['name'], root_sha256=hashlib.sha256(stage['root']).hexdigest(),
                       inputs={k: stage[k] for k in ('env', 'file', 'token')})
            report['observations'].append(row)
            try:
                row['cargo'] = runner([cargo, 'test', '--offline', '--lib', '--message-format=json',
                                       '--', '--nocapture', '--test-threads=1'], timeout, cwd=fixture, env=env)
            finally:
                row['trace'] = shared.trace_events(trace)[before:]
            row['metadata'] = dependency.metadata(row['cargo'])
            row['semantics'] = semantics(row['cargo'], stage)
            if not stage['error'] and not any(r.get('target', {}).get('name') == 'dependency_fallback_root'
                                               for r in row['metadata']):
                raise AssertionError('Missing actual compiled root artifact')
            previous_macro = validate_artifacts(row['metadata'], previous_macro)
            row['residency'] = dependency.validate_trace(stage, row['trace'], mode, require_resident_root)
            if require_resident_root and mode == 'on':
                previous_root = dependency.validate_root_transition(stage, row['residency'], previous_root)
        report['verdict'] = 'passed'
    except BaseException as error:
        report.update(verdict='failed', error=f'{type(error).__name__}: {error}')
        raise
    finally:
        (arm / 'receipt.json').write_text(json.dumps(report, indent=2) + '\n')
    return report


def capture(compiler, cargo, output, *, justrust, root_native_frozen=False,
            idle_authorized=False, timeout=180, require_resident_root=False,
            expected_compiler_sha256=None, expected_driver_sha256=None,
            expected_cargo_sha256=None, expected_client_sha256=None, runner=None):
    if root_native_frozen is not True or idle_authorized is not True:
        raise ValueError('Root native freeze and idle authorization required before execution')
    if not math.isfinite(timeout) or timeout <= 0:
        raise ValueError('Finite positive timeout required')
    if not justrust:
        raise ValueError('Actual justrust client required')
    shared.check_socket_path(output, 1)
    compiler, cargo, client = (Path(p).resolve(strict=True) for p in (compiler, cargo, justrust))
    expected = dict(compiler_sha256=expected_compiler_sha256, driver_sha256=expected_driver_sha256,
                    cargo_sha256=expected_cargo_sha256, client_sha256=expected_client_sha256)
    attestation = safety.attest(compiler, cargo, client, expected)
    root = shared.private_directory(output)
    report = dict(schema=SCHEMA, verdict='incomplete', arms={}, claim=(
        'Required root RPC same-worker proc-macro request isolation correctness. '
        if require_resident_root else 'Conservative proc-macro fallback correctness only. ') +
        'No retained typed-state or performance claim.', provenance=dict(
            attestation=attestation, expected=expected, root_native_frozen=True, idle_authorized=True,
            require_resident_root=require_resident_root, timeout=timeout,
            harness_sha256=shared.native.digest(Path(__file__)),
            helpers={Path(m.__file__).name: shared.native.digest(Path(m.__file__))
                     for m in (dependency, shared, safety, shared.native, shared.dogfood)}))
    invoke = shared.justrust_runner(client, cargo, runner=safety.timed_runner(runner or shared.native.run))
    try:
        fixture = shared.private_directory(root / 'fixture')
        for path in ('src', 'dep', 'dep/src'):
            (fixture / path).mkdir(mode=0o700)
        (fixture / 'Cargo.toml').write_text(MANIFEST)
        (fixture / 'dep/Cargo.toml').write_text(DEP_MANIFEST)
        (fixture / 'dep/src/lib.rs').write_bytes(MACRO)
        # Immutable original inputs are separate from the mutable Cargo fixture.
        originals = shared.private_directory(root / 'originals')
        (originals / 'Cargo.toml').write_text(MANIFEST)
        (originals / 'macro-Cargo.toml').write_text(DEP_MANIFEST)
        (originals / 'macro.rs').write_bytes(MACRO)
        workload = stages(uuid.uuid4().hex)
        report['workload'] = [{**{k: v for k, v in s.items() if k != 'root'},
                               'root_b64': shared.native.encoded(s['root'])} for s in workload]
        for index, stage in enumerate(workload):
            (originals / f'{index:02d}-root.rs').write_bytes(stage['root'])
        (originals / 'workload.json').write_text(json.dumps(report['workload'], indent=2) + '\n')
        for path in originals.iterdir():
            path.chmod(0o400)
        originals.chmod(0o500)
        for mode in ('off', 'on'):
            arm = root / ('000-' + mode)
            try:
                report['arms'][mode] = capture_arm(compiler, cargo, fixture, arm, mode, workload,
                                                   timeout, invoke, require_resident_root)
            finally:
                receipt = arm / 'receipt.json'
                if receipt.exists():
                    report['arms'][mode] = json.loads(receipt.read_text())
        for off, on in zip(report['arms']['off']['observations'], report['arms']['on']['observations']):
            if off['semantics'] != on['semantics']:
                raise AssertionError('Exact off/on diagnostics and runtime disagree')
        if attestation != safety.attest(compiler, cargo, client, expected):
            raise AssertionError('Frozen candidate changed during acceptance')
        report['verdict'] = 'passed'
    except BaseException as error:
        report.update(verdict='failed', error=f'{type(error).__name__}: {error}')
        raise
    finally:
        (root / 'receipt.json').write_text(json.dumps(report, indent=2) + '\n')
    return report


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    for name in ('compiler', 'cargo', 'output', 'justrust'):
        parser.add_argument('--' + name, required=True, type=Path)
    for name in ('compiler', 'driver', 'cargo', 'client'):
        parser.add_argument('--expected-' + name + '-sha256')
    for name in ('root-native-frozen', 'idle-authorized', 'require-resident-root'):
        parser.add_argument('--' + name, action='store_true')
    parser.add_argument('--timeout', type=float, default=180)
    report = capture(**vars(parser.parse_args()))
    print(json.dumps(dict(verdict=report['verdict'], claim=report['claim'])))


if __name__ == '__main__':
    main()
