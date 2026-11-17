#!/usr/bin/env python3
"""Focused actual Cargo invocation timing. Never a general/default promotion gate."""
import argparse
import hashlib
import json
import math
import os
from pathlib import Path
import platform
import statistics
import time
import uuid

import stable_resident_cargo as acceptance

SCHEMA = 1
SCOPE = 'focused-stable-resident-actual-cargo-edit-loop'
MEASURED = ('body-edit', 'recovery', 'forced-same-source')
MANIFEST = '[package]\nname="resident_cargo_fixture"\nversion="0.0.0"\nedition="2024"\n[workspace]\n'


def host():
    return dict(hostname=platform.node(), kernel=platform.release(), machine=platform.machine(),
                boot_id=Path('/proc/sys/kernel/random/boot_id').read_text().strip(),
                cpu_identity=[line for line in Path('/proc/cpuinfo').read_text().splitlines()
                              if line.startswith(('vendor_id', 'model name', 'cpu family', 'model\t', 'stepping'))],
                affinity=sorted(os.sched_getaffinity(0)))


def timed_runner(runner, clock=time.perf_counter_ns):
    def invoke(command, timeout, cwd, env):
        start = clock()
        record = runner(command, timeout, cwd=cwd, env=env)
        elapsed = (clock() - start) / 1e9
        return dict(record, invocation_wall_seconds=elapsed,
                    invocation=dict(command=list(map(str, command)), cwd=str(cwd),
                                    timeout=timeout, environment=dict(env)))
    return invoke


def identity(path):
    return dict(path=str(path), sha256=acceptance.native.digest(path)) if path else None


def capture(compiler, cargo, output, repetitions=2, timeout=180, justrust=None):
    if type(repetitions) is not int or repetitions < 2 or repetitions % 2:
        raise ValueError('An even repetition count >= 2 is required for counterbalancing')
    if not math.isfinite(timeout) or timeout <= 0:
        raise ValueError('Finite positive timeout required')
    acceptance.check_socket_path(output, repetitions)
    compiler, cargo = (Path(p).resolve(strict=True) for p in (compiler, cargo))
    client = Path(justrust).resolve(strict=True) if justrust else None
    root = acceptance.private_directory(output)
    fixture = acceptance.private_directory(root / 'fixture')
    (fixture / 'src').mkdir(mode=0o700)
    (fixture / 'Cargo.toml').write_text(MANIFEST)
    original = acceptance.SOURCE.read_bytes()
    (root / 'original.rs').write_bytes(original)
    candidates = dict(compiler=acceptance.native.identity(compiler), cargo=identity(cargo), client=identity(client))
    report = dict(schema=SCHEMA, scope=SCOPE, verdict='incomplete',
                  claim='Focused fixture only. No general, fullgate, default, or published-toolchain claim.',
                  provenance=dict(candidates=candidates, host=host(), timeout=timeout,
                                  repetitions=repetitions, manifest=MANIFEST,
                                  source_b64=acceptance.native.encoded(original),
                                  harness_sha256=acceptance.native.digest(Path(__file__)),
                                  correctness_sha256=acceptance.native.digest(Path(acceptance.__file__)),
                                  environment=dict(os.environ)), repetitions=[])
    # Timing wraps the lowest invocation boundary, including the optional real client.
    runner = timed_runner(acceptance.native.run)
    if client:
        runner = acceptance.justrust_runner(client, cargo, runner=runner)
    try:
        for repetition in range(repetitions):
            nonce = uuid.uuid4().hex
            order = ['off', 'on'] if repetition % 2 == 0 else ['on', 'off']
            pair = dict(order=order, arms={})
            report['repetitions'].append(pair)
            for mode in order:
                # Each pair shares cwd and arguments. Each arm has a cold independent target.
                pair['arms'][mode] = acceptance.capture_mode(
                    compiler, cargo, fixture, root / f'{repetition:03d}-{mode}', mode,
                    original, nonce, timeout, runner=runner)
            # Never reuse an earlier repetition's Cargo target artifacts.
            # Distinct paths rather than deletion preserve all raw evidence.
            for mode in order:
                target = fixture / ('target-' + mode)
                if target.exists():
                    target.rename(root / f'{repetition:03d}-{mode}' / 'target-evidence')
        if candidates != dict(compiler=acceptance.native.identity(compiler), cargo=identity(cargo), client=identity(client)):
            raise AssertionError('Candidate changed during capture')
        if original != acceptance.SOURCE.read_bytes():
            raise AssertionError('Shipping source changed during capture')
        if report['provenance']['host'] != host():
            raise AssertionError('Host/boot/affinity changed during capture')
        report['verdict'] = 'passed'
        report['summary'] = validate(report)
    except BaseException as error:
        report.update(verdict='failed', error=f'{type(error).__name__}: {error}')
        raise
    finally:
        (root / 'receipt.json').write_text(json.dumps(report, indent=2) + '\n')
    return report


