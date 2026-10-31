#!/usr/bin/env python3
"""Opt-in demand standard-library cold correctness and attribution diagnostic only."""
import json,sys
from pathlib import Path
sys.path.insert(0,'/home/jeremy/justrust/bench')
import resident_engine_compare as h
ROOT=Path('/home/jeremy/.jcode/scratch/blaze-native-engine-20261030')
OUT=ROOT/'demand-std-v32-native'
OUT.mkdir(mode=0o700,exist_ok=False)
SOURCE=Path('/home/jeremy/.jcode/scratch/resident-matrix-native-20261030/source63')
TC=Path('/home/jeremy/.blaze/toolchains/nightly-2026-10-27-x86_64-unknown-linux-gnu')
TOOL=ROOT/'candidate-v32-bin/jr-engine'
h.pinned_source(SOURCE)
state={'scope':__doc__,'source_before':h.source_hashes(SOURCE),'toolchain_before':h.toolchain_identity(TC),'tool_before':h.runtime.base.digest(TOOL),'rows':[],'status':'incomplete'}
try:
 for mode,private in [('off',Path('/home/jeremy/e1')),('on',Path('/home/jeremy/e2'))]:
  private.mkdir(mode=0o700,exist_ok=False)
  env=h.environment(private,'engine',TC)
  env.update(RA_JIT_STATS='1',RA_JIT_DEMAND_STD='1' if mode=='on' else '0')
  prefix=OUT/mode
  row=h.runtime.run_process(h.command(TOOL,'engine'),SOURCE,env,prefix,180)
  text='\n'.join(prefix.with_suffix('.'+s).read_text(errors='replace') for s in ('stdout','stderr'))
  row.update(mode=mode,accepted=h.runtime.base.succeeded(row) and h.valid_tests(h.runtime.test_results(text)))
  state['rows'].append(row)
  logs=list(Path(env['BLAZE_HOME']).glob('workers/*/log'))
  if not logs:raise ValueError('no actual cold worker log')
  for i,p in enumerate(logs):(OUT/f'{mode}-worker-{i}.log').write_bytes(p.read_bytes())
  state.setdefault('private_namespaces',{})[mode]={'root':str(private),'home':env['JR_ENGINE_HOME']}
  if not row['accepted']:raise ValueError('opt-in cold four-test/link correctness gate failed')
 state['status']='passed'
except BaseException as exc:
 state.update(status='failed',error=repr(exc));raise
finally:
 state.update(source_after=h.source_hashes(SOURCE),toolchain_after=h.toolchain_identity(TC),tool_after=h.runtime.base.digest(TOOL))
 state['unchanged']=state['source_before']==state['source_after'] and state['toolchain_before']==state['toolchain_after'] and state['tool_before']==state['tool_after']
 (OUT/'state.json').write_text(json.dumps(state,indent=2)+'\n')
assert state['unchanged']
print(json.dumps({'status':state['status'],'rows':len(state['rows']),'scope':__doc__}))
