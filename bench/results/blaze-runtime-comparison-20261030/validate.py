#!/usr/bin/env python3
"""Audit retained acceptance observations, never infer cross-attempt speed."""
import hashlib
import json
from pathlib import Path
import statistics
import sys

ROOT = Path(__file__).resolve().parent
REPO = ROOT.parents[2]
sys.path.insert(0, str(REPO / 'bench'))
import blaze_runtime_compare as harness


def load(path):
    return json.loads(path.read_text())


def main():
    previous = None
    exports = 0
    for case in ('br30', 'br31', 'br32'):
        directory = ROOT / case
        meta = load(directory / 'metadata.json')
        assert meta['source_before'] == meta['source_after']
        assert meta['identities_before'] == meta['identities_after']
        if previous:
            assert meta['source_before'] == previous['source_before']
            assert meta['identities_before'] == previous['identities_before']
        previous = meta
        rows = load(directory / 'records.json')
        summary = load(directory / 'summary.json')
        expected_stacks = {'blaze'} if case == 'br30' else {'owned-off', 'all3on'}
        assert set(summary['medians_seconds']) == expected_stacks
        for stack in expected_stacks:
            group = [r for r in rows if r['stack'] == stack]
            samples = [r for r in group if r['phase'] == 'edit']
            assert len(samples) == 5 and all(r.get('accepted') for r in samples)
            assert all(r['status'] == 0 and not r['error'] and not r['timed_out'] for r in samples)
            assert statistics.median(r['elapsed_seconds'] for r in samples) == summary['medians_seconds'][stack]
            gates = [r for r in group if r['phase'] in ('type-error', 'test-failure')]
            assert len(gates) == 2 and all(r['status'] == 101 and r['gate_passed'] for r in gates)
            for row in samples:
                prefix = directory / f"edit-{row['replicate']:03d}-{stack}"
                if stack == 'blaze':
                    output = ''.join(prefix.with_suffix('.' + s).read_text() for s in ('stdout', 'stderr'))
                else:
                    receipt = row['raw_output']
                    artifact = directory / receipt['artifact']
                    raw = artifact.read_bytes()
                    assert hashlib.sha256(raw).hexdigest() == receipt['sha256']
                    output = '\n'.join(json.loads(s)['l'] for s in raw.decode().splitlines())
                expected = {**row['parsed'], 'tests': [tuple(x) for x in row['parsed']['tests']]}
                assert harness.accepted(row, output, expected, f"JR_RUNTIME_EXECUTED_{row['replicate'] + 1}")
        for stack in ('owned-off', 'owned-on'):
            machine = f'xps14-runtime-{case}-{stack}-20261030'
            exported = [json.loads(s) for p in (REPO / 'data/runs' / machine).glob('*.jsonl') for s in p.read_text().splitlines()]
            assert len(exported) == (1 if case == 'br30' else 10)
            exports += len(exported)
    gate = load(ROOT / 'native-dead-code/acceptance.json')
    assert gate['verdict'] == 'passed' and gate['completed_cases'] == gate['planned_cases'] == 32
    assert len(gate['observations']) == 32 and all(len(r['modes']) == 7 for r in gate['observations'])
    for name, sha in load(ROOT / 'evidence-sha256.json').items():
        assert hashlib.sha256((ROOT / name).read_bytes()).hexdigest() == sha, name
    result = {'verdict': 'passed', 'accepted_complete_cross_engine_comparison': False,
              'retained_attempts': 3, 'exported_owned_calls': exports,
              'native_correctness_invocations': 224,
              'scope': 'acceptance/provenance audit, not significance, savings or default promotion'}
    (ROOT / 'validation.json').write_text(json.dumps(result, indent=2) + '\n')
    print(json.dumps(result, indent=2))


if __name__ == '__main__':
    main()
