#!/usr/bin/env python3
"""Read-only audit of preserved A2 evidence. Does not run builds or benchmarks."""
import collections
import hashlib
import json
from pathlib import Path
import sys

ROOT = Path(__file__).resolve().parents[2]
sys.path.insert(0, str(ROOT / 'bench'))
import suite


def digest(path):
    return hashlib.sha256(path.read_bytes()).hexdigest()


def audit(home):
    result_path = ROOT / 'bench/results/native-dispatch-synth40-A2-20261015.json'
    result = json.loads(result_path.read_text())
    engine, _ = suite._synth()
    synth = result['synth']
    failures = []
    for point in synth['results']:
        for samples in point['samples'].values():
            for sample in samples:
                if not sample['tests_failed']:
                    continue
                output = home / 'runs' / sample['id'] / 'output.jsonl'
                lines = [json.loads(line)['l'] for line in output.read_text().splitlines()]
                failures.append({
                    'point': point['i'], 'run_id': sample['id'],
                    'expect_fail': point['expect_fail'], 'exit': sample['exit'],
                    'compile_failed': sample['compile_failed'], 'tests_failed': sample['tests_failed'],
                    'saved_output_sha256': digest(output), 'saved_output_rows': len(lines),
                    'failed_test_lines': [line for line in lines if ' ... FAILED' in line],
                    'panic_context': [lines[i:i+5] for i, line in enumerate(lines) if 'panicked at' in line],
                    'failed_result_lines': [line for line in lines if 'test result: FAILED.' in line],
                })
    samples = [sample for point in synth['results'] for group in point['samples'].values() for sample in group]
    invocations = result['toolchain_identity']['invocations']
    metadata_issues = []
    for invocation in invocations:
        path = home / 'runs' / invocation['id'] / 'meta.json'
        if digest(path) != invocation['meta_sha256']:
            metadata_issues.append(invocation['id'])
    exports = [json.loads(line) for path in (ROOT / 'data/runs/dispatch-synth40-A2').glob('*.jsonl') for line in path.read_text().splitlines()]
    missing = sorted({entry['id'] for entry in invocations} - {entry['id'] for entry in exports})
    return {
        'schema': 1, 'scope': 'preserved baseline evidence audit only, no performance claim',
        'baseline_sha256': digest(result_path),
        'measurement_plan_sha256': digest(home.parent.parent / 'measurement-plan.json'),
        'eligibility_implementation_sha256': digest(ROOT / 'bench/synth/run.py'),
        'comparison_gate': 'blocked: baseline attempted, not eligible; B2 not run',
        'points_observed': len(synth['results']), 'comprehensively_eligible_points': len(engine._complete(synth)),
        'noisy_samples': sum(sample['noisy'] is True for sample in samples),
        'quiet_points': [point['i'] for point in synth['results'] if all(sample['noisy'] is False for group in point['samples'].values() for sample in group)],
        'outcome_issue_points': [entry['point'] for entry in failures],
        'expected_compile_error_points': [point['i'] for point in synth['results'] if point['expect_fail']],
        'measured_foreign_rustc_counts': sorted({sample['foreign_rustc_before'] for sample in samples}),
        'load_before_range': [min(sample['load_before'] for sample in samples), max(sample['load_before'] for sample in samples)],
        'suite_blockers': engine._blocked(synth),
        'identity_self_consistency_issues': suite.toolchain_identity_comparison_issues(result, result),
        'identity_invocations': dict(collections.Counter(entry['stage'] for entry in invocations)),
        'identity_invocation_count': len(invocations), 'identity_metadata_hash_mismatch_ids': metadata_issues,
        'exported_run_count': len(exports), 'missing_exported_identity_run_ids': missing,
        'outcome_log_audits': failures,
        'limitations': ['Self-consistency is not a matched A/B comparison.', 'Panic contexts do not establish compiler causality or a complete root cause.', 'No noisy points, expected failures, or test failures were rewritten or removed.'],
    }


if __name__ == '__main__':
    print(json.dumps(audit(Path(sys.argv[1])), indent=2))
