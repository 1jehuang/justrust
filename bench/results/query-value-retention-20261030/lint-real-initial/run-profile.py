#!/usr/bin/env python3
"""Native real-source mechanism diagnostic. Instrumented time is not speedup."""
from pathlib import Path
import hashlib,json,os,subprocess,sys,time
root=Path('/home/jeremy/justrust');here=Path('/home/jeremy/.jcode/scratch/lint-profile-20261030')
state=here/'profile-state.json'
if state.exists():raise SystemExit('refusing overwrite')
client=Path('/home/jeremy/.jcode/scratch/query-retention-bench-20261030/justrust-client')
rustc=here/'sysroot/bin/rustc';driver=next((here/'sysroot/lib').glob('librustc_driver*.so'))
cargo=here/'home/toolchains/nightly-2026-10-08/bin/cargo'
def digest(p):
 with p.open('rb') as f:return hashlib.file_digest(f,'sha256').hexdigest()
identities={str(p):digest(p) for p in [client,rustc,driver,cargo]}
data=dict(schema=1,scope='real-pinned-justrust-unchanged-lint-mechanism-only',source_commit='f97fcc2',status='running',identities=identities,runs={})
def save():
 p=state.with_suffix('.pending');p.write_text(json.dumps(data,indent=2)+'\n');p.replace(state)
save()
try:
 for mode in ['no','yes']:
  prof=here/('profiles-'+mode);prof.mkdir()
  env=dict(os.environ,JUSTRUST_HOME=str(here/'home'),JUSTRUST_TOOLCHAIN='system',JUSTRUST_REAL_CARGO=str(cargo),RUSTC=str(rustc),JUSTRUST_REMOTE='0',JUSTRUST_LOCALCACHE='0',JUSTRUST_DEPCACHE='0',JUSTRUST_PREBUILT='off',JUSTRUST_BUILDD='0',CARGO_TARGET_DIR=str(here/('target-'+mode)),RUSTFLAGS=f'-Zjr-empty-lint-level-fast-path={mode} -Zjr-cache-small-query-values=no -Zjr-cache-associated-items=no -Zself-profile={prof} -Zself-profile-events=default,function-args -Zincremental-verify-ich')
  for epoch in ['prepare','forced-unchanged']:
   assert {p:digest(Path(p)) for p in identities}==identities,'changed input binary'
   if epoch=='forced-unchanged':os.utime(here/'source/src/lib.rs',None)
   cmd=[str(client),'check','--lib'];data['mode']=mode;data['epoch']=epoch
   rec=dict(command=cmd,rustflags=env['RUSTFLAGS'],log=str(here/f'{mode}-{epoch}.log'));data['runs'][f'{mode}-{epoch}']=rec;save()
   with Path(rec['log']).open('x') as log:
    p=subprocess.run(cmd,cwd=here/'source',env=env,stdout=log,stderr=subprocess.STDOUT)
   rec['status']=p.returncode;save()
   if p.returncode:raise RuntimeError(f'{mode} {epoch} failed')
   if epoch=='forced-unchanged':
    captures=sorted(prof.glob('justrust-*.mm_profdata'),key=lambda p:p.stat().st_mtime_ns)
    if len(captures)<2:raise RuntimeError('no fresh forced compiler capture')
    profile=captures[-1];rec['profile']=str(profile);rec['profile_sha256']=digest(profile)
    cmd=[sys.executable,str(root/'bench/reuse_ceiling.py'),'--profile',str(profile),'--large-capture','--allow-unmapped-event-labels']
    with (here/f'report-{mode}.json').open('x') as out:
     p=subprocess.run(cmd,cwd=root,stdout=out,stderr=subprocess.PIPE,text=True)
    if p.returncode:raise RuntimeError('decoder failed: '+p.stderr)
 assert {p:digest(Path(p)) for p in identities}==identities,'changed input binary'
 data['status']='completed'
except BaseException as e:data['status']='failed';data['error']=repr(e)
finally:save()
