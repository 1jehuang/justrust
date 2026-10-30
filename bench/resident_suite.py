#!/usr/bin/env python3
"""Scoped native resident measurements. Never a comprehensive promotion gate."""
import hashlib
import json
import math
import os
from pathlib import Path
import platform
import re
import statistics
import subprocess

import resident_engine_compare as harness

SCHEMA = 'native_resident/v1'


def digest(path):
    return hashlib.sha256(Path(path).read_bytes()).hexdigest()


def require(condition, message):
    if not condition:
        raise ValueError('native resident: ' + message)


def pinned_hashes():
    repo = Path(__file__).resolve().parent.parent
    names = subprocess.check_output(['git', 'ls-tree', '-rz', harness.PIN], cwd=repo)
    result = {}
    for entry in names.split(b'\0'):
        if entry:
            info, name = entry.split(b'\t', 1)
            oid = info.split()[2].decode()
            blob = subprocess.check_output(['git', 'cat-file', 'blob', oid], cwd=repo)
            result[os.fsdecode(name)] = hashlib.sha256(blob).hexdigest()
    require(len(result) == 63, 'pinned source must contain exactly 63 files')
    return result


def host_identity():
    cpu = Path('/proc/cpuinfo').read_text()
    models = sorted(set(re.findall(r'^model name\s*:\s*(.+)$', cpu, re.M)))
    return {'host': platform.node(), 'cpu': models, 'ncpu': os.cpu_count(),
            'machine': platform.machine(), 'kernel': platform.release(), 'execution': 'local native direct'}


def replay(root, expected_source):
    """Recompute gates and samples from records and raw logs, not summary booleans."""
    metadata = json.loads((root / 'metadata.json').read_text())
    rows = json.loads((root / 'records.json').read_text())
    require(metadata.get('pin') == harness.PIN, 'wrong source pin')
    require(metadata.get('source_before') == expected_source == metadata.get('source_after'), 'source63 hashes differ')
    for key in ('tools', 'toolchain'):
        require(metadata.get(key + '_before') and metadata[key + '_before'] == metadata.get(key + '_after'), key + ' changed')
    require(metadata.get('source_restored') is True and metadata.get('toolchain_links_valid') is True, 'source/tools not restored')
    require(not metadata.get('error') and not metadata.get('identity_error'), 'harness error')
    require(metadata.get('profiles') == harness.runtime.PROFILES, 'profile mismatch')
    require(metadata.get('store_scope') == 'unseeded private cold stores, no automatic seeding', 'seeded or unknown stores')
    require(set(metadata['tools_before']) in ({'reference', 'engine'}, {'reference', 'engine', 'proc-macro-server'}), 'missing or unexpected engine identities')
    for identity in metadata['tools_before'].values():
        require(isinstance(identity, dict) and Path(identity.get('path', '')).is_absolute()
                and re.fullmatch(r'[0-9a-f]{64}', identity.get('sha256', '')), 'malformed executable identity')
    files = metadata.get('toolchain_before', {}).get('files')
    require(isinstance(files, dict) and {'bin/cargo', 'bin/rustc', 'lib/rustlib/components',
            'lib/rustlib/multirust-channel-manifest.toml',
            'lib/rustlib/wasm32-wasip1/lib/self-contained/crt1-reactor.o'} <= set(files), 'missing full toolchain identity')
    require(any(name.startswith('lib/rustlib/src/rust/library/') for name in files), 'missing toolchain library sources')
    require(all(isinstance(value, dict) and re.fullmatch(r'[0-9a-f]{64}', value.get('sha256', ''))
                and 'link' in value for value in files.values()), 'malformed toolchain file identity')
    manifest = metadata['toolchain_before']['files'].get('lib/rustlib/multirust-channel-manifest.toml', {})
    require(manifest.get('sha256') == harness.TOOLCHAIN_MANIFEST, 'wrong toolchain manifest')
    original = (root / 'source.original').read_bytes()
    require(hashlib.sha256(original).hexdigest() == expected_source['src/summary.rs'], 'stale backup')
    samples = {}
    artifacts = ['metadata.json', 'records.json', 'summary.json', 'source.original']
    expected_tests = None
    for stack in ('reference', 'engine'):
        selected = [r for r in rows if r.get('stack') == stack]
        wanted = [('version', 0), ('prepare', 0), ('type-error', 0), ('type-recovery', 0),
                  ('assert-failure', 0), ('assert-recovery', 0)] + [('edit', n) for n in range(5)]
        require([(r.get('phase'), r.get('replicate')) for r in selected] == wanted, 'missing/duplicate/out-of-order phases: ' + stack)
        env = metadata.get('environments', {}).get(stack, {})
        require(env.get('BLAZE_BACKEND') == 'cranelift' and env.get('BLAZE_OPT') == '0', 'backend mismatch')
        require(all(env.get(k) == v for k, v in harness.runtime.PROFILES.items()), 'effective profile mismatch')
        for key in ('BLAZE_REMOTE', 'RA_JIT_REMOTE'):
            require(env.get(key) == 'off', 'remote execution not allowed')
        for key in ('BLAZE_PROC_MACRO_SERVER', 'RA_JIT_PROC_MACRO_SERVER'):
            server = metadata['tools_before'].get('proc-macro-server', {}).get('path', '0')
            require(env.get(key) == server, 'proc macro policy mismatch')
        values = []
        for row in selected:
            phase, rep = row['phase'], row['replicate']
            prefix = f'{stack}-{phase}-{rep}'
            logs = [prefix + '.' + suffix for suffix in ('stdout', 'stderr')]
            artifacts.extend(logs)
            text = '\n'.join((root / name).read_text(errors='replace') for name in logs)
            require(row.get('error') is None and row.get('timed_out') is False, 'process error/timeout')
            elapsed = row.get('elapsed_seconds')
            require(type(elapsed) in (int, float) and math.isfinite(elapsed) and elapsed > 0, 'invalid timing')
            tool = Path(metadata['tools_before'][stack]['path'])
            command = [str(tool), '--version'] if phase == 'version' else harness.command(tool, stack)
            require(row.get('command') == command, 'stale command identity')
            contents = original
            if phase == 'type-error':
                contents = harness.runtime.edit_bytes(original, kind='type-error')
            elif phase == 'assert-failure':
                contents = harness.runtime.edit_bytes(original, kind='test-failure')
            elif phase == 'edit':
                contents = harness.runtime.edit_bytes(original, rep)
            require(row.get('source_sha256') == hashlib.sha256(contents).hexdigest(), 'stale edit identity')
            parsed = harness.runtime.test_results(text)
            if phase == 'version':
                pattern = '^' + (re.escape(harness.REFERENCE_VERSION) + '$' if stack == 'reference' else r'jr-engine\s+\S+')
                require(row.get('status') == 0 and re.search(pattern, text, re.M), 'wrong executable version')
            elif phase == 'type-error':
                require(row.get('status') == 101 and 'E0308' in text and 'JR_RUNTIME_TYPE_ERROR' in text, 'missing real type failure')
            else:
                failing = phase == 'assert-failure'
                require(row.get('status') == (101 if failing else 0) and harness.valid_tests(parsed, expected_tests, failing), 'libtest parity failure')
                if expected_tests is None:
                    expected_tests = parsed
                if failing:
                    require('JR_RUNTIME_FAILURE' in text, 'missing assertion failure')
                if phase == 'edit':
                    require(re.findall(r'JR_RUNTIME_EXECUTED_[^\s]*', text) == [f'JR_RUNTIME_EXECUTED_{rep + 1}'], 'missing/duplicate runtime marker')
                    values.append(elapsed)
        samples[stack] = {'samples_seconds': values, 'median_seconds': statistics.median(values)}
    require(all(r.get('stack') in ('reference', 'engine') for r in rows), 'unexpected stack')
    return metadata, samples, {name: digest(root / name) for name in artifacts}


