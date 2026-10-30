#!/usr/bin/env python3
"""Replay measured literal admission, never infer time saved from hit counts."""
import gzip
import hashlib
import json
from pathlib import Path
import sys
ROOT = Path(__file__).resolve().parent
REPO = ROOT.parents[2]
sys.path.insert(0, str(REPO / 'bench'))
import query_profile as q
import blaze_runtime_compare as h

def main():
 manifest = json.loads((ROOT/'sha256.json').read_text())
 for name, expected in manifest.items():
  assert hashlib.sha256((ROOT/name).read_bytes()).hexdigest()==expected, name
 state=json.loads((ROOT/'state.json').read_text())
 assert state['status']=='completed'
 assert state['source_before']==state['source_after'] and len(state['source_before'])==63
 assert state['identities_before']==state['identities_after']
 assert len(state['rows'])==3
 expected_tests=None
 for row in state['rows']:
  receipt=row['raw_output']
  raw=(ROOT/receipt['artifact']).read_bytes()
  assert hashlib.sha256(raw).hexdigest()==receipt['sha256']
  output='\n'.join(json.loads(line)['l'] for line in raw.decode().splitlines())
  marker='JR_RUNTIME_EXECUTED_39' if row['phase']=='edit' else None
  assert h.accepted(row,output,expected_tests,marker)
  expected_tests=h.test_results(output)
  assert (expected_tests['passed'],expected_tests['failed'],expected_tests['ignored'])==(4,0,0)
  assert len(row['profiles'])==1
  profile=row['profiles'][0]
  data=gzip.decompress((ROOT/(profile['path']+'.gz')).read_bytes())
  assert hashlib.sha256(data).hexdigest()==profile['sha256']
  events,strings=q.prepare(data,allow_unmapped_event_labels=True)
  metadata=q.metadata_from_strings(strings)
  assert metadata==profile['metadata'] and '--crate-name justrust ' in metadata['cmd']
  counts=dict.fromkeys(profile['markers'],0)
  unresolved=total=0
  for event in q.iter_prepared_records(events,strings,allow_unmapped_event_labels=True):
   total+=1
   if not event['label_resolved']:
    unresolved+=1
   elif event['label'] in counts:
    assert event['kind']=='GenericActivity' and event['record_type']=='interval'
    counts[event['label']]+=1
  assert counts==profile['markers']
  assert total==profile['records'] and unresolved==profile['unresolved_records']
  assert counts['jr_declarative_macro_eligible']==counts['jr_declarative_macro_cache_lookup']==counts['jr_declarative_macro_cache_success']==106
  assert counts['jr_declarative_macro_cache_provider']==(106 if row['phase']=='prepare' else 0)
  assert counts['jr_declarative_macro_ineligible']==(1858 if row['phase']=='edit' else 1856)
 print(json.dumps({'verdict':'passed','workflows':3,'eligible_per_workflow':106,'warm_provider_executions':0,'scope':'real cache reuse, not speedup'}))
if __name__=='__main__': main()
