#!/usr/bin/env python3
"""Bounded read-only progress monitor. Exit0 is monitor success, not benchmark acceptance."""
import json, shlex, subprocess, time
from pathlib import Path
local=Path('<private-evidence>')
v={k:shlex.split(val)[0] for k,val in (line.split('=',1) for line in (local/'transport.env').read_text().splitlines())}
ssh=shlex.split(v['JR_REMOTE_SSH']); dest=v['JR_REMOTE_DEST']
code='''import json,pathlib,shutil,subprocess,time
p=pathlib.Path('<trial>')
o={'time':time.time(),'free_bytes':shutil.disk_usage(p).free,'load':pathlib.Path('/proc/loadavg').read_text().split()[:3],'modes':{}}
for mode in (0,1):
 f=p/'results'/f'{mode}-exec.jsonl';rows=[json.loads(x) for x in f.read_text().splitlines()] if f.exists() else []
 a=[r for r in rows if r['event']=='after-exec'];b=[r for r in rows if r['event']=='before-exec']
 s=p/'results'/f'{mode}-suite.json'
 log=p/'results'/f'{mode}-output.log'
 checkpoints=[l for l in log.read_text().splitlines() if l.startswith('JCODE_PROGRESS ')] if log.exists() else []
 o['modes'][str(mode)]={'completed_commands':len(a),'started_commands':len(b),'suite_complete':s.exists(),'progress':json.loads(checkpoints[-1].split(' ',1)[1]) if checkpoints else None,'failed_commands':sum(r['exit']!=0 for r in a)}
o['session_alive']=subprocess.run(['tmux','has-session','-t','scheduler-legacy-3f47'],stdout=subprocess.DEVNULL,stderr=subprocess.DEVNULL).returncode==0
print(json.dumps(o))
'''
end=time.monotonic()+530
log=local/'progress-observations.jsonl'
while time.monotonic()<end:
 raw=subprocess.check_output([*ssh,dest,'python3 -'],input=code.encode(),timeout=30).decode()
 row=json.loads(raw)
 with log.open('a') as f: f.write(json.dumps(row)+'\n')
 print('JCODE_PROGRESS '+json.dumps({'message':'Read-only legacy trial progress, not a speedup result','observed':row}),flush=True)
 if not row['session_alive'] or all(x['suite_complete'] for x in row['modes'].values()): break
 time.sleep(60)
print('Monitor complete, audit fetched final/partial results separately',flush=True)
