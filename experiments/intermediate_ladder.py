#!/usr/bin/env python3
"""Counted fresh-output, warm-machine compiler ladder, retaining all failures."""
import hashlib,json,pathlib,subprocess,sys,tempfile,time
root=pathlib.Path(__file__).resolve().parent.parent
import argparse
parser=argparse.ArgumentParser(description=__doc__)
parser.add_argument('--binary',type=pathlib.Path,default=root/'target/debug/jr-rust')
parser.add_argument('--output',type=pathlib.Path,required=True)
args=parser.parse_args()
sys.path.insert(0,str(root/'experiments'))
from compiler import source_for,module_source_for
binary=args.binary.resolve(strict=True)
def run(argv):
 start=time.perf_counter_ns()
 try:
  p=subprocess.run([str(x) for x in argv],capture_output=True,timeout=60)
  return {'exit':p.returncode,'stdout':p.stdout.decode(errors='replace'),'stderr':p.stderr.decode(errors='replace'),'wall_ns':time.perf_counter_ns()-start}
 except subprocess.TimeoutExpired:return {'timeout':True,'wall_ns':time.perf_counter_ns()-start}
with args.output.open('x') as out,tempfile.TemporaryDirectory(prefix='jr-intermediate-') as temp:
 d=pathlib.Path(temp)
 def emit(row):out.write(json.dumps(row,sort_keys=True)+'\n');out.flush()
 emit({'record':'environment','revision':subprocess.check_output(['git','rev-parse','HEAD'],cwd=root,text=True).strip(),'binary_sha256':hashlib.sha256(binary.read_bytes()).hexdigest(),'rustc':run(['/usr/bin/rustc','-Vv']),'measurement':'fresh-process/fresh-output/warm-machine/uncontrolled contention; no overall speedup claim','repeats':3})
 for shape,generator in [('flat',source_for),('modules',module_source_for)]:
  for count in (4,16,64,128,256):
   text=generator(count);src=d/f'{shape}{count}.rs';src.write_text(text)
   for repeat in range(3):
    configs=[('stock',1,'build'),('native',1,'check'),('native',1,'build'),('native',8,'check'),('native',8,'build')]
    if repeat%2:configs.reverse()
    for tool,workers,mode in configs:
     exe=d/f'{shape}{count}-{repeat}-{tool}-{workers}-{mode}'
     args=([str(binary),str(src),'--workers',str(workers)]+(['--check'] if mode=='check' else ['-o',str(exe)])) if tool=='native' else ['/usr/bin/rustc','--edition=2024','--crate-name','reference','-C','opt-level=0','-C','overflow-checks=yes',str(src),'-o',str(exe)]
     row={'record':'sample','shape':shape,'functions':count,'repeat':repeat,'tool':tool,'workers':workers,'mode':mode,'source':text,'source_sha256':hashlib.sha256(text.encode()).hexdigest(),'compile':run(args),'artifact':exe.exists()}
     if row['compile'].get('exit')==0 and mode=='build':
      row['runtime']=run([exe]);row['correct']=row['runtime'].get('exit')==0 and row['runtime']['stdout']=='compiled correctly\n' and row['runtime']['stderr']==''
     elif row['compile'].get('exit')==0:row['correct']=not exe.exists()
     else:row['correct']=False
     emit(row)
   print(shape,count,'complete',flush=True)
