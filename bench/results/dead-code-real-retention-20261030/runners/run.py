from pathlib import Path
import hashlib,json,os,subprocess,sys
H=Path('/home/jeremy/.jcode/scratch/dead-code-real-retention-20261030')
R=Path('/home/jeremy/justrust')
O=R/'bench/results/dead-code-real-retention-20261030'
RUSTC=Path('/home/jeremy/.jcode/scratch/dead-item-cache-20261030/sysroot/bin/rustc')
CLIENT=Path('/home/jeremy/.jcode/scratch/query-retention-bench-20261030/justrust-client')
CARGO=Path('/home/jeremy/.jcode/scratch/lint-profile-20261030/home/toolchains/nightly-2026-10-08/bin/cargo')
DRIVER=next(RUSTC.parent.parent.glob('lib/librustc_driver*.so'))
def sha(p):
 with Path(p).open('rb') as f:return hashlib.file_digest(f,'sha256').hexdigest()
def save(n,v):(O/(n+'.json')).write_text(json.dumps(v,indent=2)+'\n')
def call(args,stem,env=None,cwd=R):
 with (O/(stem+'.stdout')).open('wb') as a,(O/(stem+'.stderr')).open('wb') as b:
  return subprocess.run(list(map(str,args)),env=env,cwd=cwd,stdout=a,stderr=b).returncode
assert sha(DRIVER)=='3527393f3db7e09eeef270a83f51c5aeedb787c77f845ab76a76618f76ae137e'
epoch=sys.argv[1]
if epoch=='unchanged':os.utime(H/'source/src/lib.rs',None)
for mode in ('off','on'):
 work=H/mode; (work/'profiles').mkdir(parents=True,exist_ok=True)
 env=dict(os.environ,JUSTRUST_HOME=str(work/'home'),JUSTRUST_TOOLCHAIN='system',JUSTRUST_REAL_CARGO=str(CARGO),RUSTC=str(RUSTC),JUSTRUST_REMOTE='0',JUSTRUST_LOCALCACHE='0',JUSTRUST_DEPCACHE='0',JUSTRUST_PREBUILT='off',JUSTRUST_BUILDD='0',CARGO_TARGET_DIR=str(work/'target'),RUSTFLAGS=f'-Zjr-empty-lint-level-fast-path=no -Zjr-cache-small-query-values=no -Zjr-cache-associated-items=no -Zjr-cache-dead-code-items={"yes" if mode=="on" else "no"} -Zself-profile={work}/profiles -Zself-profile-events=default,function-args -Zincremental-verify-ich')
 stem=mode+'-'+epoch
 record=dict(mode=mode,epoch=epoch,identities={str(p):sha(p) for p in [RUSTC,DRIVER,CLIENT,CARGO]},environment={k:env[k] for k in ('JUSTRUST_HOME','JUSTRUST_TOOLCHAIN','JUSTRUST_REAL_CARGO','RUSTC','JUSTRUST_REMOTE','JUSTRUST_LOCALCACHE','JUSTRUST_DEPCACHE','JUSTRUST_PREBUILT','JUSTRUST_BUILDD','CARGO_TARGET_DIR','RUSTFLAGS')},source_sha256=sha(H/'source/src/runs.rs'),source_commit=subprocess.check_output(['git','rev-parse','f97fcc2'],cwd=R,text=True).strip())
 previous=set((work/'profiles').glob('*.mm_profdata'))
 record['status']=call([CLIENT,'check','--lib'],stem,env,H/'source')
 files=[]
 for p in (work/'target').rglob('query-cache.bin'):
  s=p.stat();files.append(dict(path=str(p.relative_to(work/'target')),logical=s.st_size,allocated=s.st_blocks*512,sha256=sha(p)))
 record['query_cache']=dict(files=files,logical=sum(f['logical'] for f in files),allocated=sum(f['allocated'] for f in files))
 profiles=set((work/'profiles').glob('justrust-*.mm_profdata'))-previous
 if record['status']==0:
  assert len(profiles)==1,(stem,list(profiles))
  profile=profiles.pop();record.update(profile=str(profile),profile_sha256=sha(profile))
 record['stdout_sha256']=sha(O/(stem+'.stdout'));record['stderr_sha256']=sha(O/(stem+'.stderr'))
 record['export_status']=call([CLIENT,'export','--out',O/'exported-runs','--machine',f'xps14-dead-code-{mode}-20261030'],stem+'-export',env,H/'source')
 save(stem+'-receipt',record)
 print(json.dumps(dict(mode=mode,epoch=epoch,status=record['status'],export_status=record['export_status'])),flush=True)
 assert record['status']==0 and record['export_status']==0,stem
