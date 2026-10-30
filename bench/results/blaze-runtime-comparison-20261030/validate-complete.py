#!/usr/bin/env python3
"""Replay final warm-workflow acceptance, not statistical or universal claims."""
import hashlib
import argparse
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


def exported(machine):
    return [json.loads(line) for p in (REPO / 'data/runs' / machine).glob('*.jsonl')
            for line in p.read_text().splitlines()]


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--case', choices=('br34', 'br35'), default='br34')
    case = parser.parse_args().case
    stack_count = 6 if case == 'br35' else 5
    directory = ROOT / case
    meta = load(directory / 'metadata.json')
    summary = load(directory / 'summary.json')
    rows = load(directory / 'records.json')
    assert meta['unchanged'] and meta['source_restored']
    assert meta['source_before'] == meta['source_after']
    assert meta['identities_before'] == meta['identities_after']
    assert meta['seed_source_before'] == meta['seed_source_after']
    assert meta['store_scope'] == summary['store_scope'] == 'warm-store seeded'
    assert len(set(meta['wild_copy_hashes_after'].values())) == 1
    assert all(v['sha256'] == meta['wild_copy_hashes_after'][k]
               for k, v in meta['wild_copies'].items())
    assert all(v['files'] == meta['seed_source_before']
               for v in meta['blaze_seed_manifests'].values())
    assert meta['compiler_version']['nightly']
    assert meta['compiler_version']['sha256_before'] == meta['compiler_version']['sha256_after']
    assert summary['excluded'] == [] and len(rows) == stack_count * 10
    assert len(summary['medians_seconds']) == stack_count
    expected = {**meta['expected_tests'],
                'tests': [tuple(x) for x in meta['expected_tests']['tests']]}
    exports = 0
    for stack in summary['medians_seconds']:
        group = [r for r in rows if r['stack'] == stack]
        assert len(group) == 10
        edits = [r for r in group if r['phase'] == 'edit']
        assert len(edits) == 5
        assert statistics.median(r['elapsed_seconds'] for r in edits) == summary['medians_seconds'][stack]
        for row in group:
            assert not row['error'] and not row['timed_out']
            prefix = f"{row['phase']}-{row['replicate']:03d}-{stack}"
            if stack.startswith('blaze'):
                assert row['environment']['BLAZE_BACKEND'] == ('llvm' if stack == 'blaze-llvm' else 'cranelift')
                assert row['environment']['BLAZE_OPT'] == '0'
                output = ''.join((directory / (prefix + '.' + s)).read_text() for s in ('stdout', 'stderr'))
            else:
                receipt = row['raw_output']
                raw = (directory / receipt['artifact']).read_bytes()
                assert hashlib.sha256(raw).hexdigest() == receipt['sha256']
                output = '\n'.join(json.loads(line)['l'] for line in raw.decode().splitlines())
                hp = receipt['hotpatch']
                if hp['present']:
                    data = (directory / hp['artifact']).read_bytes()
                    assert hashlib.sha256(data).hexdigest() == hp['sha256']
            if row['phase'] in ('type-error', 'test-failure'):
                assert row['status'] == 101 and row['gate_passed']
            else:
                marker = f"JR_RUNTIME_EXECUTED_{row['replicate'] + 1}" if row['phase'] == 'edit' else None
                assert harness.accepted(row, output, expected, marker)
            if row['phase'] == 'edit':
                assert row['accepted']
        if not stack.startswith('blaze'):
            name = 'owned-on' if stack == 'all3on' else stack
            records = exported(f'xps14-runtime-{case}-{name}-20261030')
            assert len(records) == 10
            assert {r['raw_output']['run_id'] for r in group} == {r['id'] for r in records}
            exports += len(records)
    for rep in range(5):
        assert len({r['source_sha256'] for r in rows if r['phase'] == 'edit' and r['replicate'] == rep}) == 1
    assert summary['edit_hotpatch_outcomes']['all4hotpatch'] == ['patched'] * 5
    suite_exports = exported('xps14-native-suite-20261030')
    suite_ids = {r['id'] for r in suite_exports}
    assert len(suite_exports) == 16
    for name in ('dead-code-suite-20261030', 'hotpatch-suite-20261030'):
        base = REPO / 'bench/results' / name
        state = load(base / 'state.json')
        assert state['status'] == 'completed'
        assert state['artifacts_before'] == state['artifacts_after']
        assert state['client_sha256'] == state['client_sha256_after']
        for mode in ('no', 'yes'):
            report = load(base / (mode + '.json'))
            samples = report['scenarios']['justrust-body-test']['samples']
            assert len(samples) == 3
            assert all(s['tests_passed'] == 157 and s['tests_failed'] == 0 and s['id'] in suite_ids for s in samples)
        assert 'uncontrolled (noisy or unknown samples)' in (base / 'compare.stdout').read_text()
    for name, sha in load(ROOT / 'complete-evidence-sha256.json').items():
        assert hashlib.sha256((REPO / name).read_bytes()).hexdigest() == sha, name
    result = {'verdict': 'passed', 'scope': 'final seeded warm workflow only, not cold support or significance',
              'case': case, 'accepted_stacks': stack_count, 'matched_edits': stack_count * 5,
              'correctness_gate_stacks': stack_count,
              'exported_runtime_calls': exports, 'exported_suite_calls': len(suite_exports),
              'patched_timed_edits': 5, 'defaults_promoted': False}
    output = 'validation-complete.json' if case == 'br34' else 'validation-br35.json'
    (ROOT / output).write_text(json.dumps(result, indent=2) + '\n')
    print(json.dumps(result, indent=2))


if __name__ == '__main__':
    main()
