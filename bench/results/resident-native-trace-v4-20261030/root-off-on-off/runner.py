#!/usr/bin/env python3
"""Actual native trace and identical-source off/on/off gate, not timing evidence."""
import hashlib,json,os,sys
from pathlib import Path
sys.path.insert(0,'/home/jeremy/justrust/bench')
import resident_engine_compare as h
import blaze_runtime_compare as r
OUT=Path('/home/jeremy/.jcode/scratch/resident-trace-v4-20261030')
OUT.mkdir(mode=0o700,exist_ok=False)
BASE=Path('/home/jeremy/rsa01')
meta=json.loads((BASE/'metadata.json').read_text())
src=Path('/home/jeremy/.jcode/scratch/ba30/source/src/summary.rs')
original=src.read_bytes()
env={k:v for k,v in os.environ.items() if not k.startswith(('JUSTRUST_','JR_ENGINE_','RA_JIT_','BLAZE_','CARGO_','RUST','XDG_'))}
env.update(meta['environments']['engine'])
for var,name in [('RUNTIME','runtime'),('CACHE','cache'),('CONFIG','config'),('DATA','data')]:
 env['XDG_'+var+'_HOME' if var!='RUNTIME' else 'XDG_RUNTIME_DIR']=str(BASE/'engine'/name)
tool=Path(meta['tools_before']['engine']['path'])
state={'scope':'actual-native-telemetry-not-speedup','rows':[],'tool_before':h.runtime.base.digest(tool)}
def artifacts():
 result={}
 for p in (BASE/'engine').rglob('*'):
  if p.is_file() and not p.is_symlink() and p.stat().st_mode&0o111:
   with p.open('rb') as f: magic=f.read(4)
   if magic==b'\x7fELF':
    st=p.stat();result[str(p.relative_to(BASE/'engine'))]={'sha256':r.base.digest(p),'dev':st.st_dev,'inode':st.st_ino}
 return result
try:
 for i,mode in enumerate(('settle','off','trace','alias','off-again')):
  e=env.copy()
  trace=OUT/f'{mode}.trace.json'
  if mode=='trace':e.update(RA_JIT_STATS='1',RA_JIT_TRACE=str(trace))
  if mode=='alias':e.update(BLAZE_STATS='1',BLAZE_TRACE=str(trace))
  row=r.run_process(h.command(tool,'engine'),src.parent.parent,e,OUT/mode,60)
  text='\n'.join((OUT/f'{mode}.{s}').read_text() for s in ('stdout','stderr'))
  if not r.base.succeeded(row) or not h.valid_tests(r.test_results(text)):raise ValueError(f'{mode}: runtime gate failed')
  row.update(mode=mode,artifacts=artifacts())
  if mode in ('trace','alias'):
   data=json.loads(trace.read_text()); events=data['traceEvents']
   names=sorted({x.get('name','') for x in events})
   if not events or not any('test' in n.lower() for n in names):raise ValueError('missing actual test trace phases')
   row.update(trace_sha256=r.base.digest(trace),phase_names=names)
  elif trace.exists():raise ValueError('disabled telemetry produced trace')
  state['rows'].append(row)
  if mode=='off':baseline=row['artifacts']
  elif mode in ('trace','alias','off-again') and row['artifacts']!=baseline:raise ValueError(f'{mode}: artifact identity changed')
 state.update(status='passed',tool_after=r.base.digest(tool))
 if state['tool_before']!=state['tool_after']:raise ValueError('tool changed')
except BaseException as error:
 state.update(status='failed',error=repr(error));raise
finally:
 src.write_bytes(original)
 state['source_restored']=src.read_bytes()==original
 (OUT/'state.json').write_text(json.dumps(state,indent=2)+'\n')
print(json.dumps({'status':state['status'],'rows':len(state['rows']),'phase_names':[x.get('phase_names') for x in state['rows']]}))
