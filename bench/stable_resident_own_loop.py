#!/usr/bin/env python3
"""Focused real justrust checkout edit loop on an authenticated hosted guest.

No guest launcher, candidate build, installation, or general speed claim. The
coordinator must freeze candidates and authorize an idle checkout before capture.
"""
import argparse
import base64
import hashlib
import json
import math
import os
from pathlib import Path
import statistics
import uuid

import stable_resident_cargo as shared
import stable_resident_cargo_suite as safety
import stable_resident_cargo_dependency_suite as rss_helpers
from stable_resident_cargo_dependency_suite import observed_runner

DIAGNOSTIC_SCHEMA = 'stable-resident-own-loop-diagnostic/v1'
DIAGNOSTIC_CLAIM = ('Resident-on-only correctness diagnostic. '
                    'Not performance evidence or a speedup, comparison, or default-promotion claim.')
SCHEMA = 'stable-resident-own-loop/v1'
CLAIM = ('Focused actual-client justrust-source check/build lib edit loop only. '
         'No general speedup, default promotion, peak RSS, or retained-heap claim.')
STAGES = ('initial', 'cargo-fresh', 'body-edit-1', 'body-edit-2', 'restored-source')
MEASURED = STAGES[2:]
BODY_ANCHOR = b'        _ => agent || dev_wrapper,\n'
STRICT_ENV = dict(JUSTRUST_RUSTC_RESIDENT_FRONTEND='0',
                  JUSTRUST_CARGO_RESIDENT_EXTERNAL='1', JUSTRUST_CARGO_RESIDENT_OWNED='1',
                  JUSTRUST_CARGO_RESIDENT_MACROS='1', JUSTRUST_CARGO_RESIDENT_DECODERS='0')


def digest_bytes(data):
    return hashlib.sha256(data).hexdigest()


def validate_macro_spins(value, macro_shm):
    if type(value) is not int or not 0 <= value <= 256:
        raise ValueError('Macro spins must be an integer in 0..256')
    if value and macro_shm is not True:
        raise ValueError('Nonzero macro spins require macro SHM')
    return value


def parse_macro_spins(value):
    try:
        parsed = int(value)
        if str(parsed) != value or not 0 <= parsed <= 256:
            raise ValueError
    except ValueError:
        raise argparse.ArgumentTypeError('Macro spins must be canonical decimal 0..256') from None
    return parsed


def stages(original, nonce, edit_mode='body'):
    if edit_mode == 'body':
        if original.count(BODY_ANCHOR) != 1:
            raise ValueError('Actual justrust body edit anchor changed')
        first = original.replace(BODY_ANCHOR, b'        _ => agent && dev_wrapper,\n')
        second = original.replace(BODY_ANCHOR, b'        _ => !agent || dev_wrapper,\n')
    elif edit_mode == 'comment':
        first = original + f'\n// jr-own-loop {nonce} edit 1\n'.encode()
        second = original + f'\n// jr-own-loop {nonce} edit 2\n'.encode()
    else:
        raise ValueError('Unknown edit mode')
    return [('initial', original), ('cargo-fresh', original),
            (edit_mode + '-edit-1', first), (edit_mode + '-edit-2', second),
            ('restored-source', original)]


def semantics(record, fresh):
    if record.get('timed_out') or record['status'] != 0:
        raise AssertionError('Ordinary client check/build failed')
    messages = []
    for line in base64.b64decode(record['stdout_b64'], validate=True).decode().splitlines():
        if line.startswith('{'):
            messages.append(json.loads(line))
    # Preserve stderr as exact bytes too, even if it contains non-UTF8 diagnostics.
    base64.b64decode(record['stderr_b64'], validate=True)
    artifacts = [m for m in messages if m.get('reason') == 'compiler-artifact'
                 and m.get('target', {}).get('name') == 'justrust'
                 and 'lib' in m.get('target', {}).get('kind', [])]
    if len(artifacts) != 1 or type(artifacts[0].get('fresh')) is not bool or (fresh is not None and artifacts[0]['fresh'] is not fresh):
        raise AssertionError('Missing root justrust lib artifact or wrong Cargo freshness')
    if not any(m.get('reason') == 'build-finished' and m.get('success') is True for m in messages):
        raise AssertionError('Missing successful ordinary Cargo build-finished')
    return dict(status=0, root_crate='justrust', fresh=artifacts[0]['fresh'])


