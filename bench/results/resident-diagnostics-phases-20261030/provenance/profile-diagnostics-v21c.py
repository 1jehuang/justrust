#!/usr/bin/env python3
"""Frozen native diagnostic spans, correctness/provenance only, not speedup."""
import hashlib,json,os,sys
from pathlib import Path
sys.path.insert(0,'/home/jeremy/justrust/bench')
import resident_engine_compare as h
import blaze_runtime_compare as r
ROOT=Path('/home/jeremy/.jcode/scratch/blaze-native-engine-20261030')
OUT=Path('/home/jeremy/.jcode/scratch/resident-diagnostics-phases-v21c-20261030')
OUT.mkdir(mode=0o700,exist_ok=False)
SOURCE=Path('/home/jeremy/.jcode/scratch/ba30/source')
src=SOURCE/'src/summary.rs'
original=src.read_bytes()
(OUT/'original.rs').write_bytes(original)
meta=json.loads(Path('/home/jeremy/rsa18/metadata.json').read_text())
tool=Path(meta['tools_before']['engine']['path'])
h.pinned_source(SOURCE)
state=dict(scope='actual diagnostics intervals, inclusive and concurrent, not saved-time or critical-path evidence',rows=[],source_before=h.source_hashes(SOURCE),tool_before=r.base.digest(tool),toolchain_before=h.toolchain_identity(Path('/home/jeremy/.blaze/toolchains/nightly-2026-10-27-x86_64-unknown-linux-gnu')))
env={k:v for k,v in os.environ.items() if not k.startswith(('JUSTRUST_','JR_ENGINE_','RA_JIT_','BLAZE_','CARGO_','RUST','XDG_'))}
env.update(meta['environments']['engine'])
env['RA_JIT_DIAGNOSTICS_STATS']='1'
state['worker_startup_opt_in']={'RA_JIT_DIAGNOSTICS_STATS':'1','scope':'dedicated daemon and inherited worker startup, not dynamic client telemetry toggle'}
for var,name in [('RUNTIME','runtime'),('CACHE','cache'),('CONFIG','config'),('DATA','data')]:
 env['XDG_'+var+'_HOME' if var!='RUNTIME' else 'XDG_RUNTIME_DIR']=str(Path('/home/jeremy/rsa18/engine')/name)
assert meta['status']=='complete' and meta['source_restored'] and meta['unchanged']
assert tool.is_relative_to(ROOT) and not tool.is_symlink() and r.base.digest(tool)==meta['tools_before']['engine']['sha256']
assert Path(env['JR_ENGINE_HOME']).resolve().is_relative_to(Path('/home/jeremy/rsa18/engine').resolve())
stop=r.run_process([str(tool),'stop'],SOURCE,env,OUT/'restart-private-daemon',30)
state['private_daemon_stop']=stop
if not r.base.succeeded(stop):raise ValueError('private measurement daemon stop failed')
try:
 for rep,mode in enumerate(('off','on','off-again','edit-one','edit-two','edit-three')):
  contents=r.edit_bytes(original,900+(0 if mode in ('off','on','off-again') else rep))
  src.write_bytes(contents)
  e=env.copy();trace=OUT/f'{mode}.trace.json'
  if mode not in ('off','off-again'):e.update(RA_JIT_STATS='1',RA_JIT_TRACE=str(trace))
  row=r.run_process(h.command(tool,'engine'),SOURCE,e,OUT/mode,120)
  text='\n'.join((OUT/f'{mode}.{suffix}').read_text() for suffix in ('stdout','stderr'))
  marker=f'JR_RUNTIME_EXECUTED_{901+(0 if mode in ("off","on","off-again") else rep)}'
  if not r.base.succeeded(row) or not h.valid_tests(r.test_results(text)) or text.count(marker)!=1:raise ValueError(f'{mode}: exact runtime gate failed')
  row.update(mode=mode,source_sha256=hashlib.sha256(contents).hexdigest(),marker=marker)
  if trace.exists():
   data=json.loads(trace.read_text())
   row.update(trace_sha256=r.base.digest(trace),trace=data)
   labels={event.get('name') for event in data.get('traceEvents',[]) if event.get('ph')=='X'}
   row['diagnostic_labels']=sorted(label for label in labels if label and label.startswith('diagnostics.'))
   if mode.startswith('edit-') and not {'diagnostics.broken_in.wall','diagnostics.semantic_inclusive_asked'}.issubset(labels):raise ValueError(f'{mode}: missing positive actual native diagnostics intervals')
  elif mode not in ('off','off-again'):raise ValueError(f'{mode}: no trace')
  state['rows'].append(row)
 state['worker_logs']=[]
 runtime=Path(meta['environments']['engine']['JR_ENGINE_HOME'])/'resident-v1/runtime'
 for log in sorted((runtime/'workers').glob('*/log')):
  destination=OUT/f'worker-{log.parent.name}.log';destination.write_bytes(log.read_bytes())
  state['worker_logs'].append({'relative_worker':log.parent.name,'artifact':destination.name,'sha256':r.base.digest(destination)})
 state.update(status='passed')
except BaseException as error:
 state.update(status='failed',error=repr(error));raise
finally:
 src.write_bytes(original)
 state.update(source_restored=src.read_bytes()==original,source_after=h.source_hashes(SOURCE),tool_after=r.base.digest(tool),toolchain_after=h.toolchain_identity(Path('/home/jeremy/.blaze/toolchains/nightly-2026-10-27-x86_64-unknown-linux-gnu')))
 if state['source_before']!=state['source_after'] or state['tool_before']!=state['tool_after'] or state['toolchain_before']!=state['toolchain_after']:state.update(status='failed',identity_error='source/tool/toolchain changed')
 (OUT/'state.json').write_text(json.dumps(state,indent=2)+'\n')
print(json.dumps(dict(status=state['status'],rows=len(state['rows']))))
if state['status']!='passed':raise SystemExit(1)
