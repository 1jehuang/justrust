#!/usr/bin/env python3
"""Offline path-dependency fallback correctness, never external reuse or performance.

Only run after the coordinator freezes native candidates and authorizes idle use.
Root Cargo units carry --extern and remain unsupported by resident admission.
"""
import argparse
import base64
import hashlib
import json
import math
from pathlib import Path
import re
import uuid

import stable_resident_cargo as shared
import stable_resident_cargo_suite as safety

SCHEMA = 'stable-resident-cargo-dependency/v1'
ERROR = 'JR_CARGO_DELIBERATE_ERROR'
MANIFEST = '''[package]
name="dependency_fallback_root"
version="0.0.0"
edition="2024"
[dependencies]
fixture_dep={path="dep"}
[workspace]
'''
DEP_MANIFEST = '''[package]
name="fixture_dep"
version="0.0.0"
edition="2024"
'''


def stages(nonce):
    def dep(signature=False, constant=3, body=7):
        declaration = 'pub fn value(n: u32) -> u64' if signature else 'pub fn value() -> u32'
        expression = f'{body} + n as u64' if signature else str(body)
        return f'pub const VALUE: u64 = {constant};\n{declaration} {{ {expression} }}\n'.encode()

    def root(tag, signature=False, constant=3, body=7):
        call = 'fixture_dep::value(2)' if signature else 'fixture_dep::value() as u64'
        value = body + 2 if signature else body
        return f'''#[test]
fn dependency_nonce_probe() {{
    let value: u64 = {call};
    assert_eq!(value, {value});
    assert_eq!(fixture_dep::VALUE, {constant});
    eprintln!("JR_CARGO_NONCE={nonce}-{tag}:{{}}:{{}}", value, fixture_dep::VALUE);
}}
'''.encode(), f'{nonce}-{tag}:{value}:{constant}'

    rows = []
    specs = [('initial', False, 3, 7, None),
             ('root-only-edit', False, 3, 7, None),
             ('root-only-error', False, 3, 7, 'root'),
             ('root-only-recovery', False, 3, 7, None),
             ('dependency-signature', True, 3, 7, None),
             ('dependency-const', True, 11, 7, None),
             ('dependency-body', True, 11, 19, None),
             ('dependency-error', True, 11, 19, 'dependency'),
             ('dependency-recovery', True, 11, 19, None),
             ('root-only-final', True, 11, 19, None)]
    for name, signature, constant, body, error in specs:
        source = dep(signature, constant, body)
        lib, marker = root(name, signature, constant, body)
        if error:
            deliberate = f'\ncompile_error!("{ERROR}");\n'.encode()
            if error == 'root':
                lib += deliberate
            else:
                source += deliberate
        rows.append(dict(name=name, dependency=source, root=lib,
                         nonce=None if error else marker, expectation='error' if error else 'pass',
                         root_only=name.startswith('root-only')))
    return rows


def messages(record):
    stdout = base64.b64decode(record['stdout_b64']).decode()
    stderr = base64.b64decode(record['stderr_b64']).decode()
    cargo = [json.loads(line) for line in stdout.splitlines() if line.startswith('{')]
    return stdout, stderr, cargo


def semantics(record, stage):
    if record.get('timed_out'):
        raise AssertionError('Cargo timed out')
    stdout, stderr, cargo = messages(record)
    diagnostics = [m['message'] for m in cargo if m.get('reason') == 'compiler-message']
    markers = re.findall(r'^JR_CARGO_NONCE=(.*)$', stderr, re.MULTILINE)
    if stage['expectation'] == 'error':
        if (record['status'] != 101 or markers or ERROR not in stdout + stderr or
                not any(m.get('level') == 'error' for m in diagnostics)):
            raise AssertionError('Expected exact deliberate diagnostic without stale runtime')
        if re.search(r'^test dependency_nonce_probe \.\.\.', stdout, re.MULTILINE):
            raise AssertionError('Compile failure executed stale root test')
    else:
        tests = re.findall(r'^test (\S+) \.\.\. (ok|FAILED|ignored)(?: <\d+(?:\.\d+)?s>)?$', stdout, re.MULTILINE)
        if (record['status'] != 0 or markers != [stage['nonce']] or
                tests != [('dependency_nonce_probe', 'ok')] or
                'test result: ok. 1 passed; 0 failed;' not in stdout):
            raise AssertionError('Dependency runtime/nonce/test mismatch')
    return dict(status=record['status'], diagnostics=diagnostics, markers=markers)