def validate(result):
    require(result.get('schema') == SCHEMA, 'unsupported schema')
    root = Path(result['artifacts_root'])
    metadata, samples, receipts = replay(root, pinned_hashes())
    require(receipts == result.get('artifacts_sha256'), 'raw artifact receipt mismatch')
    require(samples == result.get('measurements'), 'sample receipt mismatch')
    require(metadata == result.get('metadata'), 'metadata receipt mismatch')
    host = result.get('host')
    require(isinstance(host, dict) and all(host.get(k) for k in ('host', 'cpu', 'ncpu', 'machine', 'kernel', 'execution')), 'missing host identity')
    require(host['execution'] == 'local native direct', 'wrong execution mode')
    return metadata, samples


def cmd_resident(args):
    args.blaze = None
    before_host = host_identity()
    status = harness.execute(args)
    require(status == 0, 'harness failed, raw evidence retained')
    require(host_identity() == before_host, 'host changed')
    metadata, samples, receipts = replay(args.output, pinned_hashes())
    result = {'schema': SCHEMA, 'artifacts_root': str(args.output),
              'artifacts_sha256': receipts, 'metadata': metadata, 'measurements': samples,
              'host': before_host, 'noise': 'unknown', 'scope': 'scoped diagnostic, not comprehensive or default promotion'}
    target = args.output / 'suite.json'
    target.write_text(json.dumps(result, indent=2) + '\n')
    print(target)


def compare(a, b, floor=10.0):
    require(a.get('schema') == SCHEMA and b.get('schema') == SCHEMA, 'cannot mix native and non-native results')
    ma, sa = validate(a)
    mb, sb = validate(b)
    require(a['host'] == b['host'], 'host/cpu/execution differs')
    for key in ('source_before', 'toolchain_before', 'profiles', 'proc_macro_policy', 'store_scope'):
        require(ma[key] == mb[key], 'different ' + key)
    for key in ('reference', 'proc-macro-server'):
        require(ma['tools_before'].get(key) == mb['tools_before'].get(key), 'different source-built reference or server')
    require(math.isfinite(floor) and floor >= 0, 'invalid floor')
    old, new = sa['engine']['median_seconds'], sb['engine']['median_seconds']
    delta = (new / old - 1) * 100
    ra = old / sa['reference']['median_seconds']
    rb = new / sb['reference']['median_seconds']
    print('Native resident scoped diagnostic: descriptive/uncontrolled, noise unknown.')
    print(f'Engine median: {old:.6f}s -> {new:.6f}s, delta {new-old:+.6f}s ({delta:+.2f}%).')
    print(f'Reference-normalized engine/reference ratio: {ra:.6f} -> {rb:.6f}, relative delta {(rb/ra-1)*100:+.2f}%.')
    print(f'Absolute median delta {"exceeds" if abs(delta) > floor else "does not exceed"} requested {floor:g}% floor. No significance or speedup verdict.')
    print('Not a comprehensive workload or default-promotion gate.')


def add_parser(sub):
    parser = sub.add_parser('resident', help=__doc__)
    for name in ('reference-engine', 'engine', 'source', 'output', 'toolchain'):
        parser.add_argument('--' + name, type=harness.absolute, required=True)
    parser.add_argument('--proc-macro-server', type=harness.absolute)
    parser.add_argument('--samples', type=int, choices=(5,), default=5)
    parser.add_argument('--timeout', type=float, default=360)
    parser.set_defaults(fn=cmd_resident)
