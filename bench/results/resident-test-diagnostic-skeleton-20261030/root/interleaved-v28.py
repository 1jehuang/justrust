#!/usr/bin/env python3
"""Extended interleaved warm edit diagnostic, separate from fixed five-sample suite."""
import hashlib,itertools,json,os,statistics,sys
from pathlib import Path
sys.path.insert(0,'/home/jeremy/justrust/bench')
import resident_engine_compare as h
import blaze_runtime_compare as r
OUT=Path('/home/jeremy/.jcode/scratch/resident-interleaved-v28-20261030')
OUT.mkdir(mode=0o700,exist_ok=False)
SOURCE=Path('/home/jeremy/.jcode/scratch/ba30/source')
TC=Path('/home/jeremy/.blaze/toolchains/nightly-2026-10-27-x86_64-unknown-linux-gnu')
h.pinned_source(SOURCE)
src=SOURCE/'src/summary.rs';original=src.read_bytes();(OUT/'original.rs').write_bytes(original)
state=dict(scope='extended interleaved warm-store diagnostic, not cold support, comprehensive gate, significance or default promotion',rows=[],source_before=h.source_hashes(SOURCE),toolchain_before=h.toolchain_identity(TC),sampling='42 identical-byte edits across all six stack permutations, no sample exclusion')
configs={}
for name,base,stack in [('old','/home/jeremy/rsb18','engine'),('new','/home/jeremy/rsa24','engine'),('blaze','/home/jeremy/rsa24','reference')]:
 base=Path(base);meta=json.loads((base/'metadata.json').read_text())
 assert meta['status']=='complete' and meta['unchanged'] and meta['source_restored']
 env={k:v for k,v in os.environ.items() if not k.startswith(('JUSTRUST_','JR_ENGINE_','RA_JIT_','BLAZE_','CARGO_','RUST','XDG_'))}
 env.update(meta['environments'][stack])
 for var,folder in [('RUNTIME','runtime'),('CACHE','cache'),('CONFIG','config'),('DATA','data')]:env['XDG_'+var+'_HOME' if var!='RUNTIME' else 'XDG_RUNTIME_DIR']=str(base/stack/folder)
 configs[name]=(Path(meta['tools_before'][stack]['path']),stack,env,meta['tools_before'][stack]['sha256'],meta['expected_tests'])
state['configs']={name:dict(tool=str(c[0]),kind=c[1],environment=r.base.logged_environment(c[2]),sha256=c[3],expected=c[4]) for name,c in configs.items()}
def invoke(name,phase,rep,contents,marker=None,failure=None):
 tool,kind,env,identity,expected=configs[name]
 assert r.base.digest(tool)==identity
 src.write_bytes(contents)
 prefix=OUT/f'{phase}-{rep}-{name}'
 row=r.run_process(h.command(tool,kind),SOURCE,env,prefix,120)
 text='\n'.join(prefix.with_suffix('.'+suffix).read_text() for suffix in ('stdout','stderr'))
 parsed=r.test_results(text)
 if failure=='type':ok=row['status']==101 and not row['error'] and not row['timed_out'] and 'E0308' in text and 'JR_RUNTIME_TYPE_ERROR' in text
 elif failure=='assert':ok=row['status']==101 and not row['error'] and not row['timed_out'] and h.valid_tests(parsed,expected,failing=True) and 'JR_RUNTIME_FAILURE' in text
 else:ok=r.base.succeeded(row) and h.valid_tests(parsed,expected) and (marker is None or text.count(marker)==1)
 row.update(stack=name,phase=phase,replicate=rep,accepted=ok,parsed=parsed,edit_sha256=hashlib.sha256(contents).hexdigest(),stdout_sha256=r.base.digest(prefix.with_suffix('.stdout')),stderr_sha256=r.base.digest(prefix.with_suffix('.stderr')))
 state['rows'].append(row)
 if not ok:raise ValueError(f'gate failed {name}/{phase}/{rep}')
try:
 for name in configs:
  invoke(name,'prepare',0,original)
  invoke(name,'type-error',0,r.edit_bytes(original,kind='type-error'),failure='type')
  invoke(name,'type-recovery',0,original)
  invoke(name,'assert-failure',0,r.edit_bytes(original,kind='test-failure'),failure='assert')
  invoke(name,'assert-recovery',0,original)
 permutations=list(itertools.permutations(configs))
 for rep in range(42):
  contents=r.edit_bytes(original,rep+2800)
  for name in permutations[rep%len(permutations)]:invoke(name,'edit',rep,contents,f'JR_RUNTIME_EXECUTED_{rep+2801}')
 state['medians_seconds']={name:statistics.median(row['elapsed_seconds'] for row in state['rows'] if row['phase']=='edit' and row['stack']==name) for name in configs}
 state['status']='passed'
except BaseException as error:
 state.update(status='failed',error=repr(error));raise
finally:
 src.write_bytes(original)
 state['source_after']=h.source_hashes(SOURCE)
 state['toolchain_after']=h.toolchain_identity(TC)
 state['identities_unchanged']=all(r.base.digest(c[0])==c[3] for c in configs.values())
 state['source_restored']=state['source_before']==state['source_after']
 state['toolchain_unchanged']=state['toolchain_before']==state['toolchain_after']
 (OUT/'state.json').write_text(json.dumps(state,indent=2)+'\n')
assert state['source_restored'] and state['identities_unchanged'] and state['toolchain_unchanged']
print(json.dumps(dict(status=state['status'],rows=len(state['rows']),medians_seconds=state['medians_seconds'],scope=state['scope']),indent=2))
