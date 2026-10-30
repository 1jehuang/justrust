from pathlib import Path
import os,subprocess,json,hashlib,sys
h=Path('/home/jeremy/.jcode/scratch/provider-real-20261030')
epoch=sys.argv[1]
rustc=Path('/home/jeremy/.jcode/scratch/provider-profile-20261030/sysroot/bin/rustc')
client=Path('/home/jeremy/.jcode/scratch/query-retention-bench-20261030/justrust-client')
cargo='/home/jeremy/.jcode/scratch/lint-profile-20261030/home/toolchains/nightly-2026-10-08/bin/cargo'
(h/'profiles').mkdir(exist_ok=True)
if epoch=='unchanged':os.utime(h/'source/src/lib.rs',None)
env=dict(os.environ,JUSTRUST_HOME=str(h/'home'),JUSTRUST_TOOLCHAIN='system',JUSTRUST_REAL_CARGO=cargo,RUSTC=str(rustc),JUSTRUST_REMOTE='0',JUSTRUST_LOCALCACHE='0',JUSTRUST_DEPCACHE='0',JUSTRUST_PREBUILT='off',JUSTRUST_BUILDD='0',CARGO_TARGET_DIR=str(h/'target'),RUSTFLAGS=f'-Zjr-empty-lint-level-fast-path=no -Zjr-cache-small-query-values=no -Zjr-cache-associated-items=no -Zself-profile={h}/profiles -Zself-profile-events=generic-activity,function-args -Zincremental-verify-ich')
def sha(p):
 with open(p,'rb') as f:return hashlib.file_digest(f,'sha256').hexdigest()
ident={str(p):sha(p) for p in [rustc,client,Path(cargo),*rustc.parent.parent.glob('lib/librustc_driver*.so')]}
with (h/f'{epoch}.log').open('x') as log:r=subprocess.run([str(client),'check','--lib'],cwd=h/'source',env=env,stdout=log,stderr=subprocess.STDOUT)
record=dict(epoch=epoch,status=r.returncode,identities=ident,rustflags=env['RUSTFLAGS'],source_sha256=sha(h/'source/src/runs.rs'))
if r.returncode==0:
 profile=max((h/'profiles').glob('justrust-*.mm_profdata'),key=lambda p:p.stat().st_mtime_ns)
 record.update(profile=str(profile),profile_sha256=sha(profile))
 with (h/f'{epoch}-inventory.json').open('x') as out:
  d=subprocess.run([sys.executable,'/home/jeremy/justrust/bench/reuse_ceiling.py','--profile',str(profile),'--large-capture'],stdout=out)
 record['decoder_status']=d.returncode
(h/f'{epoch}-receipt.json').write_text(json.dumps(record,indent=2)+'\n')
raise SystemExit(r.returncode or record.get('decoder_status',1))
