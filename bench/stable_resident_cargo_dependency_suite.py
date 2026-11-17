#!/usr/bin/env python3
"""Counterbalanced actual-client dependency owner costs, never a promotion gate.

Execution requires frozen candidates and explicit coordinator idle authorization.
RSS is a post-invocation live worker observation, not peak RSS or retained heap size.
"""
import argparse
import base64
import re
import hashlib
import json
import math
from pathlib import Path
import statistics
import uuid

import stable_resident_cargo_dependency as acceptance

shared = acceptance.shared
safety = acceptance.safety
SCHEMA = 'stable-resident-cargo-dependency-suite/v1'
CLAIM = ('Focused dependency fixture actual-client wall time and live worker RSS only. '
         'No general speedup, peak-memory, retained-heap, or default promotion claim.')


def worker_rss(pid, proc=Path('/proc')):
    """Read only the invoking host's /proc, rejecting PID exit/reuse races."""
    result = dict(pid=pid, available=False, source='local-/proc', kind='post-invocation-live-rss')
    if type(pid) is not int or pid <= 0:
        return dict(result, reason='invalid-pid')
    try:
        directory = proc / str(pid)
        before = (directory / 'stat').read_text().rsplit(')', 1)[1].split()[19]
        status = (directory / 'status').read_text()
        after = (directory / 'stat').read_text().rsplit(')', 1)[1].split()[19]
        if before != after:
            return dict(result, reason='pid-generation-changed')
        fields = dict(line.split(':', 1) for line in status.splitlines() if ':' in line)
        rss = fields['VmRSS'].split()
        if len(rss) != 2 or rss[1] != 'kB' or int(rss[0]) < 0:
            raise ValueError('invalid VmRSS')
        return dict(result, available=True, rss_bytes=int(rss[0]) * 1024,
                    starttime_ticks=int(before))
    except (OSError, ValueError, KeyError, IndexError) as error:
        return dict(result, reason=type(error).__name__)


def observed_runner(runner, sampler=worker_rss):
    timed = safety.timed_runner(runner)

    def invoke(command, timeout, cwd, env):
        trace = Path(env['JUSTRUST_CARGO_RESIDENT_TRACE'])
        record = timed(command, timeout, cwd=cwd, env=env)
        # Include earlier generations still resident in this arm, not only the
        # worker serving the current request. Dependency edits can retain owners.
        events = shared.trace_events(trace)
        pids = sorted({event.get('result', {}).get('pid') for event in events
                       if event.get('event') == 'complete'
                       and type(event.get('result', {}).get('pid')) is int})
        return dict(record, resident_worker_rss=[sampler(pid) for pid in pids])
    return invoke


def capture(compiler, cargo, output, justrust, repetitions=2, timeout=180, *,
            root_native_frozen=False, idle_authorized=False,
            expected_compiler_sha256=None, expected_driver_sha256=None,
            expected_cargo_sha256=None, expected_client_sha256=None, runner=None):
    if root_native_frozen is not True or idle_authorized is not True:
        raise ValueError('Root native freeze and idle authorization required before execution')
    if type(repetitions) is not int or repetitions < 2 or repetitions % 2:
        raise ValueError('Even repetition count >= 2 required')
    if type(timeout) not in (float, int) or not math.isfinite(timeout) or timeout <= 0:
        raise ValueError('Finite positive timeout required')
    if justrust is None:
        raise ValueError('Actual justrust client required')
    shared.check_socket_path(output, repetitions)
    compiler, cargo, client = [Path(p).resolve(strict=True) for p in (compiler, cargo, justrust)]
    expected = dict(compiler_sha256=expected_compiler_sha256, driver_sha256=expected_driver_sha256,
                    cargo_sha256=expected_cargo_sha256, client_sha256=expected_client_sha256)
    attested = safety.attest(compiler, cargo, client, expected)
    original = shared.SOURCE.read_bytes()
    root = shared.private_directory(output)
    (root / 'original.rs').write_bytes(original)
    fixture = shared.private_directory(root / 'fixture')
    for path in ('src', 'dep', 'dep/src'):
        (fixture / path).mkdir(mode=0o700)
    (fixture / 'Cargo.toml').write_text(acceptance.MANIFEST)
    (fixture / 'dep/Cargo.toml').write_text(acceptance.DEP_MANIFEST)
    host = safety.host()
    report = dict(schema=SCHEMA, verdict='incomplete', claim=CLAIM,
                  provenance=dict(attestation=attested, expected=expected, host=host,
                                  root_native_frozen=True, idle_authorized=True,
                                  repetitions=repetitions, timeout=timeout,
                                  original_b64=shared.native.encoded(original),
                                  helpers={Path(module.__file__).name: shared.native.digest(Path(module.__file__))
                                           for module in (acceptance, shared, safety, shared.native, shared.dogfood)},
                                  harness_sha256=shared.native.digest(Path(__file__))), repetitions=[])
    invoke = shared.justrust_runner(client, cargo, runner=observed_runner(runner or shared.native.run))
    try:
        for index in range(repetitions):
            nonce = uuid.uuid4().hex
            workload = acceptance.stages(nonce)
            pair = dict(order=['off', 'on'] if index % 2 == 0 else ['on', 'off'], arms={},
                        nonce=nonce, workload=[dict(name=stage['name'], root_b64=shared.native.encoded(stage['root']),
                                                   dependency_b64=shared.native.encoded(stage['dependency']))
                                              for stage in workload])
            report['repetitions'].append(pair)
            for mode in pair['order']:
                arm = root / f'{index:03d}-{mode}'
                try:
                    pair['arms'][mode] = acceptance.capture_arm(
                        compiler, cargo, fixture, arm, mode, workload, timeout, invoke,
                        require_resident_root=True, owned_batch=True)
                finally:
                    receipt = arm / 'receipt.json'
                    if receipt.exists():
                        raw = receipt.read_bytes()
                        pair.setdefault('original_receipts', {})[mode] = dict(
                            bytes_b64=shared.native.encoded(raw), sha256=hashlib.sha256(raw).hexdigest())
                        pair['arms'][mode] = json.loads(raw)
                    target = fixture / ('target-' + mode)
                    if target.exists():
                        target.rename(arm / 'target-evidence')
            if attested != safety.attest(compiler, cargo, client, expected):
                raise AssertionError('Frozen candidates changed during pair')
        if original != shared.SOURCE.read_bytes() or host != safety.host():
            raise AssertionError('Original bytes or execution host changed')
        report['verdict'] = 'passed'
        report['summary'] = validate(report)
    except BaseException as error:
        report.update(verdict='failed', error=f'{type(error).__name__}: {error}')
        raise
    finally:
        (root / 'receipt.json').write_text(json.dumps(report, indent=2) + '\n')
    return report


