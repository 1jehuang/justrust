#!/usr/bin/env python3
"""Unchanged example programs through real stock and candidate CLI, not a coverage claim."""
import hashlib,json,pathlib,subprocess,tempfile
root=pathlib.Path(__file__).resolve().parent.parent
import argparse
parser=argparse.ArgumentParser(description=__doc__)
parser.add_argument('--binary',type=pathlib.Path,default=root/'target/debug/jr-rust')
parser.add_argument('--output',type=pathlib.Path,required=True)
args=parser.parse_args()
out=args.output
binary=args.binary.resolve(strict=True)
def run(argv):
 try:
  p=subprocess.run([str(x) for x in argv],capture_output=True,timeout=60)
  return {'argv':[str(x) for x in argv],'exit':p.returncode,'stdout':p.stdout.decode(errors='replace'),'stderr':p.stderr.decode(errors='replace')}
 except subprocess.TimeoutExpired:
  return {'argv':[str(x) for x in argv],'timeout':True}
with out.open('x') as f,tempfile.TemporaryDirectory(prefix='jr-intermediate-') as temp:
 d=pathlib.Path(temp)
 def emit(row): f.write(json.dumps(row,sort_keys=True)+'\n');f.flush()
 emit({'record':'environment','revision':subprocess.check_output(['git','rev-parse','HEAD'],cwd=root,text=True).strip(),'binary_sha256':hashlib.sha256(binary.read_bytes()).hexdigest(),'stock':run(['/usr/bin/rustc','-Vv']),'scope':'unchanged repository example gallery; not external Cargo ecosystem coverage'})
 for i,source in enumerate(sorted((root/'examples/compiler').glob('*.rs'))):
  stock=d/f'stock{i}'
  row={'record':'program','source':str(source.relative_to(root)),'source_sha256':hashlib.sha256(source.read_bytes()).hexdigest(),'stock_compile':run(['/usr/bin/rustc','--edition=2024','-C','overflow-checks=yes',source,'-o',stock]),'candidate':[]}
  if row['stock_compile'].get('exit')==0:row['stock_runtime']=run([stock])
  for workers in (1,8):
   for mode in ('check','build'):
    target=d/f'ours{i}-{workers}'
    command=[binary,source,'--workers',workers]+(['--check'] if mode=='check' else ['-o',target])
    result=run(command);result.update(workers=workers,mode=mode,artifact=target.exists())
    if mode=='build' and result.get('exit')==0 and target.exists():
     result['runtime']=run([target]);r=result['runtime'];s=row.get('stock_runtime',{})
     result['matches_stock']=all(r.get(k)==s.get(k) for k in ('exit','stdout','stderr'))
    row['candidate'].append(result)
  row['passed']=row['stock_compile'].get('exit')==0 and all(r.get('exit')==0 and (r['mode']=='check' or r.get('matches_stock') is True) for r in row['candidate'])
  emit(row);print(source.name,'PASS' if row['passed'] else 'FAIL',flush=True)