def metadata(record):
    """Observe Cargo artifact identity, never require byte equality across arms."""
    result = []
    for message in messages(record)[2]:
        if message.get('reason') != 'compiler-artifact':
            continue
        artifacts = []
        for name in message.get('filenames', []):
            path = Path(name)
            artifacts.append(dict(path=name, exists=path.is_file(),
                                  bytes=path.stat().st_size if path.is_file() else None,
                                  sha256=shared.native.digest(path) if path.is_file() else None))
        result.append(dict(package_id=message.get('package_id'), target=message.get('target'),
                           fresh=message.get('fresh'), artifacts=artifacts))
    return result


def validate_artifacts(stage, observations):
    if not stage['root_only']:
        return
    dependency = [row for row in observations if row.get('target', {}).get('name') == 'fixture_dep']
    if not dependency or any(row['fresh'] is not True for row in dependency):
        raise AssertionError('Root-only edit must leave path dependency Cargo-fresh')


def validate_trace(stage, events, mode):
    complete = [event for event in events if event.get('event') == 'complete']
    if complete and (mode == 'off' or stage['root_only']):
        raise AssertionError('Root --extern fallback or normal-off unexpectedly completed resident RPC')
    # On dependency edits a std-only dependency may use the worker. This is NOT
    # evidence of root admission, retained external typed state, or external reuse.
    return dict(complete_events=len(complete), interpretation=(
        'std-only dependency compilation permitted, not external reuse' if complete
        else 'no resident complete events'))


def capture_arm(compiler, cargo, fixture, arm, mode, workload, timeout, runner):
    arm.mkdir(mode=0o700)
    workers = shared.private_directory(arm / 'workers')
    trace = arm / 'trace.jsonl'
    env = safety.safe_environment(shared.environment(compiler, fixture, mode, workers, trace))
    report = dict(mode=mode, verdict='incomplete', observations=[])
    try:
        previous_dependency = None
        for index, stage in enumerate(workload):
            source = fixture / 'dep/src/lib.rs'
            # Never rewrite an unchanged dependency. This is crucial for proving
            # that root-only stages cannot disguise dependency worker events.
            if stage['dependency'] != previous_dependency:
                source.write_bytes(stage['dependency'])
            previous_dependency = stage['dependency']
            (fixture / 'src/lib.rs').write_bytes(stage['root'])
            (arm / f'{index:02d}-dependency.rs').write_bytes(stage['dependency'])
            (arm / f'{index:02d}-root.rs').write_bytes(stage['root'])
            before = len(shared.trace_events(trace))
            row = dict(name=stage['name'], expectation=stage['expectation'], nonce=stage['nonce'],
                       root_only=stage['root_only'], dependency_path=str(source),
                       dependency_sha256=hashlib.sha256(stage['dependency']).hexdigest(),
                       root_sha256=hashlib.sha256(stage['root']).hexdigest())
            report['observations'].append(row)
            try:
                row['cargo'] = runner([cargo, 'test', '--offline', '--lib', '--message-format=json',
                                       '--', '--nocapture', '--test-threads=1'], timeout, cwd=fixture, env=env)
            finally:
                row['trace'] = shared.trace_events(trace)[before:]
            row['metadata'] = metadata(row['cargo'])
            row['semantics'] = semantics(row['cargo'], stage)
            validate_artifacts(stage, row['metadata'])
            row['residency'] = validate_trace(stage, row['trace'], mode)
        report['verdict'] = 'passed'
    except BaseException as error:
        report.update(verdict='failed', error=f'{type(error).__name__}: {error}')
        raise
    finally:
        (arm / 'receipt.json').write_text(json.dumps(report, indent=2) + '\n')
    return report


