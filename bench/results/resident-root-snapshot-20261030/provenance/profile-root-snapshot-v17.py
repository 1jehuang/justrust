#!/usr/bin/env python3
"""Matched instrumented native planning observations, not a timing gate."""
import hashlib,json,os,sys
from pathlib import Path
sys.path.insert(0,'/home/jeremy/justrust/bench')
import resident_engine_compare as h
import blaze_runtime_compare as r
OUT=Path('/home/jeremy/.jcode/scratch/resident-root-snapshot-profile-v17-20261030')
OUT.mkdir(mode=0o700,exist_ok=False)
src=Path('/home/jeremy/.jcode/scratch/ba30/source/src/summary.rs')
original=src.read_bytes();(OUT/'original.rs').write_bytes(original)
state={'scope':'matched-instrumented-native-roots-not-speedup','rows':[],'source_before':hashlib.sha256(original).hexdigest()}
configs={}
for name,base in [('old','/home/jeremy/rsb06'),('new','/home/jeremy/rsa12')]:
 base=Path(base);meta=json.loads((base/'metadata.json').read_text())
 env={k:v for k,v in os.environ.items() if not k.startswith(('JUSTRUST_','JR_ENGINE_','RA_JIT_','BLAZE_','CARGO_','RUST','XDG_'))}
 env.update(meta['environments']['engine'])
 for var,folder in [('RUNTIME','runtime'),('CACHE','cache'),('CONFIG','config'),('DATA','data')]:env['XDG_'+var+'_HOME' if var!='RUNTIME' else 'XDG_RUNTIME_DIR']=str(base/'engine'/folder)
 configs[name]=(Path(meta['tools_before']['engine']['path']),env,meta['tools_before']['engine']['sha256'])
def run(name,rep,marker=None):
 tool,env,identity=configs[name]
 if r.base.digest(tool)!=identity:raise ValueError('tool changed')
 prefix=OUT/f'{name}-{rep}';trace=prefix.with_suffix('.trace.json');env=dict(env,RA_JIT_STATS='1',RA_JIT_TRACE=str(trace))
 row=r.run_process(h.command(tool,'engine'),src.parent.parent,env,prefix,120)
 text='\n'.join(prefix.with_suffix('.'+suffix).read_text() for suffix in ('stdout','stderr'))
 if not r.base.succeeded(row) or not h.valid_tests(r.test_results(text)):raise ValueError('runtime gate failed')
 if marker is not None and text.count(marker)!=1:raise ValueError('fresh runtime marker absent')
 data=json.loads(trace.read_text());events=data['traceEvents']
 if not any(e.get('name')=='client planning' for e in events):raise ValueError('planning trace absent')
 selected={}
 for event in events:
  if event.get('ph')=='X' and event.get('name') in ('client planning','client metadata','client command','apply change','roots'):
   selected.setdefault(event['name'],[]).append(event.get('dur'))
 row.update(stack=name,replicate=rep,trace_sha256=r.base.digest(trace),selected_nested_elapsed_us=selected,tool_sha256=identity)
 state['rows'].append(row)
try:
 for name in configs:run(name,'warmup')
 for rep in range(5):
  src.write_bytes(r.edit_bytes(original,rep+500))
  for name in (('old','new') if rep%2==0 else ('new','old')):run(name,rep,f'JR_RUNTIME_EXECUTED_{rep+501}')
 state['status']='passed'
except BaseException as error:
 state.update(status='failed',error=repr(error));raise
finally:
 src.write_bytes(original);state['source_restored']=src.read_bytes()==original
 state['identities_unchanged']=all(r.base.digest(tool)==identity for tool,env,identity in configs.values())
 (OUT/'state.json').write_text(json.dumps(state,indent=2)+'\n')
print(json.dumps({'status':state['status'],'rows':len(state['rows']),'selected':[{'stack':x['stack'],'replicate':x['replicate'],'intervals':x['selected_nested_elapsed_us']} for x in state['rows']]},indent=2))
