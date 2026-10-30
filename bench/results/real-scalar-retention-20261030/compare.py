from pathlib import Path
import sys,json,hashlib,collections
sys.path.insert(0,'/home/jeremy/justrust/bench')
import query_profile,reuse_ceiling
base=Path('/home/jeremy/.jcode/scratch/provider-cost-real-20261030')
on=Path('/home/jeremy/.jcode/scratch/real-scalar-profile-20261030')
names=['hir_owner_parent_q','constness','coroutine_kind']
report={'scope':'real-pinned-scalar-retention-mechanism-only','source_commit':'f97fcc2','speedup':None,'time_saved_ns':None,'maximum_reuse':None,'captures':{},'storage':{}}
for epoch in ['unchanged','body-edit']:
 refs={}
 for mode,h in [('off',base),('on',on)]:
  r=json.loads((h/f'{epoch}-receipt.json').read_text());refs[mode]=r
  raw=Path(r['profile']).read_bytes();assert hashlib.sha256(raw).hexdigest()==r['profile_sha256']
  events,strings=query_profile.prepare(raw,True,True)
  rows=list(query_profile.iter_prepared_records(events,strings,True))
  inventory=reuse_ceiling.summarize_records(rows,strings.resolve)
  selected={name:{kind:sum(e['kind']==kind and e['label']==name for e in rows) for kind in ['Query','IncrementalLoadResult']} for name in names}
  for name in names:
   selected[name]['native_green_records']={label:inventory['native_green_reuse']['markers'][label]['query_name_interval_records'].get(name) for label in reuse_ceiling.NATIVE_MARKERS}
  report['captures'][f'{mode}-{epoch}']={'profile_sha256':r['profile_sha256'],'compiler_status':r['status'],'original_strict_decoder_status':r['decoder_status'],'allow_unmapped_event_labels':True,'unresolved_event_label_records':sum(not e['label_resolved'] for e in rows),'unknown_query_labels':sum(e['kind']=='Query' and not e['label_resolved'] for e in rows),'selected':selected}
 assert refs['off']['identities']==refs['on']['identities']
 assert refs['off']['source_sha256']==refs['on']['source_sha256']
 def policy(r,h):return r['rustflags'].replace(str(h/'profiles'),'<PROFILE>').replace('-Zjr-cache-small-query-values=yes','-Zjr-cache-small-query-values=<MODE>').replace('-Zjr-cache-small-query-values=no','-Zjr-cache-small-query-values=<MODE>')
 assert policy(refs['off'],base)==policy(refs['on'],on)
for mode,h in [('off',base),('on',on)]:
 current=json.loads((h/'body-edit-receipt.json').read_text())
 # Storage must be sampled at the edited epoch, before compiling restoration.
 # Historical counts can be replayed separately from the retained capture hashes.
 if mode=='on':
  assert hashlib.sha256((h/'source/src/runs.rs').read_bytes()).hexdigest()==current['source_sha256'], 'candidate source restored: recorded edited-state storage cannot be resampled'
 files=list((h/'target/debug/incremental').glob('justrust-*/s-*/query-cache.bin'))
 assert len(files)==1
 p=files[0];s=p.stat();report['storage'][mode]={'state':'after-body-edit-before-restoration-compile','query_cache_path':str(p),'logical_bytes':s.st_size,'allocated_bytes':s.st_blocks*512,'sha256':hashlib.sha256(p.read_bytes()).hexdigest()}
report['limitations']=['Explicit unknown-label mode preserves unresolved records. No cross-invocation numeric IDs joined.','Absent events are not a global no-work assertion. Counts are capture observations, not a request-constant avoidance calculation.','Independent isolated targets/profile roots differ. Identical binary identities, pinned source state and normalized policy verified except intended scalar flag.','Instrumented, unpaired and potentially contended. No causal timing or default promotion. Storage is one matched edited state, not a universal cache cost.']
print(json.dumps(report,indent=2))
