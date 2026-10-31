#!/usr/bin/env python3
"""Actual cold native statistics, diagnostic only, not a performance comparison."""
import json,sys
from pathlib import Path
sys.path.insert(0,'/home/jeremy/justrust/bench')
import resident_engine_compare as h
ROOT=Path('/home/jeremy/.jcode/scratch/blaze-native-engine-20261030')
OUT=ROOT/'cold-path-v30-statistics'
OUT.mkdir(mode=0o700,exist_ok=False)
SOURCE=Path('/home/jeremy/.jcode/scratch/resident-matrix-native-20261030/source63')
TC=Path('/home/jeremy/.blaze/toolchains/nightly-2026-10-27-x86_64-unknown-linux-gnu')
h.pinned_source(SOURCE)
tools={'reference':ROOT/'reference-bin/ra-jit','engine':ROOT/'candidate-v30-bin/jr-engine'}
state={'scope':__doc__,'source_before':h.source_hashes(SOURCE),'toolchain_before':h.toolchain_identity(TC),'tools_before':{k:h.runtime.base.digest(v) for k,v in tools.items()},'rows':[],'status':'incomplete'}
try:
 for stack,private in [('reference',Path('/home/jeremy/c1')),('engine',Path('/home/jeremy/c2'))]:
  private.mkdir(mode=0o700,exist_ok=False)
  env=h.environment(private,stack,TC)
  env['RA_JIT_STATS']='1'
  prefix=OUT/stack
  row=h.runtime.run_process(h.command(tools[stack],stack),SOURCE,env,prefix,180)
  text='\n'.join(prefix.with_suffix('.'+s).read_text(errors='replace') for s in ('stdout','stderr'))
  row.update(stack=stack,accepted=h.runtime.base.succeeded(row) and h.valid_tests(h.runtime.test_results(text)))
  state['rows'].append(row)
  logs=list(Path(env['BLAZE_HOME']).glob('workers/*/log'))
  if not logs: raise ValueError('no actual cold worker log')
  for i,p in enumerate(logs):
   raw=p.read_bytes();(OUT/f'{stack}-worker-{i}.log').write_bytes(raw)
  state.setdefault('private_namespaces',{})[stack]={'root':str(private),'home':env.get('JR_ENGINE_HOME',env['BLAZE_HOME'])}
  if not row['accepted']:raise ValueError('actual cold four-test gate failed')
 state['status']='passed'
finally:
 state.update(source_after=h.source_hashes(SOURCE),toolchain_after=h.toolchain_identity(TC),tools_after={k:h.runtime.base.digest(v) for k,v in tools.items()})
 state['unchanged']=state['source_before']==state['source_after'] and state['toolchain_before']==state['toolchain_after'] and state['tools_before']==state['tools_after']
 (OUT/'state.json').write_text(json.dumps(state,indent=2)+'\n')
assert state['unchanged']
print(json.dumps({'status':state['status'],'rows':len(state['rows']),'scope':__doc__}))