def validate_residency(rows, mode):
    identity = None
    initial_safety_retirement = None
    for row_index, row in enumerate(rows):
        if mode != 'off' and any('fallback' in str(e.get('event', '')).lower()
                                 or e.get('fallback')
                                 or e.get('result', {}).get('fallback') for e in row['trace']):
            raise AssertionError('Resident invocation used fallback')
        if mode != 'off' and row_index != 0 and any(
                'retir' in str(e.get('event', '')).lower() for e in row['trace']):
            raise AssertionError('Unexpected worker retirement trace')
        complete = [e for e in row['trace'] if e.get('event') == 'complete']
        if mode == 'off' or row['semantics']['fresh']:
            if complete:
                raise AssertionError('Off/fresh invocation unexpectedly compiled through resident RPC')
            continue
        root = [e for e in complete if e.get('crate_name') == 'justrust']
        if len(root) != 1:
            raise AssertionError('Require exactly one root crate_name justrust completion')
        event = root[0]
        result = event.get('result', {})
        pid, key = result.get('pid'), event.get('key')
        if type(pid) is not int or pid <= 0 or not isinstance(key, str) or not key:
            raise AssertionError('Missing root worker PID/key')
        if (type(result.get('id')) is not int or type(result.get('status')) is not int
                or result['status'] != 0 or type(event.get('elapsed_ns')) is not int
                or event['elapsed_ns'] < 0):
            raise AssertionError('Invalid root worker response')
        current = (key, pid)
        changed, retired = result.get('external_inputs_changed'), result.get('retire')
        safety_retirement = (row_index == 0 and row['name'] == 'initial'
                             and changed is True and retired is True)
        if (any(value is not None and value is not False for value in (changed, retired))
                and not safety_retirement):
            raise AssertionError('Unexpected root worker input change/retirement')
        if safety_retirement:
            initial_safety_retirement = dict(root_worker_key=key, root_worker_pid=pid,
                                             status=0, external_inputs_changed=True, retire=True)
        if identity is not None and current != identity:
            raise AssertionError('Root worker changed across repeated source edits')
        if not safety_retirement:
            identity = current
        samples = row['cargo']['resident_worker_rss']
        if not any(s.get('pid') == pid for s in samples):
            raise AssertionError('Missing root worker RSS observation')
    evidence = dict(root_worker_key=identity[0], root_worker_pid=identity[1]) if identity else {}
    if initial_safety_retirement is not None:
        evidence['initial_safety_retirement'] = initial_safety_retirement
    return evidence


def selected_commands(commands):
    if (not isinstance(commands, (tuple, list)) or not commands
            or any(command not in ('check', 'build') for command in commands)
            or len(set(commands)) != len(commands)):
        raise ValueError('Commands must be a nonempty unique selection of check/build')
    return tuple(commands)