def validate(report):
    if report.get('schema') != SCHEMA or report.get('scope') != SCOPE or report.get('verdict') != 'passed':
        raise ValueError('Not a passed focused Cargo receipt')
    provenance = report['provenance']
    count = provenance['repetitions']
    if type(count) is not int or count < 2 or count % 2 or len(report['repetitions']) != count:
        raise ValueError('Missing counterbalanced repetitions')
    samples = {name: {mode: [] for mode in ('off', 'on')} for name in MEASURED}
    reference = None
    environment_reference = None
    for index, pair in enumerate(report['repetitions']):
        if pair['order'] != (['off', 'on'] if index % 2 == 0 else ['on', 'off']):
            raise ValueError('Wrong counterbalanced order')
        rows_by_mode = {}
        for mode in ('off', 'on'):
            arm = pair['arms'][mode]
            if arm['verdict'] != 'passed' or arm['failures']:
                raise ValueError('Failed correctness arm')
            rows = arm['observations']
            expected = acceptance.stages(acceptance.base64.b64decode(provenance['source_b64']), rows[0]['nonce'].removesuffix('-original'))
            if [row['name'] for row in rows] != [stage[0] for stage in expected]:
                raise ValueError('Workload stages mismatch')
            acceptance.validate_residency(rows, mode)
            for row, stage in zip(rows, expected):
                if (row['expectation'], row['nonce'], row['source_sha256'], row['wrapper_sha256']) != (
                        stage[4], stage[3], hashlib.sha256(stage[1]).hexdigest(), hashlib.sha256(stage[2]).hexdigest()):
                    raise ValueError('Workload bytes mismatch')
                if row['semantics'] != acceptance.semantics(row['cargo'], row['nonce'], row['expectation']):
                    raise ValueError('Semantic evidence mismatch')
            fallback = arm['wrapper_fallback']
            acceptance.semantics(fallback['cargo'], fallback['nonce'], 'pass')
            if any(event.get('event') == 'complete' for event in fallback['trace']):
                raise ValueError('Wrapper fallback used resident RPC')
            for row in [*rows, dict(cargo=fallback['cargo'])]:
                record = row['cargo']
                wall = record['invocation_wall_seconds']
                if type(wall) not in (float, int) or not math.isfinite(wall) or wall <= 0:
                    raise ValueError('Invalid actual invocation wall time')
                invocation = record['invocation']
                env = dict(invocation['environment'])
                cwd = Path(invocation['cwd'])
                candidates = provenance['candidates']
                expected_command = ([candidates['client']['path'], 'cargo'] if candidates['client']
                                    else [candidates['cargo']['path']]) + [
                                        'test', '--offline', '--lib', '--message-format=json',
                                        '--', '--nocapture', '--test-threads=1']
                if invocation['command'] != expected_command or invocation['timeout'] != provenance['timeout']:
                    raise ValueError('Candidate command/timeout mismatch')
                if env['RUSTC'] != candidates['compiler']['path'] or env['RUSTFLAGS'] != acceptance.FLAGS:
                    raise ValueError('Compiler/flags mismatch')
                if env['JUSTRUST_RUSTC_RESIDENT_FRONTEND'] != '1':
                    raise ValueError('Frontend admission mismatch')
                arm_root = cwd.parent / f'{index:03d}-{mode}'
                if (env['JUSTRUST_CARGO_RESIDENT_DIR'] != str(arm_root / 'workers') or
                        env['JUSTRUST_CARGO_RESIDENT_TRACE'] != str(arm_root / 'trace.jsonl')):
                    raise ValueError('Worker/trace isolation mismatch')
                if env['CARGO_TARGET_DIR'] != str(cwd / ('target-' + mode)):
                    raise ValueError('Independent target path mismatch')
                if env['JUSTRUST_CARGO_RESIDENT'] != ('serial' if mode == 'on' else 'off'):
                    raise ValueError('Resident switch mismatch')
                for key in ('CARGO_TARGET_DIR', 'JUSTRUST_CARGO_RESIDENT',
                            'JUSTRUST_CARGO_RESIDENT_DIR', 'JUSTRUST_CARGO_RESIDENT_TRACE'):
                    env.pop(key)
                if 'name' not in row:
                    wrapper_path = env.pop('RUSTC_WRAPPER', None)
                    if wrapper_path != str(arm_root / 'rustc-wrapper'):
                        raise ValueError('Missing wrapper fallback environment')
                if environment_reference is None:
                    environment_reference = env
                if env != environment_reference:
                    raise ValueError('Effective environment mismatch')
                signature = (invocation['command'], invocation['cwd'], invocation['timeout'])
                if reference is None:
                    reference = signature
                if signature != reference:
                    raise ValueError('Command/cwd/timeout changed')
            rows_by_mode[mode] = rows
            for row in rows:
                if row['name'] in samples:
                    samples[row['name']][mode].append(row['cargo']['invocation_wall_seconds'])
        for off, on in zip(rows_by_mode['off'], rows_by_mode['on']):
            if any(off[k] != on[k] for k in ('source_sha256', 'wrapper_sha256', 'nonce', 'semantics')):
                raise ValueError('Paired semantics/workload mismatch')
    return {name: dict(samples=arms,
                       medians={mode: statistics.median(values) for mode, values in arms.items()},
                       paired_off_over_on=[off / on for off, on in zip(arms['off'], arms['on'])])
            for name, arms in samples.items()}


