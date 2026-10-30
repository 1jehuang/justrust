#!/usr/bin/env python3
import json,os,sys
from pathlib import Path
sys.path.insert(0,'/home/jeremy/justrust/bench')
import resident_engine_compare as h
import blaze_runtime_compare as r
OUT=Path('/home/jeremy/.jcode/scratch/resident-trace-errors-v4-20261030');OUT.mkdir(mode=0o700,exist_ok=False)
BASE=Path('/home/jeremy/rsa01');meta=json.loads((BASE/'metadata.json').read_text())
src=Path('/home/jeremy/.jcode/scratch/ba30/source/src/summary.rs');original=src.read_bytes()
env={k:v for k,v in os.environ.items() if not k.startswith(('JUSTRUST_','JR_ENGINE_','RA_JIT_','BLAZE_','CARGO_','RUST','XDG_'))};env.update(meta['environments']['engine'])
for var,name in [('RUNTIME','runtime'),('CACHE','cache'),('CONFIG','config'),('DATA','data')]:env['XDG_'+var+'_HOME' if var!='RUNTIME' else 'XDG_RUNTIME_DIR']=str(BASE/'engine'/name)
tool=Path(meta['tools_before']['engine']['path']);state={'rows':[],'scope':'trace-errors-recovery-and-actual-edit-not-speedup'}
try:
 for mode,kind in [('body','body'),('type-error','type-error'),('type-recovery',None),('assert-failure','test-failure'),('assert-recovery',None)]:
  contents=r.edit_bytes(original,300) if kind=='body' else r.edit_bytes(original,kind=kind) if kind else original
  src.write_bytes(contents);e=env.copy();trace=OUT/f'{mode}.json';e.update(RA_JIT_STATS='1',RA_JIT_TRACE=str(trace))
  row=r.run_process(h.command(tool,'engine'),src.parent.parent,e,OUT/mode,60)
  text='\n'.join((OUT/f'{mode}.{s}').read_text() for s in ('stdout','stderr'))
  if kind=='type-error':ok=row['status']==101 and 'E0308' in text and 'JR_RUNTIME_TYPE_ERROR' in text
  elif kind=='test-failure':
   tests=r.test_results(text);ok=row['status']==101 and tests['failed']==1 and 'JR_RUNTIME_FAILURE' in text
  else:ok=r.base.succeeded(row) and h.valid_tests(r.test_results(text))
  if not ok:raise ValueError(f'{mode}: original status/diagnostics/runtime failed {row}')
  data=json.loads(trace.read_text());names=sorted({x.get('name','') for x in data['traceEvents']})
  if not names or 'client command' not in names:raise ValueError('missing finalized trace')
  if kind=='body' and text.count('JR_RUNTIME_EXECUTED_301')!=1:raise ValueError('missing exact edited runtime marker')
  row.update(mode=mode,phase_names=names,trace_sha256=r.base.digest(trace));state['rows'].append(row)
 state['status']='passed'
except BaseException as error:state.update(status='failed',error=repr(error));raise
finally:
 src.write_bytes(original);state['source_restored']=src.read_bytes()==original
 (OUT/'state.json').write_text(json.dumps(state,indent=2)+'\n')
print(json.dumps(state,indent=2))
