#!/usr/bin/env python3
"""Matched source demand-cache mechanism diagnostic, not latency evidence."""
import argparse
import hashlib
import json
from pathlib import Path
import sys
from collections import defaultdict
sys.path.insert(0, '/home/jeremy/justrust/bench')
import blaze_runtime_compare as h
import query_profile as q
from reuse_ceiling import interval_union_ns
ROOT=Path('/home/jeremy/.jcode/scratch/dp43')
SOURCE=Path('/home/jeremy/.jcode/scratch/ba30/source')
ARGS=argparse.Namespace(output=ROOT,source=SOURCE,
 rustc=Path('/home/jeremy/.jcode/scratch/demand-query-cache-20261030/sysroot/bin/rustc'),
 cargo=Path('/home/jeremy/.jcode/scratch/lint-profile-20261030/home/toolchains/nightly-2026-10-08/bin/cargo'),
 justrust=Path('/home/jeremy/.jcode/scratch/query-retention-bench-20261030/justrust-client'),
 blaze=Path('/home/jeremy/.jcode/scratch/blaze-comparison-20261030/cargo/bin/blaze'),compiler_nightly=True,wild=None)
LABELS=('incr_comp_query_cache_promotion','jr_demand_query_cache_promotion_skipped','incr_comp_persist_result_cache','incr_comp_serialize_result_cache','encode_query_values','jr_incremental_green_recompute')
def main():
 ROOT.mkdir(mode=0o700,exist_ok=False)
 source=SOURCE/'src/summary.rs'
 original=source.read_bytes()
 (ROOT/'summary.rs.backup').write_bytes(original)
 state=dict(status='running',scope='real-demand-cache-mechanism-not-speedup',source_before=h.source_hashes(SOURCE,ROOT),identities_before=h.identities(ARGS),rows=[])
 def save(): (ROOT/'state.json').write_text(json.dumps(state,indent=2)+'\n')
 save()
 expected=None
 try:
  for stack in ('owned-off','owned-demand'):
   profiles=ROOT/'profiles'/stack
   profiles.mkdir(parents=True)
   env=h.environment(ARGS,stack)
   env['RUSTFLAGS']+=f' -Zself-profile={profiles} -Zself-profile-events=generic-activity,function-args'
   for phase in ('prepare','edit','restore'):
    content=h.edit_bytes(original,replicate=42,anchor=b'fn shares_overlapping_time() {',kind='edit') if phase=='edit' else original
    source.write_bytes(content)
    before_profiles=set(profiles.glob('*.mm_profdata'))
    before=h.recorded_runs(env)
    prefix=ROOT/f'{stack}-{phase}'
    row=h.run_process([str(ARGS.justrust),'test','--lib','--','summary::tests::','--test-threads=1','--nocapture'],SOURCE,env,prefix,360)
    output,receipt=h.recorded_output(env,before,prefix)
    row.update(stack=stack,phase=phase,raw_output=receipt,parsed=h.test_results(output),source_sha256=hashlib.sha256(content).hexdigest(),profiles=[])
    state['rows'].append(row)
    save()
    if not h.accepted(row,output,expected,'JR_RUNTIME_EXECUTED_43' if phase=='edit' else None): raise RuntimeError('actual runtime correctness gate failed')
    expected=row['parsed']
    for path in sorted(set(profiles.glob('*.mm_profdata'))-before_profiles):
     data=q.read_input(path)
     events,strings=q.prepare(data,allow_unmapped_event_labels=True)
     metadata=q.metadata_from_strings(strings)
     if '--crate-name justrust ' not in metadata.get('cmd',''): continue
     intervals=defaultdict(list)
     total=unresolved=0
     for event in q.iter_prepared_records(events,strings,allow_unmapped_event_labels=True):
      total+=1
      if not event['label_resolved']: unresolved+=1
      elif event['record_type']=='interval' and event['kind']=='GenericActivity' and event['label'] in LABELS:
       intervals[(event['label'],event['thread'])].append((event['start_ns'],event['end_ns']))
     row['profiles'].append(dict(path=str(path.relative_to(ROOT)),sha256=hashlib.sha256(data).hexdigest(),metadata=metadata,records=total,unresolved_records=unresolved,intervals=[dict(label=k[0],thread=k[1],records=len(v),union_elapsed_ns=interval_union_ns(v)) for k,v in sorted(intervals.items())]))
    if len(row['profiles'])!=1: raise RuntimeError('expected one exact main-crate profile')
    save()
  state['status']='completed'
 finally:
  source.write_bytes(original)
  state['source_after']=h.source_hashes(SOURCE,ROOT)
  state['identities_after']=h.identities(ARGS)
  save()
 assert state['source_before']==state['source_after'] and state['identities_before']==state['identities_after']
 print(json.dumps(dict(status=state['status'],intervals=[dict(stack=r['stack'],phase=r['phase'],intervals=r['profiles'][0]['intervals']) for r in state['rows']]),indent=2))
if __name__=='__main__': main()
