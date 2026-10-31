import json, os, sys
from pathlib import Path
sys.path.insert(0, '/home/jeremy/justrust/bench')
import resident_engine_compare as h
root=Path('/home/jeremy/.jcode/scratch/blaze-native-engine-20261030')
out=root/'matrix-v33-post-timeout-stop'
out.mkdir(mode=0o700,exist_ok=False)
rows=[]
for name in ('rsm03','rsm05','rsm06','rsm07'):
 p=Path('/home/jeremy')/name
 m=json.loads((p/'metadata.json').read_text())
 for key,env in m.get('environments',{}).items():
  rep,stack=key.split(':')
  base=(p/f's{rep}'/stack).resolve()
  home=Path(env['JR_ENGINE_HOME'] if stack=='engine' else env['BLAZE_HOME']).resolve()
  assert home.is_relative_to(base) and not base.is_symlink()
  tool=Path(m['tools_before'][stack]['path'])
  assert h.runtime.base.digest(tool)==m['tools_before'][stack]['sha256']
  launch={k:v for k,v in os.environ.items() if not k.startswith(('JUSTRUST_','JR_ENGINE_','RA_JIT_','BLAZE_','CARGO_','RUST','XDG_'))}
  launch.update(env)
  row=h.runtime.run_process([str(tool),'stop'],Path(m['source']),launch,out/f'{name}-{rep}-{stack}',30)
  rows.append(dict(matrix=name,session=key,namespace=str(home),**row))
(out/'receipt.json').write_text(json.dumps(dict(scope='Exact private namespaces after failed/complete matrix parents ended, no store deletion, native stop receipt only',rows=rows),indent=2))
print(json.dumps(dict(namespaces=len(rows),succeeded=sum(h.runtime.base.succeeded(r) for r in rows))))
