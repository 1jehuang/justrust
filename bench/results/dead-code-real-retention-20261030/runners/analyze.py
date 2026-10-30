from pathlib import Path
import collections,hashlib,json,subprocess,sys,traceback
H=Path('/home/jeremy/.jcode/scratch/dead-code-real-retention-20261030')
R=Path('/home/jeremy/justrust');O=R/'bench/results/dead-code-real-retention-20261030'
sys.path.insert(0,str(R/'bench'));import query_profile
SELECTED=('jr_dead_code_item_usage','jr_dead_code_item_usage_allow_expect')
def sha(p):
 with Path(p).open('rb') as f:return hashlib.file_digest(f,'sha256').hexdigest()
def save(n,v):(O/(n+'.json')).write_text(json.dumps(v,indent=2)+'\n')
records=[]
for epoch in ('prepare','unchanged','body-edit','restored'):
 for mode in ('off','on'):
  stem=mode+'-'+epoch;receipt=json.loads((O/(stem+'-receipt.json')).read_text());p=Path(receipt['profile'])
  assert sha(p)==receipt['profile_sha256']
  receipt['analyses']={}
  for tool in ('query_profile','reuse_ceiling','provider_cost'):
   for decode in ('strict','unknown'):
    name=f'{stem}-{tool}-{decode}';args=[sys.executable,R/f'bench/{tool}.py']
    if tool=='query_profile':args+=['--input',p,'--output',O/(name+'.json'),'--summary-only']
    else:args+=['--profile',p]
    args+=['--large-capture']
    if decode=='unknown':args+=['--allow-unmapped-event-labels']
    with (O/(name+'.stdout')).open('wb') as a,(O/(name+'.stderr')).open('wb') as b:
     rc=subprocess.run(list(map(str,args)),stdout=a,stderr=b,cwd=R).returncode
    receipt['analyses'][tool+'-'+decode]=dict(status=rc,command=list(map(str,args)))
    if decode=='unknown':assert rc==0,(name,rc)
  raw=query_profile.read_input(p)
  try:
   events,strings=query_profile.prepare(raw,False,True)
   for _ in query_profile.iter_prepared_records(events,strings,False):pass
   strict=dict(status='passed')
  except Exception as e:
   (O/(stem+'-strict-full-traceback.txt')).write_text(traceback.format_exc())
   strict=dict(status='failed',type=type(e).__name__,error=str(e))
  events,strings=query_profile.prepare(raw,True,True)
  selected={q:collections.Counter() for q in SELECTED};markers={q:collections.Counter() for q in SELECTED}
  unknown=collections.Counter();totals=collections.Counter();unknown_marker_arguments=collections.Counter()
  for e in query_profile.iter_prepared_records(events,strings,True):
   totals[e['kind']]+=1
   if not e['label_resolved']:unknown[e['kind']]+=1
   if e['label'] in selected:selected[e['label']][e['kind']]+=1
   label=e['label'] or ''
   if label in ('jr_incremental_green_query','jr_incremental_green_recompute') or label.startswith('jr_incremental_provider_'):
    try:parts=strings.resolve(e['event_id']).split('\x1e')
    except ValueError:
     unknown_marker_arguments[label]+=1;continue
    if len(parts)==2 and parts[1] in markers:markers[parts[1]][label]+=1
  records.append(dict(mode=mode,epoch=epoch,profile_sha256=receipt['profile_sha256'],source_sha256=receipt['source_sha256'],selected_records_by_kind={q:dict(v) for q,v in selected.items()},selected_native_markers={q:dict(v) for q,v in markers.items()},unknown_records_by_kind=dict(unknown),unknown_marker_arguments=dict(unknown_marker_arguments),all_records_by_kind=dict(totals),strict_replay=strict,query_cache=receipt['query_cache'],absent_labels='unobserved, not proof of zero work'))
  save(stem+'-receipt',receipt)
  print(json.dumps(dict(mode=mode,epoch=epoch,selected=records[-1]['selected_records_by_kind'],markers=records[-1]['selected_native_markers'],unknown=dict(unknown))),flush=True)
save('count-comparison',dict(schema=1,scope='Count and correctness observations only, no causal elapsed comparisons. Named labels and marker arguments are resolved per invocation, never by cross-session numeric query ID joins.',captures=records))
manifest=[]
for line in subprocess.check_output(['git','ls-tree','-r','f97fcc2'],cwd=R,text=True).splitlines():
 meta,path=line.split('\t',1);p=H/'source'/path
 if not p.is_file():continue
 blob=subprocess.check_output(['git','hash-object',p],cwd=R,text=True).strip();assert blob==meta.split()[2],path
 manifest.append(dict(path=path,git_object=blob,sha256=sha(p)))
save('source-manifest',dict(commit=subprocess.check_output(['git','rev-parse','f97fcc2'],cwd=R,text=True).strip(),restored_all_archived_tracked_files_match=True,files=manifest))
sysroot=Path('/home/jeremy/.jcode/scratch/dead-item-cache-20261030/sysroot')
save('identity',dict(source_commit=subprocess.check_output(['git','rev-parse','f97fcc2'],cwd=R,text=True).strip(),source_tree=subprocess.check_output(['git','rev-parse','f97fcc2^{tree}'],cwd=R,text=True).strip(),sysroot_libraries={str(p.relative_to(sysroot)):sha(p) for p in sorted((sysroot/'lib').rglob('*')) if p.is_file()},analyzers={str(p.relative_to(R)):sha(p) for p in (R/'bench/query_profile.py',R/'bench/reuse_ceiling.py',R/'bench/provider_cost.py')},private_runners={str(p):sha(p) for p in (H/'run.py',H/'analyze.py')}))