def capture(compiler, cargo, output, justrust, repetitions=2, timeout=180, *,
            checkout=None, edit_mode='body', commands=('check', 'build'), root_native_frozen=False, idle_authorized=False,
            expected_compiler_sha256=None, expected_driver_sha256=None,
            expected_cargo_sha256=None, expected_client_sha256=None, runner=None,
            diagnostic_on_only=False, macro_shm=False, macro_spins=0):
    commands = selected_commands(commands)
    if type(macro_shm) is not bool:
        raise ValueError('Macro SHM mode must be boolean')
    validate_macro_spins(macro_spins, macro_shm)
    if root_native_frozen is not True or idle_authorized is not True:
        raise ValueError('Root native freeze and idle authorization required')
    if type(diagnostic_on_only) is not bool:
        raise ValueError('Diagnostic on-only flag must be boolean')
    if diagnostic_on_only:
        if type(repetitions) is not int or repetitions < 1:
            raise ValueError('Positive diagnostic repetition count required')
    elif type(repetitions) is not int or repetitions < 2 or repetitions % 2:
        raise ValueError('Even repetition count >= 2 required')
    if type(timeout) not in (float, int) or not math.isfinite(timeout) or timeout <= 0:
        raise ValueError('Finite positive timeout required')
    if justrust is None:
        raise ValueError('Actual justrust client required')
    shared.check_socket_path(output, repetitions)
    compiler, cargo, client = [Path(p).resolve(strict=True) for p in (compiler, cargo, justrust)]
    checkout = Path(checkout or Path(__file__).resolve().parents[1]).resolve(strict=True)
    # This is the real shipping workspace, not a generated wrapper or copied fixture.
    manifest = (checkout / 'Cargo.toml').read_bytes()
    import tomllib
    if tomllib.loads(manifest.decode()).get('package', {}).get('name') != 'justrust':
        raise ValueError('Require actual justrust package checkout')
    source = checkout / 'src/lib.rs'
    if source.is_symlink() or not source.is_file():
        raise ValueError('Require non-symlink real root lib source')
    original = source.read_bytes()
    stages(original, 'preflight', edit_mode)
    original_stat = source.stat()
    expected = dict(compiler_sha256=expected_compiler_sha256, driver_sha256=expected_driver_sha256,
                    cargo_sha256=expected_cargo_sha256, client_sha256=expected_client_sha256)
    attested = safety.attest(compiler, cargo, client, expected)
    root = shared.private_directory(output)
    (root / 'original-lib.rs').write_bytes(original)
    report = dict(schema=DIAGNOSTIC_SCHEMA if diagnostic_on_only else SCHEMA,
                  claim=DIAGNOSTIC_CLAIM if diagnostic_on_only else CLAIM, verdict='incomplete',
                  provenance=dict(attestation=attested, expected=expected, checkout=str(checkout),
                                  source_path=str(source), original_b64=shared.native.encoded(original),
                                  original_sha256=digest_bytes(original), manifest_b64=shared.native.encoded(manifest),
                                  host=safety.host(), repetitions=repetitions, timeout=timeout, edit_mode=edit_mode,
                                  commands=list(commands), macro_shm=macro_shm, macro_spins=macro_spins,
                                  root_native_frozen=True, idle_authorized=True,
                                  harness_sha256=shared.native.digest(Path(__file__)),
                                  helper_sha256={Path(module.__file__).name: shared.native.digest(Path(module.__file__))
                                                 for module in (shared, safety, shared.native, rss_helpers)}), repetitions=[])
    invoke = shared.justrust_runner(client, cargo, runner=observed_runner(runner or shared.native.run))
    last_written = original
    try:
        for index in range(repetitions):
            nonce = uuid.uuid4().hex
            order = ['on'] if diagnostic_on_only else (['off', 'on'] if index % 2 == 0 else ['on', 'off'])
            pair = dict(order=order, nonce=nonce, arms={})
            report['repetitions'].append(pair)
            for mode in order:
                arm_root = shared.private_directory(root / f'{index:03d}-{mode}')
                workers = shared.private_directory(arm_root / 'workers')
                trace = arm_root / 'trace.jsonl'
                env = shared.environment(compiler, checkout, mode, workers, trace)
                env.update(STRICT_ENV)
                # Always override inherited transport settings in both arms.
                env['JUSTRUST_CARGO_RESIDENT_MACRO_SHM'] = '1' if macro_shm else '0'
                env['JUSTRUST_CARGO_RESIDENT_MACRO_SPINS'] = str(macro_spins)
                # Both commands use the same per-arm target, but check and build
                # have distinct native worker identities and validation groups.
                env['CARGO_TARGET_DIR'] = str(arm_root / 'target')
                arm = pair['arms'][mode] = dict(observations=[])
                for command in commands:
                    rows = []
                    for stage_index, (name, data) in enumerate(stages(original, nonce, edit_mode)):
                        if source.read_bytes() != last_written:
                            raise RuntimeError('Concurrent source edit detected')
                        if name != 'cargo-fresh':
                            source.write_bytes(data)
                            last_written = data
                        before = len(shared.trace_events(trace))
                        row = dict(name=name, command=command, source_sha256=digest_bytes(data))
                        arm['observations'].append(row)
                        rows.append(row)
                        (arm_root / f'{command}-{stage_index:02d}-source.rs').write_bytes(data)
                        row['cargo'] = invoke([cargo, command, '--lib', '--message-format=json'],
                                              timeout, cwd=checkout, env=env)
                        row['trace'] = shared.trace_events(trace)[before:]
                        row['semantics'] = semantics(row['cargo'], None if name == 'initial' else name == 'cargo-fresh')
                    arm.setdefault('residency', {})[command] = validate_residency(rows, mode)
        if safety.attest(compiler, cargo, client, expected) != attested:
            raise AssertionError('Candidate identities changed during capture')
        if (checkout / 'Cargo.toml').read_bytes() != manifest:
            raise AssertionError('Manifest changed during capture')
        if safety.host() != report['provenance']['host']:
            raise AssertionError('Host/boot/affinity changed during capture')
        report['verdict'] = 'diagnostic-passed' if diagnostic_on_only else 'passed'
        if diagnostic_on_only:
            _validate(report, diagnostic=True)
        else:
            report['summary'] = validate(report)
    except BaseException as error:
        report.update(verdict='failed', error=f'{type(error).__name__}: {error}')
        raise
    finally:
        # Never reset git or discard a concurrent editor's work. Preserve unexpected
        # bytes first, then restore the backup owned by this exclusive capture.
        try:
            current = source.read_bytes()
            if current != last_written:
                (root / 'concurrent-lib.rs').write_bytes(current)
                report.update(verdict='failed', restoration_warning='Concurrent bytes preserved in concurrent-lib.rs')
            source.write_bytes((root / 'original-lib.rs').read_bytes())
            os.chmod(source, original_stat.st_mode & 0o7777)
            os.utime(source, ns=(original_stat.st_atime_ns, original_stat.st_mtime_ns))
            report['restoration'] = dict(restored=source.read_bytes() == original,
                                        source_sha256=digest_bytes(source.read_bytes()))
        finally:
            (root / 'receipt.json').write_text(json.dumps(report, indent=2) + '\n')
    return report


