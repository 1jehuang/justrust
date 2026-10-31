#!/usr/bin/env python3
"""Exact original/OFF/ON fixture diagnostic replay, not performance proof."""
import json
from pathlib import Path
ROOT=Path('/home/jeremy/.jcode/scratch/blaze-native-engine-20261030')
p=ROOT/'cold-check-overlap-v54-fixture-observations';s=json.loads((p/'state.json').read_text())
assert s['status']=='collected' and len(s['rows'])==30 and s['source_unchanged_except_new_lock'] and s['identities_unchanged']
rows=[]
for case in dict.fromkeys(r['case'] for r in s['rows']):
 group=[r for r in s['rows'] if r['case']==case];assert [r['mode'] for r in group]==['original','off','on']
 observed=[]
 for r in group:
  assert r['complete'] and not r['timed_out'] and not r.get('error') and r['post_observation_stop']['status']==0
  prefix=p/(case+'-'+r['mode']+'-raw');out=prefix.with_suffix('.stdout').read_bytes();err=prefix.with_suffix('.stderr').read_bytes()
  # Preserve the entire actual diagnostic block, with no message/location normalization.
  diagnostic=err[err.index(b'error'):err.index(b'blaze time:')] if b'error' in err else b''
  counts=[(p/x).read_bytes().count(b'cold-check-overlap: asynchronous diagnostics started') for x in r['worker_logs']]
  assert sum(counts)==(1 if r['mode']=='on' else 0),(case,r['mode'],counts)
  assert r['status']==(0 if case in ('valid','include_valid') else 101)
  if not diagnostic: assert b'Running `' in err
  observed.append((r['status'],out,diagnostic))
 assert observed[0]==observed[1]==observed[2],(case,'complete original/OFF/ON diagnostics differ')
 rows.append({'case':case,'status':observed[0][0],'complete_diagnostics_identical':True,'activation_controls_passed':True})
print(json.dumps({'status':'passed','scope':__doc__,'rows':30,'cases':rows,'limitations':'No injected panic/unset/procmacro failure proof, no long-lived include recovery or lint/borrow alternate policy coverage in this fixture batch.'},indent=2))
