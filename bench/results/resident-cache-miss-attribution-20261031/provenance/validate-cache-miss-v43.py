#!/usr/bin/env python3
"""Replay actual bounded cache observations, never unique function miss totals."""
import json,re
from pathlib import Path
ROOT=Path('/home/jeremy/.jcode/scratch/blaze-native-engine-20261030')
result={'status':'passed','scope':'Actual overlapping native cache observations, not performance or unique function misses','native':[],'modes':{}}
for p in sorted((ROOT/'candidate-unit-v43/justrust-home/runs').glob('*/summary.json')):
 d=json.loads(p.read_text()); t=d['tests']; assert t['failed']==0 and t['ignored']==0
 result['native'].append({'run_id':p.parent.name,'passed':t['passed']})
assert sorted(x['passed'] for x in result['native'])==[53,360]
for mode in ('on','off'):
 folder=ROOT/f'cache-miss-attribution-v43-{mode}'
 state=json.loads((folder/'state.json').read_text()); assert state['status']=='passed' and state['unchanged'] and len(state['rows'])==7
 rows=[]
 for i,row in enumerate(state['rows']):
  assert row['accepted'] and row['failures']==[] and row['input_unchanged']
  reports=[]
  for log in sorted(folder.glob(f'{i:02}-*-worker-*.log')):
   for line in log.read_text().splitlines():
    if 'ra-jit cache-miss-stats v1 ' in line:
     fields=dict((k,int(v)) for k,v in re.findall(r'([a-z_]+)=(\d+)',line.split('ra-jit cache-miss-stats v1 ',1)[1]))
     assert len(fields)==19
     reports.append(fields)
  if mode=='on': assert reports and reports[-1]['direct_key_missing']>0
  else: assert not reports
  rows.append({'phase':row['phase'],'status':row['status'],'elapsed_seconds':row['elapsed_seconds'],'report_count_in_cumulative_log':len(reports),'last_report':reports[-1] if reports else None})
 result['modes'][mode]=rows
print(json.dumps(result,indent=2))
