#!/usr/bin/env python3
"""Admit exact native correctness before prospective scoped runtime measurement."""
import ctypes,json,os,select,subprocess,sys,time
from pathlib import Path
R=Path('/home/jeremy/.jcode/scratch/blaze-native-engine-20261030')
p=R/'candidate-build-v28/orchestration-exit'
lib=ctypes.CDLL(None);fd=lib.inotify_init1(os.O_NONBLOCK|os.O_CLOEXEC)
assert fd>=0 and lib.inotify_add_watch(fd,os.fsencode(p.parent),0x8|0x80|0x100)>=0
end=time.monotonic()+900
while not p.exists() and time.monotonic()<end:
    if select.select([fd],[],[],max(0,end-time.monotonic()))[0]:os.read(fd,65536)
os.close(fd)
if not p.exists() or p.read_text().strip()!='0':
    (R/'suite-v28-candidate.exit').write_text('2\n')
    raise SystemExit('Native admission failed or incomplete, candidate timing not launched')
U=R/'candidate-unit-v28'
rows=[]
for name,count in [('resident_diagnostic_skeleton',8),(None,359)]:
    found=[]
    for path in (U/'justrust-home/runs').glob('*/summary.json'):
        data=json.loads(path.read_text());args=data.get('args',[])
        if data.get('subcommand')!='test':continue
        matched=(name in args) if name else '--' not in args
        if matched:found.append((path,data))
    assert len(found)==1,(name,len(found))
    path,data=found[0];tests=data['tests']
    assert data['exit']==0 and tests['passed']==count and tests['failed']==0 and tests['ignored']==0,(name,tests)
    assert path.with_name('output.jsonl').stat().st_size>0
    rows.append(dict(filter=name,run_id=data['id'],passed=count))
subprocess.run(['sha256sum','-c',str(R/'candidate-build-v28/source-before.sha256')],cwd='/home/jeremy/justrust/toolchain/resident',check=True,stdout=(U/'admission-source-after.log').open('w'))
(R/'candidate-admission-v28.json').write_text(json.dumps(dict(verdict='passed',rows=rows),indent=2)+'\n')
cmd=[sys.executable,'bench/suite.py','resident','--reference-engine',str(R/'reference-bin/ra-jit'),'--engine',str(R/'candidate-v28-bin/jr-engine'),'--toolchain','/home/jeremy/.blaze/toolchains/nightly-2026-10-27-x86_64-unknown-linux-gnu','--source','/home/jeremy/.jcode/scratch/ba30/source','--output','/home/jeremy/rsa24','--samples','5','--timeout','360']
with (R/'suite-v28-candidate.log').open('w') as log:code=subprocess.run(cmd,cwd='/home/jeremy/justrust',stdout=log,stderr=subprocess.STDOUT).returncode
(R/'suite-v28-candidate.exit').write_text(str(code)+'\n')
if code:raise SystemExit(code)
with (R/'suite-v28-compare.log').open('w') as log:
    subprocess.run([sys.executable,'bench/suite.py','compare','/home/jeremy/rsb18/suite.json','/home/jeremy/rsa24/suite.json','--floor','10'],cwd='/home/jeremy/justrust',stdout=log,stderr=subprocess.STDOUT,check=True)
print((R/'candidate-admission-v28.json').read_text());print((R/'suite-v28-compare.log').read_text())