def validate(report):
    if report.get('schema') != SCHEMA or report.get('verdict') != 'passed':
        raise ValueError('Not a passed dependency cost suite')
    provenance = report['provenance']
    if (provenance.get('root_native_frozen') is not True
            or provenance.get('idle_authorized') is not True):
        raise ValueError('Missing execution authorization')
    original = base64.b64decode(provenance['original_b64'], validate=True)
    if not original or shared.native.encoded(original) != provenance['original_b64']:
        raise ValueError('Invalid original source receipt')
    count = provenance['repetitions']
    if type(count) is not int or count < 2 or count % 2 or len(report['repetitions']) != count:
        raise ValueError('Missing balanced repetitions')
    for component in ('compiler', 'driver', 'cargo', 'client'):
        wanted = provenance['expected'][component + '_sha256']
        if (not isinstance(wanted, str) or not re.fullmatch('[0-9a-f]{64}', wanted)
                or provenance['attestation'][component]['sha256'] != wanted):
            raise ValueError('Candidate attestation mismatch')
    samples = {}
    rss_samples = []
    rss_totals = {}
    for index, pair in enumerate(report['repetitions']):
        if pair['order'] != (['off', 'on'] if index % 2 == 0 else ['on', 'off']):
            raise ValueError('Counterbalance order mismatch')
        stages = acceptance.stages(pair['nonce'])
        expected_workload = [dict(name=s['name'], root_b64=shared.native.encoded(s['root']),
                                  dependency_b64=shared.native.encoded(s['dependency'])) for s in stages]
        if pair['workload'] != expected_workload:
            raise ValueError('Original workload bytes mismatch')
        semantics = {}
        for mode in ('off', 'on'):
            arm = pair['arms'][mode]
            receipt = pair['original_receipts'][mode]
            raw = base64.b64decode(receipt['bytes_b64'], validate=True)
            if (hashlib.sha256(raw).hexdigest() != receipt['sha256']
                    or json.loads(raw) != arm or arm.get('mode') != mode):
                raise ValueError('Exact original arm receipt mismatch')
            rows = arm['observations']
            if arm['verdict'] != 'passed' or [r['name'] for r in rows] != [s['name'] for s in stages]:
                raise ValueError('Incomplete correctness arm')
            previous_root = None
            previous_artifacts = None
            semantics[mode] = []
            known_pids = set()
            for row, stage in zip(rows, stages):
                if (row['root_sha256'] != hashlib.sha256(stage['root']).hexdigest()
                        or row['dependency_sha256'] != hashlib.sha256(stage['dependency']).hexdigest()):
                    raise ValueError('Source digest mismatch')
                if any(row[field] != stage[field] for field in ('expectation', 'nonce', 'root_only')):
                    raise ValueError('Stage identity mismatch')
                semantic = acceptance.semantics(row['cargo'], stage)
                if semantic != row['semantics']:
                    raise ValueError('Semantic receipt mismatch')
                semantics[mode].append(semantic)
                acceptance.validate_artifacts(stage, row['metadata'])
                previous_artifacts = acceptance.validate_dependency_transition(stage, row['metadata'], previous_artifacts)
                residency = acceptance.validate_trace(stage, row['trace'], mode, True)
                if mode == 'on':
                    acceptance.validate_owned_batch(stage, row['trace'], previous_root)
                    previous_root = acceptance.validate_root_transition(stage, residency, previous_root)
                    if previous_root is not None and stage['name'] != 'dependency-error':
                        roots = [e for e in row['trace'] if e.get('event') == 'complete'
                                 and e.get('crate_name') == 'dependency_fallback_root']
                        previous_root['owned_revisions'] = roots[0]['result']['owned']['revisions']
                cargo = row['cargo']
                wall = cargo['invocation_wall_seconds']
                if type(wall) not in (int, float) or not math.isfinite(wall) or wall <= 0:
                    raise ValueError('Invalid actual-client wall time')
                invocation = cargo['invocation']
                if invocation['command'] != [provenance['attestation']['client']['path'], 'cargo',
                        'test', '--offline', '--lib', '--message-format=json', '--', '--nocapture', '--test-threads=1']:
                    raise ValueError('Not actual attested client invocation')
                env = invocation['environment']
                if (safety.safe_environment(env) != env or invocation['timeout'] != provenance['timeout']
                        or env.get('RUSTC') != provenance['attestation']['compiler']['path']
                        or env.get('JUSTRUST_CARGO_RESIDENT') != ('off' if mode == 'off' else 'serial')
                        or env.get('JUSTRUST_REAL_CARGO') != provenance['attestation']['cargo']['path']
                        or env.get('RUSTFLAGS') != shared.FLAGS
                        or env.get('JUSTRUST_RUSTC_RESIDENT_FRONTEND') != '0'
                        or env.get('JUSTRUST_CARGO_RESIDENT_OWNED') != '1'
                        or env.get('JUSTRUST_CARGO_RESIDENT_EXTERNAL') != '1'):
                    raise ValueError('Invocation environment mismatch')
                samples.setdefault(stage['name'], dict(off=[], on=[]))[mode].append(wall)
                known_pids.update(e['result']['pid'] for e in row['trace']
                                  if e.get('event') == 'complete'
                                  and type(e.get('result', {}).get('pid')) is int)
                expected_pids = sorted(known_pids)
                if [rss.get('pid') for rss in cargo['resident_worker_rss']] != expected_pids:
                    raise ValueError('RSS worker identities differ from original trace')
                for rss in cargo['resident_worker_rss']:
                    if (type(rss.get('available')) is not bool
                            or rss.get('source') != 'local-/proc'
                            or rss.get('kind') != 'post-invocation-live-rss'):
                        raise ValueError('Invalid RSS provenance')
                    if rss['available']:
                        if type(rss.get('starttime_ticks')) is not int or rss['starttime_ticks'] < 0:
                            raise ValueError('Invalid worker generation')
                        if type(rss['rss_bytes']) is not int or rss['rss_bytes'] < 0:
                            raise ValueError('Invalid worker RSS')
                        rss_samples.append(rss['rss_bytes'])
                rss_rows = cargo['resident_worker_rss']
                total = (sum(rss['rss_bytes'] for rss in rss_rows)
                         if all(rss.get('available') is True for rss in rss_rows) else None)
                rss_totals.setdefault(stage['name'], dict(off=[], on=[]))[mode].append(total)
        if semantics['off'] != semantics['on']:
            raise ValueError('Exact off/on semantics differ')
    return dict(stages={name: dict(off_median_seconds=statistics.median(modes['off']),
                                  on_median_seconds=statistics.median(modes['on']),
                                  paired_on_minus_off_seconds=[on - off for off, on in zip(modes['off'], modes['on'])])
                        for name, modes in samples.items()},
                worker_rss=dict(available_samples=len(rss_samples),
                                stage_total_live_bytes=rss_totals,
                                total_interpretation='sum of known worker generations when all readable, null if incomplete',
                                max_observed_bytes=max(rss_samples) if rss_samples else None,
                                interpretation='post-invocation live worker RSS, not peak or retained heap'))


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    for name in ('compiler', 'cargo', 'output', 'justrust'):
        parser.add_argument('--' + name, type=Path, required=True)
    for name in ('compiler', 'driver', 'cargo', 'client'):
        parser.add_argument('--expected-' + name + '-sha256', required=True)
    parser.add_argument('--repetitions', type=int, default=2)
    parser.add_argument('--timeout', type=float, default=180)
    parser.add_argument('--root-native-frozen', action='store_true')
    parser.add_argument('--idle-authorized', action='store_true')
    print(json.dumps(capture(**vars(parser.parse_args()))['summary']))


if __name__ == '__main__':
    main()
