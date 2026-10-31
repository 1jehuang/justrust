#!/usr/bin/env python3
"""Scoped native scope experiment posthoc replay, not a comprehensive performance gate."""
import json
from pathlib import Path
ROOT = Path('/home/jeremy/.jcode/scratch/blaze-native-engine-20261030')
cases = ('block-scopes-v47-baseline', 'block-scopes-v49-candidate', 'block-scopes-v49-candidate-repeat', 'block-scopes-v49-baseline-repeat')
states = {case: json.loads((ROOT / case / 'state.json').read_text()) for case in cases}
phases = ('cold-prepare', 'repeated-noop-1', 'manifest-feature-add', 'type-error', 'type-recovery', 'assert-failure', 'assert-recovery')
for state in states.values():
    assert state['status'] == 'passed' and state['unchanged']
    assert tuple(row['phase'] for row in state['rows']) == phases
    assert all(row['accepted'] and row['input_unchanged'] and not row['failures'] for row in state['rows'])
    assert state['source_before'] == state['source_after']
    assert state['tool_before'] == state['tool_after']
    assert state['toolchain_before'] == state['toolchain_after']
assert len({json.dumps(state['source_before'], sort_keys=True) for state in states.values()}) == 1
assert len({json.dumps(state['toolchain_before'], sort_keys=True) for state in states.values()}) == 1
assert len({json.dumps(state['startup_controls'], sort_keys=True) for state in states.values()}) == 1
comparisons = []
for index, phase in enumerate(phases):
    times = {case: states[case]['rows'][index]['elapsed_seconds'] for case in cases}
    comparisons.append({'phase': phase, 'seconds': times, 'first_candidate_relative_percent': 100 * (times[cases[1]] / times[cases[0]] - 1), 'reverse_candidate_relative_percent': 100 * (times[cases[2]] / times[cases[3]] - 1)})
print(json.dumps({'status': 'passed', 'decision': 'REJECTED', 'promotion': False, 'raw_rows': 28, 'noise': 'unknown', 'comparisons': comparisons, 'limitations': ['First baseline preceded candidate build by approximately24minutes, not a source-isolated attribution experiment.', 'Two scoped state pairs establish no significance, session-weighted default promotion or general speed claim.', 'Native correctness/provider fixtures passed, but runtime observations do not establish repeatable useful latency benefit.']}, indent=2))
