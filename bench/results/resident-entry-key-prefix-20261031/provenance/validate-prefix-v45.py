#!/usr/bin/env python3
"""Recompute scoped v45 activation admission and counterordered state observations."""
import json
from pathlib import Path
ROOT = Path('/home/jeremy/.jcode/scratch/blaze-native-engine-20261030')
observations = {}
for mode in ('off', 'on', 'on-repeat', 'off-repeat'):
    directory = ROOT / f'entry-key-prefix-v45-{mode}'
    state = json.loads((directory / 'state.json').read_text())
    assert state['status'] == 'passed' and state['unchanged']
    assert len(state['rows']) == 7
    assert all(row['accepted'] and row['input_unchanged'] and not row['failures'] for row in state['rows'])
    counts = [sum(path.read_text().count('ra-jit entry-key-prefix v1 prepared') for path in directory.glob(f'{index:02}-*-worker-*.log')) for index in range(7)]
    assert all(count > 0 for count in counts) if mode.startswith('on') else not any(counts)
    observations[mode] = {'activation_marker_counts': counts, 'rows': [{'phase': row['phase'], 'seconds': row['elapsed_seconds']} for row in state['rows']]}
comparisons = []
for index, row in enumerate(observations['off']['rows']):
    values = {mode: state['rows'][index]['seconds'] for mode, state in observations.items()}
    comparisons.append({'phase': row['phase'], 'seconds': values, 'first_on_relative_to_off_percent': 100 * (values['on'] / values['off'] - 1), 'repeat_on_relative_to_off_percent': 100 * (values['on-repeat'] / values['off-repeat'] - 1)})
print(json.dumps({'status': 'passed', 'decision': 'REJECTED', 'promotion': False, 'scope': 'Scoped counterordered seven-state diagnostic, unknown noise, no significance/general/default speed claim. Marker counts are cumulative snapshots, not fresh constructor counts per phase.', 'observations': observations, 'comparisons': comparisons}, indent=2))
