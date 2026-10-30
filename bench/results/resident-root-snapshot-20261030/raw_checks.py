#!/usr/bin/env python3
"""Offline raw checks adapted from sibling signature evidence validator. No execution.
Only suite parsing is reused; version identities are explicit caller arguments.
"""
import argparse
import hashlib
import json
import math
from pathlib import Path
import re
import statistics

NAMES = sorted('summary::tests::' + n for n in (
    'parses_libtest_results', 'shares_overlapping_time',
    'splits_passes_without_double_counting', 'test_failures_are_not_compile_errors'))
PROFILES = {f'CARGO_PROFILE_{p}_{k}': v for p in ['DEV', 'TEST'] for k, v in
            [('OPT_LEVEL', '0'), ('DEBUG', '0'), ('INCREMENTAL', 'true')]}
REFERENCE = 'blaze 0.20261027.0 (5d4f78f7577cf7170993b74f735099dd03f5c6ca)'
REFERENCE_HASH = 'fa622aeca09858c050264f701fbbebd1a14aa07d2b02c1eb4de511b684ba0263'

class InvalidEvidence(ValueError):
    pass


def need(condition, message):
    if not condition:
        raise InvalidEvidence(message)


def sha(data):
    return hashlib.sha256(data).hexdigest()


def canonical(value):
    return sha(json.dumps(value, sort_keys=True, separators=(',', ':')).encode())


class Evidence:
    def __init__(self, root, overrides=None):
        self.root = Path(root)
        self.overrides = overrides or {}

    def read(self, relative):
        path = Path(relative)
        need(not path.is_absolute() and '..' not in path.parts, 'unsafe artifact path')
        if relative in self.overrides:
            need(self.overrides[relative] is not None, f'missing artifact {relative}')
            return self.overrides[relative]
        try:
            need(all(not (self.root / item).is_symlink() for item in [path, *path.parents]), f'artifact symlink {relative}')
            return (self.root / path).read_bytes()
        except OSError as error:
            raise InvalidEvidence(f'missing artifact {relative}') from error

    def text(self, relative):
        return self.read(relative).decode('utf-8')

    def obj(self, relative):
        return json.loads(self.text(relative))

    def journal(self, relative):
        return [json.loads(line) for line in self.text(relative).splitlines()]


def libtest(text, native=False):
    pattern = r'^test (\S+) \.\.\. (?:JR_RUNTIME_EXECUTED_\d+\n)?(ok|FAILED|ignored)(?: <[0-9.]+s>)?$'
    tests = re.findall(pattern, text, re.M)
    footers = re.findall(r'^test result: (ok|FAILED)\. (\d+) passed; (\d+) failed; (\d+) ignored; (\d+) measured; (\d+) filtered out;', text, re.M)
    need(len(footers) == 1, 'missing/duplicate raw libtest footer')
    status, passed, failed, ignored, measured, filtered = footers[0]
    counts = tuple(map(int, (passed, failed, ignored, measured, filtered)))
    need(status == ('FAILED' if counts[1] else 'ok'), 'raw libtest verdict disagrees with counts')
    need(len(tests) == len(set(name for name, _ in tests)), 'duplicate raw native test names')
    need(sum(state == 'ok' for _, state in tests) == counts[0] and
         sum(state == 'FAILED' for _, state in tests) == counts[1] and
         sum(state == 'ignored' for _, state in tests) == counts[2], 'raw test names disagree with counts')
    need(('running %d tests' % len(tests)) in text or ('running %d test' % len(tests)) in text, 'missing positive test admission')
    return sorted(tests), counts


def edit_bytes(original, phase, replicate):
    anchor = b'fn shares_overlapping_time() {'
    need(original.count(anchor) == 1, 'workload edit anchor mismatch')
    n = replicate + 1
    body = (f'let observed = std::hint::black_box({n}u64); assert_eq!(observed, {n}u64); '
            f'println!("JR_RUNTIME_EXECUTED_{n}");').encode()
    if phase == 'type-error':
        body = b'let _: u64 = "JR_RUNTIME_TYPE_ERROR";'
    elif phase == 'assert-failure':
        body = b'assert_eq!(std::hint::black_box(1u64), 2u64, "JR_RUNTIME_FAILURE");'
    return original.replace(anchor, anchor + b'\n' + body, 1)


