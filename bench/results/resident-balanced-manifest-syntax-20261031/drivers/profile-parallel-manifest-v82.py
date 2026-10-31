#!/usr/bin/env python3
"""Actual accepted native warm-path spans, inclusive diagnostic, not a speed comparison."""
import hashlib,json,sys
from pathlib import Path
sys.path.insert(0,'<repo>/bench')
import resident_engine_compare as h
import blaze_runtime_compare as r
ROOT=Path('<native-root>')
OUT=ROOT/'resolver-subphases-profile-v82';OUT.mkdir(mode=0o700,exist_ok=False)
SOURCE=Path('<source63>')
TC=Path('<toolchain>')
TOOL=ROOT/'candidate-v82-bin/jr-engine'
h.pinned_source(SOURCE)
src=SOURCE/'src/summary.rs';original=src.read_bytes();(OUT/'source.original').write_bytes(original)
env=h.environment(Path('<home>/pr82a'),'engine',TC)
env.update(RA_JIT_MANIFEST_CONTENT_GENERATION='1',RA_JIT_WATCHED_MANIFEST_BYTES='1',RA_JIT_DEMAND_STD='1',RA_JIT_PARALLEL_MANIFEST_PARSE='1')
state={'scope':__doc__,'rows':[],'source_before':h.source_hashes(SOURCE),'tool_before':r.base.digest(TOOL),'toolchain_before':h.toolchain_identity(TC)}
def save(): (OUT/'state.json').write_text(json.dumps(state,indent=2)+'\n')
save()
try:
 prepare=r.run_process(h.command(TOOL,'engine'),SOURCE,env,OUT/'prepare',180)
 text='\n'.join((OUT/('prepare.'+s)).read_text() for s in ('stdout','stderr'))
 assert r.base.succeeded(prepare) and h.valid_tests(r.test_results(text))
 state['prepare']=prepare
 for rep in range(4):
  contents=r.edit_bytes(original,8200+rep);src.write_bytes(contents)
  for kind in ('edit','noop'):
   prefix=OUT/f'{rep}-{kind}';trace=OUT/f'{rep}-{kind}.trace.json'
   callenv=dict(env,RA_JIT_STATS='1',RA_JIT_TRACE=str(trace))
   row=r.run_process(h.command(TOOL,'engine'),SOURCE,callenv,prefix,120)
   text='\n'.join(prefix.with_suffix('.'+s).read_text() for s in ('stdout','stderr'))
   marker=f'JR_RUNTIME_EXECUTED_{8201+rep}'
   assert r.base.succeeded(row) and h.valid_tests(r.test_results(text)) and text.count(marker)==1
   assert text.count('ra-jit: parallel-manifest-parse-v81 batch')==1, 'actual parallel batch marker required'
   data=json.loads(trace.read_text());events=[x for x in data['traceEvents'] if x.get('ph')=='X']
   assert {'client planning','client planning rustc host','client planning target cfg','client planning asked setup','client planning owned resolver'}.issubset({x.get('name') for x in events})
   resolver_events=[x for x in events if x.get('name','').startswith('resolver ')]
   assert {'resolver package loading','resolver version resolve'}.issubset({x['name'] for x in resolver_events}), resolver_events
   assert len({x['name'] for x in resolver_events})>=4, resolver_events
   row.update(resolver_labels_observed=sorted({x['name'] for x in resolver_events}),kind=kind,replicate=rep,marker=marker,source_sha256=hashlib.sha256(contents).hexdigest(),trace_sha256=r.base.digest(trace),events=events)
   state['rows'].append(row);save()
 state['status']='passed'
except BaseException as error:
 state.update(status='failed',error=repr(error));raise
finally:
 src.write_bytes(original)
 state['source_after']=h.source_hashes(SOURCE);state['tool_after']=r.base.digest(TOOL);state['toolchain_after']=h.toolchain_identity(TC)
 state['unchanged']=state['source_before']==state['source_after'] and state['tool_before']==state['tool_after'] and state['toolchain_before']==state['toolchain_after']
 state['stop']=r.run_process([str(TOOL),'stop'],SOURCE,env,OUT/'stop',30)
 save()
assert state['unchanged'] and r.base.succeeded(state['stop'])
print(json.dumps({'status':state['status'],'rows':len(state['rows']),'unchanged':state['unchanged'],'stores_deleted':False}))
