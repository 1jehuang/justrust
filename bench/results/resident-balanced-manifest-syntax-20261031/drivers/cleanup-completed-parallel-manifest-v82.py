#!/usr/bin/env python3
"""Stop only validated completed v82 strict namespaces, preserving stores."""
import json, os, sys
from pathlib import Path
sys.path.insert(0, '<repo>/bench')
import resident_engine_compare as h
ROOT=Path('<native-root>')
OUT=ROOT/'completed-parallel-manifest-v82-stop'
OUT.mkdir(mode=0o700, exist_ok=False)
SOURCE=Path('<source63>')
rows=[]
for arm in ('off','on','on-repeat','off-repeat'):
 case=f'parallel-manifest-seven-v82-{arm}'
 meta=json.loads((ROOT/case/'state.json').read_text())
 private=Path('<home>')/f'p82-{arm}'
 assert meta['status']=='passed' and meta['unchanged'] and Path(meta['private'])==private
 assert meta['source_after']==h.source_hashes(SOURCE)
 assert all(r['accepted'] and r['input_unchanged'] and not r['failures'] for r in meta['rows'])
 home=private/'engine/engine-home'
 assert home.is_dir() and not private.is_symlink() and not home.is_symlink()
 tool=ROOT/'candidate-v82-bin/jr-engine'
 assert h.runtime.base.digest(tool)==meta['tool_after']
 env={k:v for k,v in os.environ.items() if not k.startswith(('JUSTRUST_','JR_ENGINE_','RA_JIT_','BLAZE_','CARGO_','RUST','XDG_'))}
 env['JR_ENGINE_HOME']=str(home)
 row=h.runtime.run_process([str(tool),'stop'],SOURCE,env,OUT/case,30)
 rows.append({'case':case,'private':str(private),'post_measurement_only':True,**row})
(OUT/'receipt.json').write_text(json.dumps({'scope':__doc__,'stores_deleted':False,'rows':rows},indent=2)+'\n')
assert len(rows)==4 and all(h.runtime.base.succeeded(r) for r in rows)
print(json.dumps({'status':'passed','namespaces':4,'stores_deleted':False}))
