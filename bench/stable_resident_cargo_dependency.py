#!/usr/bin/env python3
"""Offline path-dependency correctness with optional required resident root admission.

Only run after the coordinator freezes native candidates and authorizes idle use.
Default mode checks conservative fallback. --require-resident-root instead requires
positive root RPC identity, same-worker edits/recovery, and dependency invalidation.
Neither mode establishes retained typed external state or a performance claim.
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

SCHEMA = 'stable-resident-cargo-dependency/v2'
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


def validate_dependency_transition(stage, observations, previous):
    if stage['name'] == 'dependency-error':
        return previous
    current = {}
    for row in observations:
        if row.get('target', {}).get('name') == 'fixture_dep':
            for artifact in row.get('artifacts', []):
                if Path(artifact['path']).suffix in ('.rlib', '.rmeta'):
                    if (artifact.get('exists') is not True or
                            not isinstance(artifact.get('sha256'), str) or
                            not re.fullmatch(r'[0-9a-f]{64}', artifact['sha256'])):
                        raise AssertionError('Missing dependency artifact content identity')
                    current[artifact['path']] = artifact['sha256']
    if not current:
        raise AssertionError('No dependency Rust artifact content identity observed')
    if previous is not None:
        if stage['root_only'] and current != previous:
            raise AssertionError('Root-only edit changed dependency artifact identity')
        if stage['name'] in ('dependency-signature', 'dependency-const', 'dependency-body'):
            common = current.keys() & previous.keys()
            if not any(current[path] != previous[path] for path in common):
                raise AssertionError('No same-path dependency artifact content replacement observed')
    return current


def validate_trace(stage, events, mode, require_resident_root=False):
    complete = [event for event in events if event.get('event') == 'complete']
    if mode == 'off' and complete:
        raise AssertionError('normal-off unexpectedly completed resident RPC')
    if require_resident_root and mode == 'on':
        # Attribute the root explicitly, never infer it from event ordering or
        # mistake an admitted dependency for an admitted --extern root unit.
        roots = [event for event in complete
                 if event.get('crate_name') == 'dependency_fallback_root']
        if stage['name'] == 'dependency-error':
            if roots:
                raise AssertionError('Dependency compile failure unexpectedly compiled the root')
            return dict(complete_events=len(complete), root=None,
                        interpretation='dependency error prevented root compilation')
        if not roots:
            raise AssertionError('Required resident root completion missing')
        if len(roots) != 1 or (stage['root_only'] and len(complete) != 1):
            raise AssertionError('Stage requires exactly one root completion')
        root = roots[0]
        result = root.get('result', {})
        key, pid, request = root.get('key'), result.get('pid'), result.get('id')
        expected_status = 1 if stage['expectation'] == 'error' else 0
        if (not isinstance(key, str) or not re.fullmatch(r'[0-9a-f]{64}', key) or
                type(pid) is not int or pid <= 0 or type(request) is not int or request < 0 or
                type(result.get('status')) is not int or result['status'] != expected_status):
            raise AssertionError('Malformed root resident identity or completion status')
        return dict(complete_events=len(complete), root=dict(key=key, pid=pid, request=request),
                    interpretation='root resident RPC, not retained external typed-state evidence')
    if complete and stage['root_only']:
        raise AssertionError('Root --extern fallback unexpectedly completed resident RPC')
    return dict(complete_events=len(complete), interpretation=(
        'std-only dependency compilation permitted, not external reuse' if complete
        else 'no resident complete events'))


def validate_root_transition(stage, residency, previous):
    """Require stable worker generations on root-only changes, new keys on dep changes."""
    current = residency.get('root')
    if current is None:
        return previous
    if previous is not None:
        if stage['root_only']:
            if (current['key'], current['pid']) != (previous['key'], previous['pid']):
                raise AssertionError('Root-only edit/error/recovery replaced the resident worker')
            # Request IDs belong to each short-lived Cargo client, not the
            # persistent worker. The same ID can legitimately recur across edits.
        elif stage['name'] in ('dependency-signature', 'dependency-const', 'dependency-body'):
            if current['key'] == previous['key']:
                raise AssertionError('Dependency content change did not invalidate root identity')
    return current


def validate_owned_batch(stage, events, previous):
    roots = [event for event in events if event.get('event') == 'complete'
             and event.get('crate_name') == 'dependency_fallback_root']
    if stage['name'] == 'dependency-error':
        if roots:
            raise AssertionError('Failed dependency executed owned root')
        return
    if len(roots) != 1:
        raise AssertionError('Owned root completion missing')
    owned = roots[0].get('result', {}).get('owned', {})
    if (owned.get('enabled') is not True or type(owned.get('revisions')) is not int
            or owned['revisions'] < 1 or type(owned.get('contexts')) is not int
            or owned['contexts'] < 1):
        raise AssertionError('Actual retained compiler batch telemetry missing')
    if (stage['root_only'] and previous is not None
            and owned['revisions'] <= previous.get('owned_revisions', 0)):
        raise AssertionError('Owned compiler revision did not advance')


def validate_input_watch(trace):
    for event in trace:
        result = event.get('result', {})
        if event.get('event') == 'complete' and result.get('external_inputs_verified'):
            if result.get('input_watch_enabled') is not True or result.get('input_watch_active') is not True:
                raise AssertionError('External worker eviction watch was not active')


def capture_arm(compiler, cargo, fixture, arm, mode, workload, timeout, runner,
                require_resident_root=False, owned_batch=False, evict_inputs=False):
    arm.mkdir(mode=0o700)
    workers = shared.private_directory(arm / 'workers')
    trace = arm / 'trace.jsonl'
    env = safety.safe_environment(shared.environment(compiler, fixture, mode, workers, trace))
    if require_resident_root:
        env['JUSTRUST_CARGO_RESIDENT_EXTERNAL'] = '1'
    if owned_batch:
        env['JUSTRUST_CARGO_RESIDENT_OWNED'] = '1'
        # The owned compiler's lifecycle is separate from the snapshot-only
        # frontend experiment. Compare the same disabled setting in both arms.
        env['JUSTRUST_RUSTC_RESIDENT_FRONTEND'] = '0'
    env['JUSTRUST_CARGO_RESIDENT_EVICT'] = '1' if evict_inputs else '0'
    report = dict(mode=mode, verdict='incomplete', observations=[])
    try:
        previous_dependency = None
        previous_root = None
        previous_artifacts = None
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
            if require_resident_root:
                previous_artifacts = validate_dependency_transition(stage, row['metadata'], previous_artifacts)
            row['residency'] = validate_trace(stage, row['trace'], mode, require_resident_root)
            if evict_inputs and mode == 'on':
                validate_input_watch(row['trace'])
            if owned_batch and mode == 'on':
                validate_owned_batch(stage, row['trace'], previous_root)
            if require_resident_root and mode == 'on':
                previous_root = validate_root_transition(stage, row['residency'], previous_root)
                if owned_batch and previous_root is not None:
                    roots = [event for event in row['trace'] if event.get('event') == 'complete'
                             and event.get('crate_name') == 'dependency_fallback_root']
                    if roots:
                        previous_root['owned_revisions'] = roots[0]['result']['owned']['revisions']
        report['verdict'] = 'passed'
    except BaseException as error:
        report.update(verdict='failed', error=f'{type(error).__name__}: {error}')
        raise
    finally:
        (arm / 'receipt.json').write_text(json.dumps(report, indent=2) + '\n')
    return report


def capture(compiler, cargo, output, *, root_native_frozen=False, idle_authorized=False,
            timeout=180, justrust=None, require_resident_root=False, expected_compiler_sha256=None,
            expected_driver_sha256=None, expected_cargo_sha256=None, expected_client_sha256=None,
            runner=None, owned_batch=False):
    if owned_batch and not require_resident_root:
        raise ValueError('Owned batch requires explicit resident root acceptance')
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
    report = dict(schema=SCHEMA, verdict='incomplete', claim=(
                  'Dependency resident root RPC correctness and identity invalidation only. '
                  if require_resident_root else 'Dependency fallback correctness only. ') +
                  'No retained external typed reuse or performance claim.', arms={},
                  provenance=dict(attestation=attestation, expected=expected,
                                  root_native_frozen=True, idle_authorized=True,
                                  manifest=MANIFEST, dependency_manifest=DEP_MANIFEST,
                                  timeout=timeout, require_resident_root=require_resident_root,
                                  owned_batch=owned_batch,
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
                                                   mode, workload, timeout, invoke,
                                                   require_resident_root=require_resident_root,
                                                   owned_batch=owned_batch)
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
    parser.add_argument('--owned-batch', action='store_true')
    parser.add_argument('--require-resident-root', action='store_true')
    parser.add_argument('--timeout', type=float, default=180)
    report = capture(**vars(parser.parse_args()))
    print(json.dumps(dict(verdict=report['verdict'], claim=report['claim'])))


if __name__ == '__main__':
    main()
