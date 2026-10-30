import json,sys
from pathlib import Path
from collections import defaultdict
sys.path.insert(0,'/home/jeremy/justrust/bench')
import query_profile as q
from reuse_ceiling import interval_union_ns
root=Path('/home/jeremy/.jcode/scratch/dp41')
state=json.loads((root/'state.json').read_text())
out=[]
for row in state['rows']:
 p=root/'profiles'/row['profiles'][0]['path']
 events,strings=q.prepare(q.read_input(p),allow_unmapped_event_labels=True)
 selected=defaultdict(list)
 for r in q.iter_prepared_records(events,strings,allow_unmapped_event_labels=True):
  if r['record_type']=='interval' and r['label_resolved'] and (r['label'].startswith('incr_comp_') or r['label']=='encode_query_values' or r['label']=='encode_query_results_for'):
   selected[(r['label'],r['thread'])].append((r['start_ns'],r['end_ns']))
 out.append(dict(phase=row['phase'],intervals=[dict(label=k[0],thread=k[1],records=len(v),elapsed_union_ns=interval_union_ns(v)) for k,v in sorted(selected.items())]))
(root/'persistence-intervals.json').write_text(json.dumps(out,indent=2)+'\n')
print(json.dumps(out,indent=2))
