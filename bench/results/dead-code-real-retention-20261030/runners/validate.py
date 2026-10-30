from pathlib import Path
import hashlib,json,sys
R=Path('/home/jeremy/justrust');O=R/'bench/results/dead-code-real-retention-20261030'
H=Path('/home/jeremy/.jcode/scratch/dead-code-real-retention-20261030')
def sha(p):
 with Path(p).open('rb') as f:return hashlib.file_digest(f,'sha256').hexdigest()
def load(n):return json.loads((O/(n+'.json')).read_text())
comparison=load('count-comparison');captures=comparison['captures'];assert len(captures)==8
identities=None
checks=[]
for c in captures:
 stem=c['mode']+'-'+c['epoch'];receipt=load(stem+'-receipt')
 assert receipt['status']==receipt['export_status']==0
 if identities is None:identities=receipt['identities']
 assert receipt['identities']==identities
 assert '3527393f3db7e09eeef270a83f51c5aeedb787c77f845ab76a76618f76ae137e' in identities.values()
 expected_source='380d2c460c21db44000355fda5628eec49a9411fb59b1a0af79de662950d6786' if c['epoch']=='body-edit' else '8272de3756d2323b05777c851b19c2cc6aec6bdf073af79056da29370daf6bde'
 assert receipt['source_sha256']==expected_source
 assert sha(receipt['profile'])==receipt['profile_sha256']
 assert sha(O/(stem+'.stdout'))==receipt['stdout_sha256']
 assert sha(O/(stem+'.stderr'))==receipt['stderr_sha256']
 assert len(receipt['analyses'])==6
 for tool in ('query_profile','reuse_ceiling','provider_cost'):
  assert receipt['analyses'][tool+'-strict']['status']==2
  assert receipt['analyses'][tool+'-unknown']['status']==0
 assert c['strict_replay']==dict(status='failed',type='ValueError',error='unmapped virtual string ID')
 assert not c['unknown_marker_arguments']
 flags=receipt['environment']['RUSTFLAGS'];assert '-Zincremental-verify-ich' in flags
 assert f'-Zjr-cache-dead-code-items={"yes" if c["mode"]=="on" else "no"}' in flags
 metadata=load(stem+'-query_profile-unknown')['metadata']['cmd']
 assert '-Zincremental-verify-ich' in metadata
 assert f'-Zjr-cache-dead-code-items={"yes" if c["mode"]=="on" else "no"}' in metadata
 for q in ('jr_dead_code_item_usage','jr_dead_code_item_usage_allow_expect'):
  actual=c['selected_records_by_kind'][q];markers=c['selected_native_markers'][q]
  if c['mode']=='off' or c['epoch']=='unchanged':assert actual==markers=={}
  elif c['epoch']=='prepare':
   n=3216 if q=='jr_dead_code_item_usage' else 180
   assert actual==dict(IncrementalResultHashing=n,Query=n) and markers=={}
  elif q=='jr_dead_code_item_usage':
   assert actual==dict(IncrementalLoadResult=3211,IncrementalResultHashing=5,Query=5)
   assert markers==dict(jr_incremental_green_query=3211,jr_incremental_provider_reexecuted_same_value=5)
  else:
   assert actual==dict(IncrementalLoadResult=180) and markers==dict(jr_incremental_green_query=180)
 checks.append(dict(capture=stem,status='passed'))
for item in load('raw-diagnostics-index')['runs']:
 assert item['diagnostics']['compile_errors']==item['diagnostics']['warnings']==0
 for filename,record in item['files'].items():assert sha(O/(item['mode']+'-'+item['epoch']+'-raw-'+filename))==record['sha256']
assert len(load('raw-diagnostics-index')['runs'])==8
manifest=load('source-manifest');assert manifest['restored_all_archived_tracked_files_match']
for f in manifest['files']:assert sha(H/'source'/f['path'])==f['sha256']
for mode in ('off','on'):
 records=[json.loads(line) for line in (O/f'exported-runs/xps14-dead-code-{mode}-20261030/2026-10.jsonl').read_text().splitlines()]
 assert len(records)==4 and all(r['exit']==0 for r in records)
validation=dict(verdict='passed',scope='Counts and correctness only, no elapsed performance inference or default promotion',checks=checks,real_checks=8,exports=8,strict_expected_failures=24,explicit_unknown_analyzer_successes=24,restored_source_files=len(manifest['files']),identities=identities,limitations=['Strict decode fails in all captures due to unmapped virtual string IDs.','Unknown labels are never reassigned to selected query names. No selected named query/load/marker records are observed on forced unchanged.','Flag-off has no item-query records because it keeps the original direct visitor route.','Count differences do not establish a speedup, total saved work, or a global reuse maximum.'])
(O/'validation.json').write_text(json.dumps(validation,indent=2)+'\n')
print(json.dumps(validation,indent=2))
