#!/usr/bin/env python3
"""Actual literal-cache eligibility diagnostic, not a timing benchmark."""
import argparse
import hashlib
import importlib.util
import json
from pathlib import Path
import sys
sys.path.insert(0, '/home/jeremy/justrust/bench')
import blaze_runtime_compare as h
import query_profile as q
spec = importlib.util.spec_from_file_location('accept', '/home/jeremy/justrust/bench/reuse-declarative-macro-accept.py')
a = importlib.util.module_from_spec(spec)
spec.loader.exec_module(a)
ROOT = Path('/home/jeremy/.jcode/scratch/dp41')
SOURCE = Path('/home/jeremy/.jcode/scratch/ba30/source')
ARGS = argparse.Namespace(output=ROOT, source=SOURCE,
 rustc=Path('/home/jeremy/.jcode/scratch/declarative-cache-20261030/sysroot-tt/bin/rustc'),
 cargo=Path('/home/jeremy/.jcode/scratch/lint-profile-20261030/home/toolchains/nightly-2026-10-08/bin/cargo'),
 justrust=Path('/home/jeremy/.jcode/scratch/query-retention-bench-20261030/justrust-client'),
 blaze=Path('/home/jeremy/.jcode/scratch/blaze-comparison-20261030/cargo/bin/blaze'), compiler_nightly=True, wild=None)

def main():
 ROOT.mkdir(mode=0o700, exist_ok=False)
 profiles = ROOT / 'profiles'
 profiles.mkdir()
 source = SOURCE / 'src/summary.rs'
 original = source.read_bytes()
 (ROOT / 'summary.rs.backup').write_bytes(original)
 state = dict(status='running', scope='real-tt-cache-eligibility-not-speedup', source_before=h.source_hashes(SOURCE,ROOT), identities_before=h.identities(ARGS), rows=[])
 def save(): (ROOT/'state.json').write_text(json.dumps(state,indent=2)+'\n')
 save()
 env = h.environment(ARGS,'owned-declarative')
 env['RUSTFLAGS'] += f' -Zself-profile={profiles} -Zself-profile-events=generic-activity,function-args'
 expected = None
 try:
  for phase in ('prepare','edit','restore'):
   content = h.edit_bytes(original,replicate=38,anchor=b'fn shares_overlapping_time() {',kind='edit') if phase=='edit' else original
   source.write_bytes(content)
   before_profiles = set(profiles.glob('*.mm_profdata'))
   before = h.recorded_runs(env)
   prefix=ROOT/phase
   row=h.run_process([str(ARGS.justrust),'test','--lib','--','summary::tests::','--test-threads=1','--nocapture'],SOURCE,env,prefix,360)
   output,receipt=h.recorded_output(env,before,prefix)
   row.update(phase=phase,raw_output=receipt,parsed=h.test_results(output),source_sha256=hashlib.sha256(content).hexdigest(),profiles=[])
   if not h.accepted(row,output,expected,'JR_RUNTIME_EXECUTED_39' if phase=='edit' else None): raise RuntimeError('observable runtime gate failed')
   expected=row['parsed']
   for path in sorted(set(profiles.glob('*.mm_profdata'))-before_profiles):
    data=q.read_input(path)
    events,strings=q.prepare(data,allow_unmapped_event_labels=True)
    metadata=q.metadata_from_strings(strings)
    if '--crate-name justrust ' not in metadata.get('cmd',''): continue
    counts=a.marker_counts(q.iter_prepared_records(events,strings,allow_unmapped_event_labels=True))
    counts.update(path=path.name,sha256=hashlib.sha256(data).hexdigest(),metadata=metadata)
    row['profiles'].append(counts)
   if len(row['profiles']) != 1: raise RuntimeError('expected one exact main-crate profile')
   state['rows'].append(row)
   save()
  state['status']='completed'
 finally:
  source.write_bytes(original)
  state['source_after']=h.source_hashes(SOURCE,ROOT)
  state['identities_after']=h.identities(ARGS)
  save()
 assert state['source_before']==state['source_after']
 assert state['identities_before']==state['identities_after']
 print(json.dumps({'status':state['status'],'counts':[{r['phase']:r['profiles'][0]['markers']} for r in state['rows']]},indent=2))
if __name__=='__main__': main()
