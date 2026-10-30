#!/usr/bin/env python3
"""Offline admission of copied evidence. No subprocesses, native tools or workloads."""
import hashlib
import json
import re
import sys
from pathlib import Path


def require(ok, message):
    if not ok:
        raise ValueError(message)


def validate(root):
    def digest(path):
        return hashlib.sha256(path.read_bytes()).hexdigest()

    def read(relative):
        return json.loads((root / relative).read_text())

    entries = {}
    for line in (root / 'SHA256SUMS').read_text().splitlines():
        sha, name = line.split('  ', 1)
        path = Path(name)
        require(not path.is_absolute() and '..' not in path.parts, 'unsafe manifest path')
        require(name not in entries and re.fullmatch('[a-f0-9]{64}', sha), 'invalid manifest row')
        entries[name] = sha
        require(digest(root / name) == sha, f'hash mismatch: {name}')
    actual = {str(p.relative_to(root)) for p in root.rglob('*') if p.is_file()
              and p.name not in ('SHA256SUMS', 'validation.json') and '__pycache__' not in p.parts}
    require(actual == set(entries), 'manifest coverage mismatch')
    for path in ('root/final-v26-snapshot.exit', 'root/suite-v26-snapshot.exit',
                 'build/check-exit', 'build/compile-exit', 'native/full-native.exit'):
        require((root / path).read_text().strip() == '0', f'failed gate: {path}')
    before = read('native/source-copy-before.json')
    after = read('native/source-copy-after.json')
    require(len(before['original']) == 2444, 'unexpected source snapshot cardinality')
    require(before['original'] == before['copy'] == after['original'], 'original snapshot changed')
    require(before['selected'] == after['selected'], 'private snapshot changed')
    require([k for k in before['original'] if before['original'][k] != before['selected'][k]] == ['Cargo.lock'], 'non-lock private source delta')
    require(digest(root / 'native/private-resolved.Cargo.lock') == before['selected']['Cargo.lock'], 'private lock payload mismatch')
    for relative in ('crates/hir/src/diagnostics.rs', 'crates/hir/src/lib.rs',
                     'crates/ide-diagnostics/src/lib.rs', 'crates/ide-diagnostics/src/tests.rs'):
        frozen = root / 'build/frozen-source/vendor/rust-analyzer' / relative
        require(digest(frozen) == before['original'][relative], f'compiled source mismatch: {relative}')
        handed = root / 'source-ready/frozen/toolchain/resident/vendor/rust-analyzer' / relative
        require(frozen.read_bytes() == handed.read_bytes(), f'handoff mismatch: {relative}')
    require((root / 'build/frozen-source/crates/ra-jit/src/main.rs').read_bytes() ==
            (root / 'source-ready/frozen/toolchain/resident/crates/ra-jit/src/main.rs').read_bytes(), 'main handoff mismatch')
    admission = read('native/positive-test-admission.json')
    require(admission['verdict'] == 'passed', 'native admission failed')
    require([row['passed'] for row in admission['native_test_rows']] == [6, 2, 5, 2, 3, 1, 1, 3], 'native gate count mismatch')
    for row in admission['native_test_rows']:
        journal = root / 'native/raw-runs' / row['run_id'] / 'output.jsonl'
        lines = '\n'.join(json.loads(line)['l'] for line in journal.read_text().splitlines())
        require(f"test result: ok. {row['passed']} passed; 0 failed" in lines, f"raw gate missing: {row['filter']}")
    full = root / 'native/raw-runs/20261030-135710393-3931042/output.jsonl'
    lines = '\n'.join(json.loads(line)['l'] for line in full.read_text().splitlines())
    require('test result: ok. 351 passed; 0 failed' in lines, 'raw full-native result missing')
    require(len(re.findall(r'^test .* \.\.\. ok(?: <[0-9.]+s>)?$', lines, re.M)) == 351, 'raw native individual test count mismatch')
    regression = (root / 'root/v26-final-offline-regression.log').read_text()
    require('Ran 700 tests in 10.498s' in regression and '[stderr] OK' in regression, '700 regression gate missing')
    for name in ('rsb17', 'rsa23'):
        metadata = read(f'disabled-runtime/{name}/metadata.json')
        require(metadata['status'] == 'complete' and metadata['unchanged'] and metadata['source_restored'], f'{name}: dirty run')
        require(metadata['source_before'] == metadata['source_after'], f'{name}: source mismatch')
        require(not any('DIAGNOSTICS_STATS' in key for key in metadata['environments']['engine']), f'{name}: telemetry enabled')
        suite = read(f'disabled-runtime/{name}/suite.json')
        for artifact, sha in suite['artifacts_sha256'].items():
            require(digest(root / f'disabled-runtime/{name}' / artifact) == sha, f'{name}: suite artifact mismatch')
    state = read('profile/state.json')
    require(state['status'] == 'passed' and state['unchanged'] and len(state['rows']) == 6, 'profile admission failed')
    require(state['source_before'] == state['source_after'] and state['toolchain_before'] == state['toolchain_after'], 'profile identity changed')
    engine_sha = (root / 'build/engine-identity.sha256').read_text().split()[0]
    require(engine_sha == state['configs']['new']['sha256'] == state['tools_after']['new'] ==
            read('disabled-runtime/rsa23/metadata.json')['tools_before']['engine']['sha256'], 'engine identity linkage mismatch')
    require([row['replicate'] for row in state['rows']] == list(range(6)), 'profile replicate coverage mismatch')
    grouped = read('inventory-groups.json')
    for row, groups in zip(state['rows'], grouped['rows'], strict=True):
        rep = row['replicate']
        require(row['status'] == 0 and not row['timed_out'] and row['error'] is None, f'profile row failed: {rep}')
        trace = root / f'profile/{rep}-new.trace.json'
        delta = root / f'profile/{rep}-new.worker-delta.log'
        require(digest(trace) == row['trace_sha256'] and digest(delta) == row['worker_delta_sha256'], 'profile raw hash mismatch')
        parsed = [dict(label=label, calls=int(calls), cumulative_ns=int(ns)) for label, calls, ns in
                  re.findall(r'diagnostics collector: label=(hir\.collect\.[a-z_]+) calls=(\d+) cumulative_ns=(\d+) inclusive=true', delta.read_text())]
        require(parsed == row['collector_inventory'], 'raw inventory mismatch')
        callbacks = [[int(v) for v in match] for match in re.findall(
            r'asked_callback_calls=(\d+) asked_callback_cumulative_ns=(\d+)', delta.read_text())]
        transport = [[int(v) for v in match] for match in re.findall(
            r'diagnostics transport: retained=(\d+) stale=(\d+) dropped=(\d+) cap=4000', delta.read_text())]
        require(callbacks == row['callbacks'] and callbacks, 'raw callback accounting mismatch')
        require(transport == row['transport'] and transport, 'raw transport accounting mismatch')
        require([c for group in groups['collections'] for c in group] == parsed, 'grouping mismatch')
        require(any(c['label'] == 'hir.collect.selected_body_inclusive' and c['calls'] == 373 for c in parsed), '373-body normal edit missing')
        for group in groups['collections']:
            require(0 < len(group) <= 16 and len({c['label'] for c in group}) == len(group), 'unbounded or duplicate category group')
            require(all(c['calls'] > 0 and c['cumulative_ns'] > 0 for c in group), 'nonpositive actual counters')
        require(all(drop == 0 for _, _, drop in row['transport']), 'dropped actual spans')
        events = json.loads(trace.read_text())['traceEvents']
        require(not any(str(e.get('name', '')).startswith('hir.collect.') for e in events), 'synthetic inventory timeline event')
        require(any(e.get('name') == 'diagnostics.semantic.collect' for e in events), 'actual outer collection event missing')
    return {'status': 'passed', 'files': len(entries), 'profile_rows': 6,
            'snapshot_files': 2444, 'native_tests': 351, 'bench_regression_tests': 700,
            'claim': 'bounded inclusive attribution only, no speedup claim'}


if __name__ == '__main__':
    root = Path(sys.argv[1]).resolve() if len(sys.argv) > 1 else Path(__file__).resolve().parent
    try:
        result = validate(root)
    except (ValueError, KeyError, OSError, TypeError) as error:
        print(json.dumps({'status': 'failed', 'error': str(error)}))
        raise SystemExit(1)
    print(json.dumps(result, sort_keys=True))
