#!/usr/bin/env python3
"""Read-only scoped v50 state replay, not a comprehensive performance gate."""
import json
from pathlib import Path
ROOT = Path(__file__).resolve().parent
names = ('baseline', 'candidate', 'candidate-repeat', 'baseline-repeat')
states = [json.loads((ROOT / ('linear-adjacency-v50-' + n) / 'state.json').read_text()) for n in names]
phases = ('cold-prepare', 'repeated-noop-1', 'manifest-feature-add', 'type-error', 'type-recovery', 'assert-failure', 'assert-recovery')
for state in states:
    assert state['status'] == 'passed' and state['unchanged']
    assert tuple(row['phase'] for row in state['rows']) == phases
    assert all(row['accepted'] and row['input_unchanged'] and not row['failures'] for row in state['rows'])
    assert state['source_before'] == state['source_after']
    assert state['tool_before'] == state['tool_after']
    assert state['toolchain_before'] == state['toolchain_after']
rows = []
for i, phase in enumerate(phases):
    times = [state['rows'][i]['elapsed_seconds'] for state in states]
    rows.append(dict(phase=phase, seconds=dict(zip(names, times)), first_percent=100*(times[1]/times[0]-1), reverse_percent=100*(times[2]/times[3]-1)))
print(json.dumps(dict(status='passed', strict_rows=28, scope='Scoped counterordered native states, unknown noise, not comprehensive/default promotion', performance='Mixed transitions, assertion recovery faster in both pairs, attribution not established', rows=rows), indent=2))