def compare(before, after):
    left, right = validate(before), validate(after)
    if before['provenance'] != after['provenance']:
        raise ValueError('Provenance mismatch: candidates/workload/hostboot/affinity/environment must match exactly')
    for mode in ('off', 'on'):
        first = before['repetitions'][0]['arms'][mode]['observations'][0]['cargo']['invocation']
        second = after['repetitions'][0]['arms'][mode]['observations'][0]['cargo']['invocation']
        def comparable(invocation):
            env = dict(invocation['environment'])
            for key in ('CARGO_TARGET_DIR', 'JUSTRUST_CARGO_RESIDENT_DIR', 'JUSTRUST_CARGO_RESIDENT_TRACE'):
                env.pop(key)
            return dict(command=invocation['command'], timeout=invocation['timeout'],
                        workspace_layout=Path(invocation['cwd']).name, environment=env)
        if comparable(first) != comparable(second):
            raise ValueError('Effective invocation environment/workspace mismatch')
    return dict(scope=SCOPE, claim='Focused stages only, not a fullgate or default promotion.',
                stages={name: dict(before=left[name], after=right[name],
                                   on_wall_ratio=left[name]['medians']['on'] / right[name]['medians']['on'])
                        for name in MEASURED})


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    sub = parser.add_subparsers(dest='operation', required=True)
    run = sub.add_parser('run')
    for name in ('compiler', 'cargo', 'output'):
        run.add_argument('--' + name, type=Path, required=True)
    run.add_argument('--justrust', type=Path)
    run.add_argument('-n', type=int, default=2)
    run.add_argument('--timeout', type=float, default=180)
    diff = sub.add_parser('compare')
    diff.add_argument('before', type=Path)
    diff.add_argument('after', type=Path)
    args = parser.parse_args()
    os.umask(0o077)
    if args.operation == 'run':
        result = capture(args.compiler, args.cargo, args.output, args.n, args.timeout, args.justrust)
    else:
        result = compare(json.loads(args.before.read_text()), json.loads(args.after.read_text()))
    print(json.dumps(result, indent=2))


if __name__ == '__main__':
    main()
