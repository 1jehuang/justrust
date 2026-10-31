#!/usr/bin/env python3
"""Actual native-module-input inventory diagnostic, not persistent closure or performance proof."""
import json,re,sys
from pathlib import Path
sys.path.insert(0,'/home/jeremy/justrust/bench')
import resident_engine_compare as h
ROOT=Path('/home/jeremy/.jcode/scratch/blaze-native-engine-20261030')
OUT=ROOT/'demand-inventory-v36-native'
OUT.mkdir(mode=0o700,exist_ok=False)
SOURCE=Path('/home/jeremy/.jcode/scratch/resident-matrix-native-20261030/source63')
TC=Path('/home/jeremy/.blaze/toolchains/nightly-2026-10-27-x86_64-unknown-linux-gnu')
TOOL=ROOT/'candidate-v36-bin/jr-engine'
h.pinned_source(SOURCE)
pattern=re.compile(r'demand inventory: (\d+) compiled roots, (\d+) skipped roots, (\d+) declared instances, (\d+) found instances, (\d+) rejected instances, (\d+) kept statics, (\d+) data definitions \(full round\)')
state=dict(scope=__doc__,status='incomplete',source_before=h.source_hashes(SOURCE),toolchain_before=h.toolchain_identity(TC),tool_before=h.runtime.base.digest(TOOL),rows=[])
try:
 for mode,private in [('off',Path('/home/jeremy/g1')),('on',Path('/home/jeremy/g2'))]:
  private.mkdir(mode=0o700,exist_ok=False)
  env=h.environment(private,'engine',TC)
  env.update(RA_JIT_DEMAND_STD='1',RA_JIT_DEMAND_INVENTORY='1' if mode=='on' else '0')
  prefix=OUT/mode
  row=h.runtime.run_process(h.command(TOOL,'engine'),SOURCE,env,prefix,180)
  text='\n'.join(prefix.with_suffix('.'+s).read_text(errors='replace') for s in ('stdout','stderr'))
  logs=list(Path(env['BLAZE_HOME']).glob('workers/*/log'))
  if not logs:raise ValueError('missing real native worker log')
  all_logs=[]
  for i,p in enumerate(logs):
   data=p.read_bytes(); (OUT/f'{mode}-worker-{i}.log').write_bytes(data); all_logs.append(data.decode(errors='replace'))
  counts=[list(map(int,m.groups())) for m in pattern.finditer('\n'.join(all_logs))]
  row.update(mode=mode,startup_controls={'RA_JIT_DEMAND_STD':'1','RA_JIT_DEMAND_INVENTORY':env['RA_JIT_DEMAND_INVENTORY']},inventory_counts=counts,private_root=str(private),accepted=h.runtime.base.succeeded(row) and h.valid_tests(h.runtime.test_results(text)))
  state['rows'].append(row)
  if not row['accepted']:raise ValueError('exact four-test/link/native runtime gate failed')
  if mode=='off' and counts:raise ValueError('disabled inventory unexpectedly reported coverage')
  if mode=='on' and not any(c[0]>0 and c[2]>0 and c[6]>0 for c in counts):raise ValueError('no positive actual root/instance/data inventory')
 state['status']='passed'
except BaseException as exc:
 state.update(status='failed',error=repr(exc));raise
finally:
 state.update(source_after=h.source_hashes(SOURCE),toolchain_after=h.toolchain_identity(TC),tool_after=h.runtime.base.digest(TOOL))
 state['unchanged']=state['source_before']==state['source_after'] and state['toolchain_before']==state['toolchain_after'] and state['tool_before']==state['tool_after']
 (OUT/'state.json').write_text(json.dumps(state,indent=2)+'\n')
assert state['unchanged']
print(json.dumps({'status':state['status'],'rows':len(state['rows']),'scope':__doc__}))
