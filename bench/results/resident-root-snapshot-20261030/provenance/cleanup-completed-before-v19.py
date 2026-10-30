#!/usr/bin/env python3
"""Stop only completed, receipt-identified private benchmark daemons, without deleting stores."""
import hashlib,json,os,subprocess,time
from pathlib import Path
ROOT=Path('/home/jeremy/.jcode/scratch/blaze-native-engine-20261030')
OUT=Path('/home/jeremy/.jcode/scratch/resident-private-cleanup-before-v19-20261030.json')
CASES=['rsb02','rsa07','rsa08','rsb03','rsa09','rsb04','rsa10','rsb05','rsa11','rsb06','rsa12','rsb07','rsa13','rsb08','rsa14','rsa04','rsa05']
rows=[]
for case in CASES:
 base=Path('/home/jeremy')/case
 meta=json.loads((base/'metadata.json').read_text())
 assert meta['status']=='complete' and meta['source_restored'] and meta['unchanged'],case
 for stack in ['reference','engine']:
  receipt=meta['tools_before'][stack]; binary=Path(receipt['path'])
  assert binary.is_relative_to(ROOT) and not binary.is_symlink(),binary
  assert hashlib.sha256(binary.read_bytes()).hexdigest()==receipt['sha256'],binary
  declared=meta['environments'][stack]
  for key in ['BLAZE_HOME','BLAZE_CACHE','RA_JIT_HOME','RA_JIT_CACHE']:
   path=Path(declared[key]); assert path.is_relative_to(base/stack) and path.resolve().is_relative_to((base/stack).resolve()),(case,key)
  env={key:value for key,value in os.environ.items() if key in ['PATH','LANG','LC_ALL','TERM','USER','LOGNAME','SHELL']}
  env.update(declared)
  env.update({key:str(base/stack/folder) for key,folder in [('XDG_CACHE_HOME','cache'),('XDG_CONFIG_HOME','config'),('XDG_DATA_HOME','data'),('XDG_RUNTIME_DIR','runtime')]})
  command=[str(binary)]+(['cargo','stop'] if stack=='reference' else ['stop'])
  result=subprocess.run(command,cwd=base,env=env,text=True,capture_output=True,timeout=30)
  rows.append(dict(case=case,stack=stack,command=command,tool_sha256=receipt['sha256'],namespace={k:declared[k] for k in ['BLAZE_HOME','RA_JIT_HOME']},status=result.returncode,stdout=result.stdout,stderr=result.stderr))
  OUT.write_text(json.dumps(dict(scope='native stop receipts, not independent process-exit attestation; no stores deleted',rows=rows),indent=2)+'\n')
  assert result.returncode==0,(case,stack,result.stderr)
print(json.dumps(dict(status='passed',namespaces=len(rows),receipt=str(OUT))))
