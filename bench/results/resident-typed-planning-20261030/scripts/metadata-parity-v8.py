#!/usr/bin/env python3
"""Old/new native metadata byte parity on identical local manifests, outside timing."""
import json,os,sys
from pathlib import Path
sys.path.insert(0,'/home/jeremy/justrust/bench')
import blaze_runtime_compare as r
OUT=Path('/home/jeremy/.jcode/scratch/typed-planning-metadata-parity-v8-dispatch-20261030');OUT.mkdir(mode=0o700,exist_ok=False)
root=OUT/'fixture';root.mkdir()
files={'Cargo.toml':"[workspace]\nmembers=['alpha','beta']\nresolver='2'\n",'alpha/Cargo.toml':"[package]\nname='alpha'\nversion='0.1.0'\nedition='2021'\n[dependencies]\nbeta={path='../beta'}\n[features]\ndefault=['enabled']\nenabled=[]\n[[bin]]\nname='feature-bin'\npath='src/main.rs'\nrequired-features=['enabled']\n",'alpha/src/lib.rs':'pub fn answer()->u32 { beta::answer() }\n','alpha/src/main.rs':'fn main() {}\n','beta/Cargo.toml':"[package]\nname='beta'\nversion='0.1.0'\nedition='2021'\n[lib]\nharness=false\n",'beta/src/lib.rs':'pub fn answer()->u32 {42}\n'}
for name,text in files.items():
 p=root/name;p.parent.mkdir(parents=True,exist_ok=True);p.write_text(text)
base=Path('/home/jeremy/rsa04');meta=json.loads((base/'metadata.json').read_text());state={'scope':'native-old-new-identical-metadata-bytes-not-speedup','rows':[]}
cases=[('full',[]),('no-deps',['--no-deps']),('all-features',['--all-features']),('no-default-features',['--no-default-features']),('missing-feature',['--features','JR_TYPED_UNKNOWN_FEATURE']),('bad-format',['--format-version','999'])]
try:
 for name,flags in cases:
  pair=[]
  for variant in ('old','new'):
   env={k:v for k,v in os.environ.items() if not k.startswith(('JUSTRUST_','JR_ENGINE_','RA_JIT_','BLAZE_','CARGO_','RUST','XDG_'))};env.update(meta['environments']['engine'])
   env['CARGO_TARGET_DIR']=str(OUT/'shared-target')
   tool=Path('/home/jeremy/.jcode/scratch/blaze-native-engine-20261030')/('candidate-v4-bin' if variant=='old' else 'candidate-v8-bin')/'jr-engine'
   command=[str(tool),'cargo','metadata','--format-version','1']+flags
   row=r.run_process(command,root,env,OUT/f'{name}-{variant}',60)
   row.update(case=name,variant=variant,tool_sha256=r.base.digest(tool));state['rows'].append(row)
   pair.append((row['status'],(OUT/f'{name}-{variant}.stdout').read_bytes(),(OUT/f'{name}-{variant}.stderr').read_bytes()))
  if pair[0]!=pair[1]:raise ValueError(f'{name}: native status/stdout/stderr differ')
  if name not in ('missing-feature','bad-format') and pair[0][0]!=0:raise ValueError(f'{name}: unexpected failure')
  if name in ('missing-feature','bad-format') and pair[0][0]==0:raise ValueError(f'{name}: expected rejection absent')
 state['status']='passed'
except BaseException as error:state.update(status='failed',error=repr(error));raise
finally:(OUT/'state.json').write_text(json.dumps(state,indent=2)+'\n')
print(json.dumps({'status':state['status'],'cases':len(cases),'comparisons':'exact status/stdout/stderr bytes'}))
