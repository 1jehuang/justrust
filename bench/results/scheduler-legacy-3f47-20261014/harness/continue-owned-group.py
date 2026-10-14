#!/usr/bin/env python3
"""One nonfatal continuation of the exact owned experiment group, fully recorded."""
import json,shlex,subprocess
from pathlib import Path
local=Path('<private-evidence>')
v={k:shlex.split(val)[0] for k,val in (line.split('=',1) for line in (local/'transport.env').read_text().splitlines())}
ssh=shlex.split(v['JR_REMOTE_SSH']);dest=v['JR_REMOTE_DEST']
code='''import json,os,pathlib,signal,subprocess,time
prefix='<trial>'
head=43441;parent=43443;child=275015
assert os.getpgid(head)==head and os.getpgid(parent)==head and os.getpgid(child)==head
cmd=lambda pid:pathlib.Path(f'/proc/{pid}/cmdline').read_bytes().replace(b'\\0',b' ').decode()
assert prefix+'/observer.py' in cmd(head) and prefix+'/observer.py' in cmd(parent)
assert cmd(child).startswith(prefix+'/bin/justrust test -p jcode-tui --lib')
ps=subprocess.check_output(['ps','-eo','pid,ppid,pgid,sid,tpgid,tty,stat,args']).decode().splitlines()
members=[l for l in ps[1:] if l.split(None,7)[2]==str(head)]
for l in members:
 assert int(l.split(None,7)[0]) in {head,parent,child,275048},l
receipt={'schema':'owned-trial-nonfatal-continue-v1','time_before':time.time(),'run_id':'20261014-101022743-43443971','group':head,'members_before':members,'intervention':'SIGCONT only; no source, scheduler flag or workload edits','timing_eligibility':'disturbed invocation is ineligible, do not claim speedup'}
os.killpg(head,signal.SIGCONT);receipt['signal_sent_at']=time.time();time.sleep(2)
receipt['members_after']=subprocess.check_output(['ps','-eo','pid,ppid,pgid,sid,tpgid,tty,stat,args']).decode().splitlines()
receipt['members_after']=[l for l in receipt['members_after'][1:] if l.split(None,7)[2]==str(head)]
p=pathlib.Path(prefix)/'results/nonfatal-continue.json'
with p.open('x') as f:json.dump(receipt,f,indent=2)
print(json.dumps(receipt))
'''
raw=subprocess.check_output([*ssh,dest,'python3 -'],input=code.encode(),timeout=30).decode()
with (local/'nonfatal-continue.json').open('x') as f:f.write(raw)
print(raw)
