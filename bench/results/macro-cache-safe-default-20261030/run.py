import hashlib,json,os,pathlib,subprocess
root=pathlib.Path(__file__).parent
cargo='/home/jeremy/.jcode/scratch/lint-profile-20261030/home/toolchains/nightly-2026-10-08/bin/cargo'
rustc='/home/jeremy/.justrust/toolchains/jr-ac20de5e7921/bin/rustc'
client='/home/jeremy/.cargo/bin/justrust'
env=os.environ.copy()
for key in ['RUSTC_WRAPPER','RUSTC_WORKSPACE_WRAPPER','RUSTFLAGS','CARGO_ENCODED_RUSTFLAGS','JUSTRUST_CACHE_PROC_MACROS']:
 env.pop(key,None)
env.update({'RUSTC':rustc,'CARGO':cargo,'JUSTRUST_REAL_CARGO':cargo,'CARGO_TARGET_DIR':str(root/'target'),'JUSTRUST_HOME':str(root/'home'),'JUSTRUST_TOOLCHAIN':'system','JUSTRUST_TOOLCHAIN_JR':'1','JUSTRUST_TOOLCHAIN_AUTO_INSTALL':'0','JUSTRUST_REMOTE':'off','JUSTRUST_LOCALCACHE':'0','JUSTRUST_DEPCACHE':'0','JUSTRUST_PREBUILT':'off','JUSTRUST_PEER_CACHE':'0','JUSTRUST_REMOTE_CACHE':'0','JUSTRUST_BUILDD':'0','JUSTRUST_CARGO_FORK':'0','JUSTRUST_CUTOFF':'0','JUSTRUST_HOTPATCH':'0'})
rows=[]
for phase,cmd in [('recorded',[client,'check','--local','-vv']),('unrecorded',[cargo,'check','-vv']),('recorded-repeat',[client,'check','--local','-vv'])]:
 p=subprocess.run(cmd,cwd=root,env=env,capture_output=True,timeout=60)
 (root/(phase+'.stdout')).write_bytes(p.stdout);(root/(phase+'.stderr')).write_bytes(p.stderr)
 assert p.returncode==0,(phase,p.stderr.decode(errors='replace'))
 hashes={str(x.relative_to(root/'target')):hashlib.sha256(x.read_bytes()).hexdigest() for x in (root/'target').rglob('*.rmeta')}
 assert hashes,phase
 rows.append({'phase':phase,'status':p.returncode,'rmeta':hashes})
assert rows[0]['rmeta']==rows[1]['rmeta']==rows[2]['rmeta']
text=(root/'unrecorded.stderr').read_text();assert 'Fresh jr_recording_safety_fixture' in text and 'Dirty ' not in text and 'Running `' not in text,text
runs=[]
for d in sorted((root/'home'/'runs').iterdir()):
 if (d/'summary.json').exists():
  s=json.loads((d/'summary.json').read_text());runs.append({'id':d.name,'summary':s})
assert len(runs)==2
report={'verdict':'passed','scope':'local recorder compatibility, not performance','client_sha256':hashlib.sha256(pathlib.Path(client).read_bytes()).hexdigest(),'rows':rows,'recorded_runs':runs,'reason_local':'unreleased hosted maintenance hold; direct unrecorded Cargo is required to check recorder artifact compatibility'}
(root/'acceptance.json').write_text(json.dumps(report,indent=2)+'\n');print(json.dumps({'verdict':'passed','runs':[r['id'] for r in runs],'rmeta_identical':True}))
