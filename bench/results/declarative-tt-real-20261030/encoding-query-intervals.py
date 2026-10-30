import json,sys
from pathlib import Path
from collections import defaultdict
sys.path.insert(0,'/home/jeremy/justrust/bench')
import query_profile as q
from reuse_ceiling import interval_union_ns
root=Path('/home/jeremy/.jcode/scratch/dp41')
state=json.loads((root/'state.json').read_text())
row=state['rows'][1]
events,strings=q.prepare(q.read_input(root/'profiles'/row['profiles'][0]['path']),allow_unmapped_event_labels=True)
groups=defaultdict(list)
for r in q.iter_prepared_records(events,strings,allow_unmapped_event_labels=True):
 if r['record_type']=='interval' and r['label_resolved'] and r['kind']=='GenericActivity':
  label=r['label']
  if label in ('encode_query_results_for','jr_incremental_green_recompute'):
   text=strings.resolve(r['event_id']).split('\x1e')
   name=text[1] if len(text)==2 else '<unknown>'
   groups[(label,name,r['thread'])].append((r['start_ns'],r['end_ns']))
out=[dict(label=k[0],query=k[1],thread=k[2],records=len(v),union_ns=interval_union_ns(v)) for k,v in groups.items()]
out.sort(key=lambda r:r['union_ns'],reverse=True)
(root/'encoding-query-intervals.json').write_text(json.dumps(out,indent=2)+'\n')
print(json.dumps(out[:20],indent=2))