def validate(report):
    # Diagnostic receipts must never enter comparisons or performance summaries.
    return _validate(report, diagnostic=False)


def _validate(report, *, diagnostic):
    schema = DIAGNOSTIC_SCHEMA if diagnostic else SCHEMA
    verdict = 'diagnostic-passed' if diagnostic else 'passed'
    if report.get('schema') != schema or report.get('verdict') != verdict:
        raise ValueError('Not a passed own-source loop receipt')
    provenance = report['provenance']
    if type(provenance.get('macro_shm')) is not bool:
        raise ValueError('Missing or invalid macro SHM provenance')
    validate_macro_spins(provenance.get('macro_spins'), provenance['macro_shm'])
    original = base64.b64decode(provenance['original_b64'], validate=True)
    if digest_bytes(original) != provenance['original_sha256']:
        raise ValueError('Original source mismatch')
    for name in ('compiler', 'driver', 'cargo', 'client'):
        if provenance['attestation'][name]['sha256'] != provenance['expected'][name + '_sha256']:
            raise ValueError('Component attestation mismatch')
    count = provenance['repetitions']
    if (type(count) is not int or count < (1 if diagnostic else 2)
            or (not diagnostic and count % 2) or len(report['repetitions']) != count):
        raise ValueError('Missing counterbalanced repetitions')
    if provenance.get('root_native_frozen') is not True or provenance.get('idle_authorized') is not True:
        raise ValueError('Missing root execution authorization')
    commands = selected_commands(provenance.get('commands', ('check', 'build')))
    measured = (provenance['edit_mode'] + '-edit-1', provenance['edit_mode'] + '-edit-2', 'restored-source')
    samples = {command: {stage: {mode: [] for mode in ('off', 'on')}
                         for stage in measured} for command in commands}
    environment_reference = None
    for index, pair in enumerate(report['repetitions']):
        order = ['on'] if diagnostic else (['off', 'on'] if index % 2 == 0 else ['on', 'off'])
        if pair['order'] != order or set(pair['arms']) != set(order):
            raise ValueError('Wrong counterbalanced order')
        expected_stages = stages(original, pair['nonce'], provenance['edit_mode'])
        for mode in order:
            observations = pair['arms'][mode]['observations']
            if len(observations) != len(commands) * len(STAGES):
                raise ValueError('Missing selected command stages')
            for command_index, command in enumerate(commands):
                rows = observations[command_index * len(STAGES):(command_index + 1) * len(STAGES)]
                validate_residency(rows, mode)
                for row, (name, data) in zip(rows, expected_stages):
                    if (row['name'], row['command'], row['source_sha256']) != (name, command, digest_bytes(data)):
                        raise ValueError('Workload mismatch')
                    record = row['cargo']
                    if row['semantics'] != semantics(record, None if name == 'initial' else name == 'cargo-fresh'):
                        raise ValueError('Semantic evidence mismatch')
                    invocation = record['invocation']
                    wanted = [provenance['attestation']['client']['path'], 'cargo', command,
                              '--lib', '--message-format=json']
                    env = invocation['environment']
                    if (invocation['command'] != wanted or invocation['cwd'] != provenance['checkout']
                            or invocation['timeout'] != provenance['timeout']):
                        raise ValueError('Actual client command mismatch')
                    if (env['RUSTC'] != provenance['attestation']['compiler']['path']
                            or env['JUSTRUST_REAL_CARGO'] != provenance['attestation']['cargo']['path']
                            or env['JUSTRUST_CARGO_RESIDENT'] != ('serial' if mode == 'on' else 'off')
                            or env['RUSTFLAGS'] != shared.FLAGS
                            or env.get('JUSTRUST_CARGO_RESIDENT_MACRO_SHM') != ('1' if provenance['macro_shm'] else '0')
                            or env.get('JUSTRUST_CARGO_RESIDENT_MACRO_SPINS') != str(provenance['macro_spins'])
                            or any(env.get(key) != value for key, value in STRICT_ENV.items())
                            or safety.safe_environment(env) != env):
                        raise ValueError('Candidate environment mismatch')
                    normalized_env = dict(env)
                    for key in ('CARGO_TARGET_DIR', 'JUSTRUST_CARGO_RESIDENT_DIR', 'JUSTRUST_CARGO_RESIDENT_TRACE'):
                        normalized_env.pop(key)
                    normalized_env.pop('JUSTRUST_CARGO_RESIDENT')
                    if environment_reference is None:
                        environment_reference = normalized_env
                    if environment_reference != normalized_env:
                        raise ValueError('Effective paired environment mismatch')
                    worker_pids = sorted({event.get('result', {}).get('pid') for observed in observations[:observations.index(row) + 1]
                                          for event in observed['trace'] if event.get('event') == 'complete'
                                          and type(event.get('result', {}).get('pid')) is int})
                    rss_rows = record['resident_worker_rss']
                    if [sample.get('pid') for sample in rss_rows] != worker_pids:
                        raise ValueError('RSS samples do not cover observed worker generations')
                    for sample in rss_rows:
                        rss_helpers.validate_worker_cpu(sample)
                        if (type(sample.get('available')) is not bool or sample.get('source') != 'local-/proc'
                                or sample.get('kind') != 'post-invocation-live-rss'):
                            raise ValueError('Invalid worker RSS sample provenance')
                        if sample['available'] and (type(sample.get('rss_bytes')) is not int or sample['rss_bytes'] < 0
                                                    or type(sample.get('starttime_ticks')) is not int):
                            raise ValueError('Invalid worker RSS observation')
                    wall = record['invocation_wall_seconds']
                    if type(wall) not in (int, float) or not math.isfinite(wall) or wall <= 0:
                        raise ValueError('Invalid actual client wall time')
                    if name in measured:
                        samples[command][name][mode].append(wall)
    if diagnostic:
        return None
    return {command: {name: dict(samples=arms,
                                medians={mode: statistics.median(values) for mode, values in arms.items()},
                                paired_off_over_on=[off / on for off, on in zip(arms['off'], arms['on'])])
                      for name, arms in commands.items()} for command, commands in samples.items()}