def capture(compiler, cargo, output, *, root_native_frozen=False, idle_authorized=False,
            timeout=180, justrust=None, expected_compiler_sha256=None,
            expected_driver_sha256=None, expected_cargo_sha256=None, expected_client_sha256=None,
            runner=None):
    if root_native_frozen is not True or idle_authorized is not True:
        raise ValueError('Root native freeze and idle authorization required before execution')
    if not math.isfinite(timeout) or timeout <= 0:
        raise ValueError('Finite positive timeout required')
    shared.check_socket_path(output, 1)
    compiler, cargo = (Path(p).resolve(strict=True) for p in (compiler, cargo))
    client = Path(justrust).resolve(strict=True) if justrust else None
    expected = dict(compiler_sha256=expected_compiler_sha256, driver_sha256=expected_driver_sha256,
                    cargo_sha256=expected_cargo_sha256, client_sha256=expected_client_sha256)
    attestation = safety.attest(compiler, cargo, client, expected)
    root = shared.private_directory(output)
    report = dict(schema=SCHEMA, verdict='incomplete', claim='Dependency fallback correctness only. '
                  'No retained external typed reuse or performance claim.', arms={},
                  provenance=dict(attestation=attestation, expected=expected,
                                  root_native_frozen=True, idle_authorized=True,
                                  manifest=MANIFEST, dependency_manifest=DEP_MANIFEST,
                                  timeout=timeout,
                                  helpers={Path(module.__file__).name: shared.native.digest(Path(module.__file__))
                                           for module in (shared, safety, shared.native, shared.dogfood)},
                                  harness_sha256=shared.native.digest(Path(__file__))))
    invoke = safety.timed_runner(runner or shared.native.run)
    if client:
        invoke = shared.justrust_runner(client, cargo, runner=invoke)
    try:
        fixture = shared.private_directory(root / 'fixture')
        for path in ('src', 'dep', 'dep/src'):
            (fixture / path).mkdir(mode=0o700)
        (fixture / 'Cargo.toml').write_text(MANIFEST)
        (fixture / 'dep/Cargo.toml').write_text(DEP_MANIFEST)
        workload = stages(uuid.uuid4().hex)
        report['workload'] = [{**{k: v for k, v in stage.items() if k not in ('root', 'dependency')},
                               'root_b64': shared.native.encoded(stage['root']),
                               'dependency_b64': shared.native.encoded(stage['dependency'])} for stage in workload]
        # Separate never-used target directories. Off is a fresh normal-process
        # diagnostic/runtime oracle, not a reference artifact or byte oracle.
        for mode in ('off', 'on'):
            arm = root / ('000-' + mode)
            try:
                report['arms'][mode] = capture_arm(compiler, cargo, fixture, arm,
                                                   mode, workload, timeout, invoke)
            finally:
                # Include failed-arm raw evidence in the top-level receipt too.
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
    for name in ('compiler', 'cargo', 'output'):
        parser.add_argument('--' + name, required=True, type=Path)
    parser.add_argument('--justrust', type=Path)
    for name in ('compiler', 'driver', 'cargo', 'client'):
        parser.add_argument('--expected-' + name + '-sha256')
    parser.add_argument('--root-native-frozen', action='store_true')
    parser.add_argument('--idle-authorized', action='store_true')
    parser.add_argument('--timeout', type=float, default=180)
    report = capture(**vars(parser.parse_args()))
    print(json.dumps(dict(verdict=report['verdict'], claim=report['claim'])))


if __name__ == '__main__':
    main()
