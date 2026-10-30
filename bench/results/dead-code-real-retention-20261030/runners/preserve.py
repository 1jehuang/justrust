from pathlib import Path
import hashlib,json,shutil
H=Path('/home/jeremy/.jcode/scratch/dead-code-real-retention-20261030');O=Path('/home/jeremy/justrust/bench/results/dead-code-real-retention-20261030')
def sha(p):
 with Path(p).open('rb') as f:return hashlib.file_digest(f,'sha256').hexdigest()
result=[]
for mode in ('off','on'):
 exported=O/f'exported-runs/xps14-dead-code-{mode}-20261030/2026-10.jsonl'
 records=[json.loads(line) for line in exported.read_text().splitlines()];assert len(records)==4
 for epoch,record in zip(('prepare','unchanged','body-edit','restored'),records):
  stem=mode+'-'+epoch;run=H/mode/'home/runs'/record['id']
  assert record['exit']==0 and record['diagnostics']['compile_errors']==0 and record['diagnostics']['warnings']==0
  files={}
  for filename in ('output.jsonl','meta.json','summary.json'):
   target=O/(stem+'-raw-'+filename);shutil.copyfile(run/filename,target);files[filename]=dict(source_path=str(run/filename),sha256=sha(target),bytes=target.stat().st_size)
  result.append(dict(mode=mode,epoch=epoch,run_id=record['id'],diagnostics=record['diagnostics'],files=files))
(O/'raw-diagnostics-index.json').write_text(json.dumps(dict(scope='Unmodified full recorded cargo/compiler output plus meta and summary. All eight checks have no compiler errors or warnings. Reports/verbose commands contain private paths and non-comparable timing observations, not normalized diagnostic equality claims.',runs=result),indent=2)+'\n')
print('Preserved full raw output/meta/summary for eight runs, all diagnostics counts zero')
