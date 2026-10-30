#!/usr/bin/env python3
"""Bounded focused diagnostic, never a comprehensive promotion gate."""
import hashlib,json,os,pathlib,subprocess,time
root=pathlib.Path('/home/jeremy/justrust')
here=pathlib.Path('/home/jeremy/.jcode/scratch/query-retention-bench-20261030')
state=here/'desktop-check-state.json'
if state.exists(): raise SystemExit('refusing overwrite')
env=dict(os.environ,JUSTRUST=str(here/'justrust-client'),JUSTRUST_HOME=str(here/'home'),JUSTRUST_BENCH_WORK='/home/jeremy/.justrust/bench/work',JUSTRUST_REMOTE='0',JUSTRUST_DEPCACHE='0',JUSTRUST_LOCALCACHE='0')
prefix=here/'home/toolchains/staged/jr-50f30aea8dbb'
artifacts=[here/'justrust-client',prefix/'bin/rustc',*sorted((prefix/'lib').glob('librustc_driver*.so'))]
def hashes():return {str(p):hashlib.file_digest(p.open('rb'),'sha256').hexdigest() for p in artifacts}
data=dict(scope='desktop-check-desktop-body-check-only',status='running',started=time.time(),identities=hashes(),runs={})
def save():
 p=state.with_suffix('.pending');p.write_text(json.dumps(data,indent=2)+'\n');p.replace(state)
save()
try:
 for mode in ('no','yes'):
  assert hashes()==data['identities'],'frozen inputs changed'
  env['RUSTFLAGS']='-Zjr-cache-small-query-values='+mode
  out=here/f'desktop-check-{mode}.json'
  cmd=['python3','-u','bench/suite.py','run','desktop-body-check','-n','3','--warmup','1','--quiet-wait','30','--toolchain','jr-50f30aea8dbb','-o',str(out)]
  data['mode']=mode;data['runs'][mode]={'command':cmd,'rustflags':env['RUSTFLAGS'],'output':str(out)};save()
  with (here/f'desktop-check-{mode}.log').open('x') as log:
   result=subprocess.run(cmd,cwd=root,env=env,stdout=log,stderr=subprocess.STDOUT)
  data['runs'][mode]['status']=result.returncode;save()
  if result.returncode:raise RuntimeError('focused command failed: '+mode)
 assert hashes()==data['identities'],'frozen inputs changed'
 with (here/'desktop-check-compare.log').open('x') as log:
  r=subprocess.run(['python3','bench/suite.py','compare',str(here/'desktop-check-no.json'),str(here/'desktop-check-yes.json')],cwd=root,env=env,stdout=log,stderr=subprocess.STDOUT)
 data['compare_status']=r.returncode;data['status']='completed' if r.returncode==0 else 'failed'
except BaseException as e:
 data['status']='failed';data['error']=repr(e)
finally:
 data['finished']=time.time();save()
