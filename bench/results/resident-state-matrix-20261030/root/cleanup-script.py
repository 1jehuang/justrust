#!/usr/bin/env python3
"""Stop only completed matrix sessions1-4. Preserve session0 for diagnostic probe and all stores."""
import json,os,sys
from pathlib import Path
sys.path.insert(0,'/home/jeremy/justrust/bench')
import resident_engine_compare as h
ROOT=Path('/home/jeremy/.jcode/scratch/blaze-native-engine-20261030')
OUT=ROOT/'completed-matrix-private-stop'
OUT.mkdir(mode=0o700,exist_ok=False)
meta=json.loads(Path('/home/jeremy/rsm01/metadata.json').read_text())
assert meta['source_restored'] and meta['unchanged'] and meta['raw_receipts_unchanged']
source=Path(meta['source'])
assert h.source_hashes(source)==meta['source_before']
results=[]
for rep in range(1,5):
 for stack in ('reference','engine'):
  key=f'{rep}:{stack}';env=meta['environments'][key].copy()
  tool=Path(meta['tools_before'][stack]['path'])
  assert h.runtime.base.digest(tool)==meta['tools_before'][stack]['sha256']
  base=Path(f'/home/jeremy/rsm01/s{rep}/{stack}').resolve()
  home=Path(env['JR_ENGINE_HOME'] if stack=='engine' else env['BLAZE_HOME']).resolve()
  assert home.is_relative_to(base) and not base.is_symlink()
  launch={k:v for k,v in os.environ.items() if not k.startswith(('JUSTRUST_','JR_ENGINE_','RA_JIT_','BLAZE_','CARGO_','RUST','XDG_'))}
  launch.update(env)
  row=h.runtime.run_process([str(tool),'stop'],source,launch,OUT/f'{rep}-{stack}',30)
  results.append(dict(session=key,namespace=str(home),**row))
(OUT/'receipt.json').write_text(json.dumps(dict(scope=__doc__,signal_or_native_stop_receipts_only=True,rows=results),indent=2)+'\n')
assert all(h.runtime.base.succeeded(x) for x in results)
print(json.dumps(dict(status='passed',namespaces=len(results),stores_deleted=False)))