def session(e, name, engine_hash, build_id, version):
    prefix = f'sessions/{name}/'
    metadata, suite, rows = [e.obj(prefix + f'{n}.json') for n in ['metadata', 'suite', 'records']]
    need(suite['schema'] == 'native_resident/v1' and suite['metadata'] == metadata, 'suite metadata mismatch')
    need(len(rows) == 22 and metadata['status'] == 'complete' and metadata['unchanged'] is True and
         metadata['source_restored'] is True and metadata['toolchain_links_valid'] is True, 'incomplete runtime session')
    need(metadata['source_before'] == metadata['source_after'] and len(metadata['source_before']) == 63 and
         canonical(metadata['source_before']) == '79c53ff6e87a22c84c9bc7bac6917c69b3f8d0fbc29b107f734cc00cc39a64df', 'wrong pinned source63 manifest')
    need(metadata['toolchain_before'] == metadata['toolchain_after'] and
         canonical(metadata['toolchain_before']) == '8bdbccdd1ca3fd7f2935cfb5667047eb4c46824928823e96311d8eb5ec0a6371', 'wrong pinned full toolchain identity')
    need(metadata['tools_before'] == metadata['tools_after'] and set(metadata['tools_before']) == {'reference', 'engine'}, 'changed/unexpected executable identities')
    need(metadata['tools_before']['reference']['sha256'] == REFERENCE_HASH and
         metadata['tools_before']['engine']['sha256'] == engine_hash, 'wrong exact executable hash')
    need(metadata['pin'] == 'f97fcc2' and metadata['reference_version_required'] == REFERENCE and
         metadata['profiles'] == PROFILES and metadata['store_scope'] == 'unseeded private cold stores, no automatic seeding', 'runtime input/policy mismatch')
    original = e.read(prefix + 'source.original')
    need(sha(original) == metadata['source_before']['src/summary.rs'], 'wrong original workload source')
    artifacts = suite['artifacts_sha256']
    need(len(artifacts) == 48, 'incomplete suite raw inventory')
    for path, digest in artifacts.items():
        need(sha(e.read(prefix + path)) == digest, 'raw suite checksum mismatch')
    samples = {}
    wanted = [('version', 0), ('prepare', 0), ('type-error', 0), ('type-recovery', 0),
              ('assert-failure', 0), ('assert-recovery', 0)] + [('edit', n) for n in range(5)]
    need(all(row['stack'] in ['reference', 'engine'] for row in rows), 'unexpected stack')
    for stack in ['reference', 'engine']:
        env = metadata['environments'][stack]
        need(all(env.get(k) == v for k, v in PROFILES.items()), 'profile environment mismatch')
        need(env['BLAZE_BACKEND'] == 'cranelift' and env['BLAZE_OPT'] == '0', 'wrong runtime backend')
        need(all(env.get(k) == v for k, v in [('BLAZE_REMOTE', 'off'), ('RA_JIT_REMOTE', 'off'),
             ('BLAZE_PROC_MACRO_SERVER', '0'), ('RA_JIT_PROC_MACRO_SERVER', '0')]), 'remote/proc-macro policy mismatch')
        private = f'/home/jeremy/{name}/{stack}/'
        for key in ['CARGO_HOME', 'CARGO_TARGET_DIR', 'RUSTUP_HOME', 'BLAZE_CACHE', 'RA_JIT_CACHE', 'BLAZE_HOME', 'RA_JIT_HOME']:
            need(env[key].startswith(private), 'stores not private/unseeded')
        selected = [row for row in rows if row['stack'] == stack]
        need([(r['phase'], r['replicate']) for r in selected] == wanted, 'missing/duplicate/out-of-order runtime phases')
        values = []
        for row in selected:
            phase, rep = row['phase'], row['replicate']
            raw = '\n'.join(e.text(prefix + f'{stack}-{phase}-{rep}.{s}') for s in ['stdout', 'stderr'])
            need(row['error'] is None and row['timed_out'] is False, 'runtime process error/timeout')
            elapsed = row['elapsed_seconds']
            need(type(elapsed) in [float, int] and math.isfinite(elapsed) and elapsed > 0, 'invalid raw timing')
            command = [metadata['tools_before'][stack]['path']]
            command += ['--version'] if phase == 'version' else ((['cargo'] if stack == 'reference' else []) +
                       ['test', '--lib', '--', 'summary::tests::', '--test-threads=1', '--nocapture'])
            need(row['command'] == command, 'runtime command mismatch')
            need(row['environment']['CARGO_TARGET_DIR'] == env['CARGO_TARGET_DIR'] and
                 all(row['environment'].get(k) == v for k, v in PROFILES.items()), 'recorded command environment mismatch')
            # The historical per-command allowlist omits remote/proc-macro
            # variables. Those are checked in session metadata above, not
            # invented as a per-command attestation here. Backend is recorded.
            need(row['environment'].get('BLAZE_BACKEND') == 'cranelift' and
                 row['environment'].get('BLAZE_OPT') == '0',
                 'effective per-command backend mismatch')
            contents = edit_bytes(original, phase, rep) if phase in ['type-error', 'assert-failure', 'edit'] else original
            need(row['source_sha256'] == sha(contents), 'stale workload edit hash')
            need(row['noise']['contamination'] == 'unknown', 'noise guarantee invented')
            if phase == 'version':
                need(row['status'] == 0 and (REFERENCE if stack == 'reference' else build_id) in raw, 'wrong raw executable version')
                need(metadata['versions'][stack]['valid'] is True and metadata['versions'][stack]['text'].strip() == raw.strip(), 'version metadata mismatch')
            elif phase == 'type-error':
                need(row['status'] == 101 and row['gate_passed'] is True and 'E0308' in raw and
                     'JR_RUNTIME_TYPE_ERROR' in raw and 'test result:' not in raw, 'missing actual type error gate')
            else:
                tests, counts = libtest(raw)
                failing = phase == 'assert-failure'
                expected = [(n, 'FAILED' if failing and n.endswith('shares_overlapping_time') else 'ok') for n in NAMES]
                need(tests == expected and counts == (3 if failing else 4, 1 if failing else 0, 0, 0, 144), 'raw libtest correctness parity failed')
                need(row['status'] == (101 if failing else 0), 'wrong runtime process status')
                need(row['parsed']['tests'] == [list(t) for t in expected] and
                     row['parsed']['passed'] == counts[0] and row['parsed']['failed'] == counts[1], 'recorded parsed tests mismatch')
                if failing:
                    need(row['gate_passed'] is True and 'JR_RUNTIME_FAILURE' in raw, 'missing assertion failure gate')
                if phase == 'edit':
                    need(row['accepted'] is True and re.findall(r'JR_RUNTIME_EXECUTED_[^\s]*', raw) == [f'JR_RUNTIME_EXECUTED_{rep + 1}'], 'stale/missing/duplicate runtime marker')
                    values.append(elapsed)
        samples[stack] = {'samples_seconds': values, 'median_seconds': statistics.median(values)}
    need(suite['measurements'] == samples, 'suite timing summary not derived from raw records')
    summary = e.obj(prefix + 'summary.json')
    need(summary['medians_seconds'] == {k: v['median_seconds'] for k, v in samples.items()} and
         summary['excluded'] == [] and summary['source_built_parity'] is True, 'excluded sample or false parity summary')
    need(suite['noise'] == 'unknown' and suite['scope'] == 'scoped diagnostic, not comprehensive or default promotion', 'scope/noise overclaim')
    build = f'native/candidate-build-v{version}'
    need(e.text(build + '/engine-identity.sha256').split()[0] == engine_hash and
         e.text(build + '/engine-version.txt').strip() == metadata['versions']['engine']['text'].strip(), 'build/runtime executable binding mismatch')
    return metadata, samples, suite['host']


