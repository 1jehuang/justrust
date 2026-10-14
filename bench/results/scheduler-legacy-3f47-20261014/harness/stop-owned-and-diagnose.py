#!/usr/bin/env python3
"""Preserve and stop only the suspended experiment, then one bounded headless diagnostic."""
import json,shlex,subprocess
from pathlib import Path
local=Path('<private-evidence>')
v={k:shlex.split(val)[0] for k,val in (line.split('=',1) for line in (local/'transport.env').read_text().splitlines())}
ssh=shlex.split(v['JR_REMOTE_SSH']);dest=v['JR_REMOTE_DEST']
code='''import hashlib,json,os,pathlib,signal,subprocess,time
p=pathlib.Path('<trial>'); head=43441
cmd=lambda pid:pathlib.Path(f'/proc/{pid}/cmdline').read_bytes().replace(b'\\0',b' ').decode()
assert os.getpgid(43443)==head and os.getpgid(275015)==head
assert str(p)+'/observer.py' in cmd(head) and str(p)+'/observer.py' in cmd(43443)
assert cmd(275015).startswith(str(p)+'/bin/justrust test -p jcode-tui --lib')
ps=subprocess.check_output(['ps','-eo','pid,ppid,pgid,sid,tpgid,tty,stat,args']).decode().splitlines();members=[l for l in ps[1:] if l.split(None,7)[2]==str(head)]
assert {int(l.split(None,7)[0]) for l in members}=={43441,43443,275015,275048}
r=p/'home/runs/20261014-101022743-43443971'
receipt={'schema':'owned-suspended-trial-stop-v1','time_before':time.time(),'unfinished_run_id':r.name,'members_before':members,'reason':'owned process group repeatedly stops; preserve partial, no full OFF/ON comparison','unfinished_files':{f.name:{'bytes':f.stat().st_size,'sha256':hashlib.sha256(f.read_bytes()).hexdigest()} for f in r.iterdir() if f.is_file()}}
os.killpg(head,signal.SIGTERM);os.killpg(head,signal.SIGCONT);time.sleep(2)
receipt['time_after']=time.time();receipt['members_remaining']=[l for l in subprocess.check_output(['ps','-eo','pid,ppid,pgid,stat,args']).decode().splitlines()[1:] if l.split(None,4)[2]==str(head)]
assert not receipt['members_remaining'],receipt
with (p/'results/early-stop.json').open('x') as f:json.dump(receipt,f,indent=2)
print(json.dumps(receipt),flush=True)
env=os.environ.copy();env.update({'JUSTRUST_HOME':str(p/'home'),'JUSTRUST_REMOTE':'0','JUSTRUST_TOOLCHAIN':'nightly-2026-10-08','JUSTRUST_CARGO_FORK':'1','JUSTRUST_CARGO_DAEMON':'0','JUSTRUST_SHADOW':'0','JUSTRUST_CARGO_CRITICAL_PATH':'0','JUSTRUST_CARGO_CRITICAL_PATH_COSTS':str(p/'costs.json'),'JUSTRUST_NO_UPDATE':'1','JUSTRUST_NO_TELEMETRY':'1','JUSTRUST_TOOLCHAIN_AUTO_INSTALL':'0','CARGO_BUILD_JOBS':'24','CARGO_TARGET_DIR':str(p/'home/bench/versus/target/desktop/suite-nightly-2026-10-08')})
for k in ('RUSTFLAGS','RUSTC','RUSTC_WRAPPER','JUSTRUST_RUN_ID_OVERRIDE','JUSTRUST_DISABLE','JCODE_API_KEY','JUSTRUST_API_KEY'):env.pop(k,None)
cwd=p/'home/bench/work/desktop/jcode@abff94eeb-jcode-desktop@7d38fb7/jcode'
assert cwd.is_dir()
before={x.name for x in (p/'home/runs').iterdir()}
diag={'schema':'headless-invocation-diagnostic-v1','argv':['test','-p','jcode-tui','--lib'],'start':time.time(),'stdin':'DEVNULL','controlling_tty':'SSH without PTY, not the suspended tmux context','source':'same retained point16 workspace bytes, no source restore or edit','performance_eligibility':'diagnostic only, no benchmark sample'}
with (p/'results/headless-diagnostic.log').open('xb') as f:
 try:
  r=subprocess.run(['timeout','--signal=TERM','--kill-after=5s','70',str(p/'bin/justrust'),*diag['argv']],cwd=cwd,env=env,stdin=subprocess.DEVNULL,stdout=f,stderr=subprocess.STDOUT,timeout=85)
  diag['exit']=r.returncode
 except subprocess.TimeoutExpired:diag['timeout']=True
 diag['end']=time.time()
diag['run_ids']=sorted({x.name for x in (p/'home/runs').iterdir()}-before)
with (p/'results/headless-diagnostic.json').open('x') as f:json.dump(diag,f,indent=2)
print(json.dumps(diag),flush=True)
subprocess.run([str(p/'bin/justrust'),'export','--out',str(p/'export'),'--machine','scheduler-legacy-c7i-20261014'],env=env,cwd=p,check=True)
'''
raw=subprocess.check_output([*ssh,dest,'python3 -'],input=code.encode(),timeout=110).decode()
with (local/'stop-and-diagnostic-observation.txt').open('x') as f:f.write(raw)
print(raw)
