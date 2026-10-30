#!/usr/bin/env python3
"""Offline corrected full-lib admission with independently pinned ignored microbenchmark."""
import hashlib
import json
from pathlib import Path
import re
import subprocess
import sys
sys.path.insert(0, '/home/jeremy/justrust/bench')
import resident_engine_compare as shared

OUT=Path('/home/jeremy/rmp01')
ROOT=Path('/home/jeremy/.jcode/scratch/blaze-native-engine-20261030')
original=(OUT/'state.json').read_bytes()
state=json.loads(original)
assert state['status']=='failed/support-not-established' and state['unchanged']
for key in ('source','toolchain','tools'):
    assert state[key+'_before']==state[key+'_after'], key
pinned=subprocess.check_output(['git','show','f97fcc2:src/split_index.rs'],cwd='/home/jeremy/justrust')
assert re.search(rb'#\[test\]\s*#\[ignore\]\s*fn attach_cost\(',pinned), 'ignored benchmark must be independently declared in pinned source'
assert len(state['rows'])==2
raw_receipts=[]
for row in state['rows']:
    stack=row['stack']
    prefix=OUT/f'{stack}-full-lib'
    stdout=prefix.with_suffix('.stdout').read_bytes()
    stderr=prefix.with_suffix('.stderr').read_bytes()
    assert hashlib.sha256(stdout).hexdigest()==row['stdout_sha256']
    assert hashlib.sha256(stderr).hexdigest()==row['stderr_sha256']
    parsed=json.loads(json.dumps(shared.runtime.test_results(stdout.decode(errors='replace')+'\n'+stderr.decode(errors='replace'))))
    assert shared.runtime.base.succeeded(row) and parsed==row['parsed']
    assert (parsed['passed'],parsed['failed'],parsed['ignored'],parsed['filtered'])==(147,0,1,0)
    assert len(parsed['tests'])==148 and len({n for n,_ in parsed['tests']})==148
    assert [x for x in parsed['tests'] if x[1]!='ok']==[['split_index::tests::attach_cost','ignored']]
    raw_receipts.append(dict(stack=stack,status=row['status'],stdout_sha256=row['stdout_sha256'],stderr_sha256=row['stderr_sha256'],passed=147,ignored=1))
assert state['rows'][0]['parsed']==state['rows'][1]['parsed']
assert (OUT/'state.json').read_bytes()==original
report=dict(status='passed',scope='Offline semantic admission only. Not new native execution or cold speed claim.',original_probe_status=state['status'],original_state_sha256=hashlib.sha256(original).hexdigest(),correction='Original harness wrongly demanded no ignored tests. Exact pinned ignored microbenchmark is preserved, with no new ignored tests permitted.',ignored_source_sha256=hashlib.sha256(pinned).hexdigest(),rows=raw_receipts)
(ROOT/'probe-resident-full-lib-v28-corrected-admission.json').write_text(json.dumps(report,indent=2)+'\n')
print(json.dumps(report,indent=2))
