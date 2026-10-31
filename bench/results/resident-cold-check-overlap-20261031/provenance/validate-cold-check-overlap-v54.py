#!/usr/bin/env python3
"""Strict same-binary cold-check overlap OFF/ON scoped replay, not comprehensive performance acceptance."""
import json
from pathlib import Path
ROOT = Path(__file__).resolve().parent
cases = ('off','on','on-repeat','off-repeat')
states = [json.loads((ROOT/('cold-check-overlap-v54-'+case)/'state.json').read_text()) for case in cases]
phases = ('cold-prepare','repeated-noop-1','manifest-feature-add','type-error','type-recovery','assert-failure','assert-recovery')
marker = b'cold-check-overlap: asynchronous diagnostics started'
for case,state in zip(cases,states):
    assert state['status']=='passed' and state['unchanged']
    assert tuple(row['phase'] for row in state['rows'])==phases
    assert state['source_before']==state['source_after']==states[0]['source_before']
    assert state['tool_before']==state['tool_after']==states[0]['tool_before']
    assert state['toolchain_before']==state['toolchain_after']==states[0]['toolchain_before']
    assert state['startup_controls']['RA_JIT_COLD_CHECK_OVERLAP']==('1' if case.startswith('on') else '0')
    assert state['startup_controls']['RA_JIT_DEMAND_STD']=='1'
    assert all(row['accepted'] and row['input_unchanged'] and not row['failures'] for row in state['rows'])
    logs=list((ROOT/('cold-check-overlap-v54-'+case)).glob('*-worker-*.log'))
    assert logs
    counts=[log.read_bytes().count(marker) for log in sorted(logs)]
    if case.startswith('on'):
        assert any(counts), 'positive actual asynchronous diagnostic startup marker required'
    else:
        assert not any(counts), 'disabled overlap must emit no positive marker'
    state['_snapshot_marker_counts']=counts
rows=[]
for i,phase in enumerate(phases):
    times=[state['rows'][i]['elapsed_seconds'] for state in states]
    rows.append(dict(phase=phase,seconds=dict(zip(cases,times)),first_percent=100*(times[1]/times[0]-1),reverse_percent=100*(times[2]/times[3]-1)))
print(json.dumps(dict(status='passed',strict_rows=28,noise='unknown',scope=__doc__,marker_counts={case:state['_snapshot_marker_counts'] for case,state in zip(cases,states)},rows=rows),indent=2))