def compare(before, after, floor=10, *, expected_before_compiler_sha256=None,
            expected_after_compiler_sha256=None, expected_before_driver_sha256=None,
            expected_after_driver_sha256=None, expected_before_macro_shm=None,
            expected_after_macro_shm=None, expected_before_macro_spins=None,
            expected_after_macro_spins=None):
    """Compare focused receipts, with explicit candidate and transport opt-ins.

    All four expected hashes are required to authorize different compiler or
    driver candidates. Both exact boolean transport expectations are required
    to authorize a macro RPC transport change, independently of those hashes.
    Both exact integer spin expectations are required for spin count changes.
    Workload, host, client, Cargo and every other effective environment setting
    remain identical. Each receipt must independently validate. This remains
    focused real-source evidence, not a broad performance claim.
    """
    if type(floor) not in (int, float) or not math.isfinite(floor) or floor < 0:
        raise ValueError('Finite nonnegative floor required')
    left, right = validate(before), validate(after)
    expected = ((expected_before_compiler_sha256, expected_before_driver_sha256),
                (expected_after_compiler_sha256, expected_after_driver_sha256))
    explicit = any(value is not None for pair in expected for value in pair)
    transports = (expected_before_macro_shm, expected_after_macro_shm)
    explicit_transport = any(value is not None for value in transports)
    spin_counts = (expected_before_macro_spins, expected_after_macro_spins)
    explicit_spins = any(value is not None for value in spin_counts)
    normalized = []
    for report, hashes, transport, spins in zip((before, after), expected, transports, spin_counts):
        provenance = json.loads(json.dumps(report['provenance']))
        if explicit_spins:
            validate_macro_spins(spins, provenance['macro_shm'])
            if provenance['macro_spins'] != spins:
                raise ValueError('Explicit before/after macro spins mismatch')
            provenance.pop('macro_spins')
        if explicit_transport:
            if type(transport) is not bool or provenance['macro_shm'] is not transport:
                raise ValueError('Explicit before/after macro SHM mode mismatch')
            provenance.pop('macro_shm')
        if explicit:
            for name, wanted in zip(('compiler', 'driver'), hashes):
                if (not isinstance(wanted, str) or len(wanted) != 64
                        or any(c not in '0123456789abcdef' for c in wanted)
                        or provenance['attestation'][name]['sha256'] != wanted
                        or provenance['expected'][name + '_sha256'] != wanted):
                    raise ValueError('Explicit before/after compiler and driver hash mismatch')
                provenance['attestation'].pop(name)
                provenance['expected'].pop(name + '_sha256')
        normalized.append(provenance)
    if normalized[0] != normalized[1]:
        raise ValueError('Provenance mismatch outside authorized compiler/driver change')
    for first_pair, second_pair in zip(before['repetitions'], after['repetitions']):
        for mode in ('off', 'on'):
            for first, second in zip(first_pair['arms'][mode]['observations'], second_pair['arms'][mode]['observations']):
                if any(first[key] != second[key] for key in ('name', 'command', 'source_sha256')):
                    raise ValueError('Compared source workload mismatch')
                environments = []
                for row in (first, second):
                    env = dict(row['cargo']['invocation']['environment'])
                    for key in ('CARGO_TARGET_DIR', 'JUSTRUST_CARGO_RESIDENT_DIR', 'JUSTRUST_CARGO_RESIDENT_TRACE'):
                        env.pop(key)
                    if explicit:
                        env.pop('RUSTC')
                    if explicit_transport:
                        env.pop('JUSTRUST_CARGO_RESIDENT_MACRO_SHM')
                    if explicit_spins:
                        env.pop('JUSTRUST_CARGO_RESIDENT_MACRO_SPINS')
                    environments.append(env)
                if environments[0] != environments[1]:
                    raise ValueError('Compared effective environment mismatch')
    return {command: {name: {mode: dict(
                before=left[command][name]['medians'][mode],
                after=right[command][name]['medians'][mode],
                change_percent=100 * (right[command][name]['medians'][mode]
                                      / left[command][name]['medians'][mode] - 1),
                regression=right[command][name]['medians'][mode]
                           > left[command][name]['medians'][mode] * (1 + floor / 100))
            for mode in ('off', 'on')} for name in left[command]} for command in left}


