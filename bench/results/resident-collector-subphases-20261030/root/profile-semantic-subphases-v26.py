#!/usr/bin/env python3
"""Actual native semantic subphases, inclusive observations not critical-path savings."""
import hashlib,json,os,re,sys
from pathlib import Path
sys.path.insert(0,'/home/jeremy/justrust/bench')
import resident_engine_compare as h
import blaze_runtime_compare as r
ROOT=Path('/home/jeremy/.jcode/scratch/blaze-native-engine-20261030')
OUT=Path('/home/jeremy/.jcode/scratch/resident-semantic-subphases-v26-profile-20261030')
OUT.mkdir(mode=0o700,exist_ok=False)
SOURCE=Path('/home/jeremy/.jcode/scratch/ba30/source')
TC=Path('/home/jeremy/.blaze/toolchains/nightly-2026-10-27-x86_64-unknown-linux-gnu')
h.pinned_source(SOURCE)
src=SOURCE/'src/summary.rs';original=src.read_bytes();(OUT/'original.rs').write_bytes(original)
state=dict(scope=__doc__,rows=[],source_before=h.source_hashes(SOURCE),toolchain_before=h.toolchain_identity(TC))
configs={}
for name,base in [('new',Path('/home/jeremy/rsa23'))]:
 meta=json.loads((base/'metadata.json').read_text())
 assert meta['status']=='complete' and meta['source_restored'] and meta['unchanged']
 tool=Path(meta['tools_before']['engine']['path']);sha=meta['tools_before']['engine']['sha256']
 assert tool.is_relative_to(ROOT) and not tool.is_symlink() and r.base.digest(tool)==sha
 env={k:v for k,v in os.environ.items() if not k.startswith(('JUSTRUST_','JR_ENGINE_','RA_JIT_','BLAZE_','CARGO_','RUST','XDG_'))}
 env.update(meta['environments']['engine']);env['RA_JIT_DIAGNOSTICS_STATS']='1'
 for var,folder in [('RUNTIME','runtime'),('CACHE','cache'),('CONFIG','config'),('DATA','data')]:env['XDG_'+var+'_HOME' if var!='RUNTIME' else 'XDG_RUNTIME_DIR']=str(base/'engine'/folder)
 home=Path(env['JR_ENGINE_HOME']);assert home.resolve().is_relative_to((base/'engine').resolve())
 stop=r.run_process([str(tool),'stop'],SOURCE,env,OUT/f'{name}-stop',30)
 assert r.base.succeeded(stop)
 configs[name]=(tool,sha,env,home/'resident-v1/runtime/workers')
state['configs']={n:dict(tool=str(c[0]),sha256=c[1],environment=r.base.logged_environment(c[2])) for n,c in configs.items()}
try:
 state['prepares']=[]
 for name,(tool,sha,env,workers) in configs.items():
  src.write_bytes(original)
  prefix=OUT/f'{name}-prepare'
  row=r.run_process(h.command(tool,'engine'),SOURCE,env,prefix,120)
  text='\n'.join(prefix.with_suffix('.'+s).read_text() for s in ('stdout','stderr'))
  if not r.base.succeeded(row) or not h.valid_tests(r.test_results(text)):raise ValueError('startup prepare runtime gate')
  state['prepares'].append(dict(stack=name,**row))
 for rep in range(6):
  contents=r.edit_bytes(original,rep+2600);src.write_bytes(contents)
  for name in ('new',):
   tool,sha,env,workers=configs[name];assert r.base.digest(tool)==sha
   before={str(p):p.stat().st_size for p in workers.glob('*/log')}
   prefix=OUT/f'{rep}-{name}';trace=OUT/f'{rep}-{name}.trace.json';e=env.copy();e.update(RA_JIT_STATS='1',RA_JIT_TRACE=str(trace))
   row=r.run_process(h.command(tool,'engine'),SOURCE,e,prefix,120)
   text='\n'.join(prefix.with_suffix('.'+s).read_text() for s in ('stdout','stderr'))
   marker=f'JR_RUNTIME_EXECUTED_{rep+2601}'
   if not r.base.succeeded(row) or not h.valid_tests(r.test_results(text)) or text.count(marker)!=1:raise ValueError(f'{rep}/{name}: runtime gate')
   data=json.loads(trace.read_text());events=[x for x in data['traceEvents'] if x.get('ph')=='X' and str(x.get('name','')).startswith('diagnostics.')]
   if not {'diagnostics.broken_in.wall','diagnostics.semantic_inclusive_asked','diagnostics.semantic.prepare_parse','diagnostics.semantic.syntax_walk','diagnostics.semantic.module_context','diagnostics.semantic.collect','diagnostics.semantic.render_and_filter'}.issubset({x['name'] for x in events}):raise ValueError('missing positive diagnostics spans')
   delta=b'\n'.join(p.read_bytes()[before.get(str(p),0):] for p in sorted(workers.glob('*/log')))
   log=OUT/f'{rep}-{name}.worker-delta.log';log.write_bytes(delta)
   callbacks=re.findall(rb'asked_callback_calls=(\d+) asked_callback_cumulative_ns=(\d+)',delta)
   transport=re.findall(rb'diagnostics transport: retained=(\d+) stale=(\d+) dropped=(\d+) cap=4000',delta)
   if not callbacks or not transport or any(int(x[2]) for x in transport):raise ValueError('missing callback/transport or dropped spans')
   inventory=re.findall(rb'diagnostics collector: label=(hir\.collect\.[a-z_]+) calls=(\d+) cumulative_ns=(\d+) inclusive=true',delta)
   if not inventory or not {b'hir.collect.module_inclusive',b'hir.collect.item_case',b'hir.collect.selected_body_inclusive'}.issubset({x[0] for x in inventory}):raise ValueError('missing positive collector inventory')
   row['collector_inventory']=[dict(label=x[0].decode(),calls=int(x[1]),cumulative_ns=int(x[2])) for x in inventory]
   row.update(stack=name,replicate=rep,marker=marker,edit_sha256=hashlib.sha256(contents).hexdigest(),events=events,callbacks=[[int(v) for v in x] for x in callbacks],transport=[[int(v) for v in x] for x in transport],trace_sha256=r.base.digest(trace),worker_delta_sha256=r.base.digest(log))
   state['rows'].append(row)
 state['status']='passed'
except BaseException as error:
 state.update(status='failed',error=repr(error));raise
finally:
 src.write_bytes(original)
 state.update(source_after=h.source_hashes(SOURCE),toolchain_after=h.toolchain_identity(TC),tools_after={n:r.base.digest(c[0]) for n,c in configs.items()})
 state['unchanged']=state['source_after']==state['source_before'] and state['toolchain_after']==state['toolchain_before'] and all(state['tools_after'][n]==c[1] for n,c in configs.items())
 (OUT/'state.json').write_text(json.dumps(state,indent=2)+'\n')
assert state['unchanged']
print(json.dumps(dict(status=state['status'],rows=len(state['rows']))))