def console_summary(report, output):
    """Keep raw source, environments and command streams only in the receipt."""
    return dict(schema=report['schema'], verdict=report['verdict'], claim=report['claim'],
                receipt=str(Path(output).absolute() / 'receipt.json'),
                summary={} if report.get('schema') == DIAGNOSTIC_SCHEMA else report.get('summary', {}),
                restoration=report.get('restoration', {}))


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    for name in ('compiler', 'cargo', 'justrust', 'output'):
        parser.add_argument('--' + name, type=Path, required=True)
    for name in ('compiler', 'driver', 'cargo', 'client'):
        parser.add_argument('--expected-' + name + '-sha256', required=True)
    parser.add_argument('--checkout', type=Path)
    parser.add_argument('--commands', nargs='+', choices=('check', 'build'), default=['check', 'build'])
    parser.add_argument('--edit-mode', choices=('body', 'comment'), default='body')
    parser.add_argument('--macro-shm', action='store_true',
                        help='Explicitly use shared-memory macro RPC instead of Unix sockets in both arms')
    parser.add_argument('--macro-spins', type=parse_macro_spins, default=0,
                        help='Canonical decimal spin count 0..256, nonzero requires --macro-shm')
    parser.add_argument('--diagnostic-on-only', action='store_true',
                        help='Run only resident-on as a correctness diagnostic, never performance evidence')
    parser.add_argument('--repetitions', type=int, default=2)
    parser.add_argument('--timeout', type=float, default=180)
    parser.add_argument('--root-native-frozen', action='store_true', required=True)
    parser.add_argument('--idle-authorized', action='store_true', required=True)
    args = vars(parser.parse_args())
    print(json.dumps(console_summary(capture(**args), args['output']), indent=2))


if __name__ == '__main__':
    main()
