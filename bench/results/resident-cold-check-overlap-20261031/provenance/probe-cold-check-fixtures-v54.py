#!/usr/bin/env python3
"""Original/OFF/ON real cold-check fixture observations, no timing claim."""
import argparse,json,sys,shutil
from pathlib import Path
sys.path.insert(0,'/home/jeremy/justrust/bench')
import resident_engine_compare as h
import resident_state_matrix as m
p=argparse.ArgumentParser(); p.add_argument('--candidate',type=Path,required=True); p.add_argument('--output',type=Path,required=True); a=p.parse_args()
ROOT=Path('/home/jeremy/.jcode/scratch/blaze-native-engine-20261030'); TC=Path('/home/jeremy/.blaze/toolchains/nightly-2026-10-27-x86_64-unknown-linux-gnu')
a.output.mkdir(mode=0o700,exist_ok=False)
frozen=ROOT/'cold-check-overlap-v54/fixtures'; source=a.output/'source'; shutil.copytree(frozen,source)
original=ROOT/'candidate-v43-bin/jr-engine'; tools={'original':original,'off':a.candidate,'on':a.candidate}
state={'scope':__doc__,'status':'incomplete','rows':[],'source_before':h.source_hashes(source),'tools':{k:h.runtime.base.digest(v) for k,v in tools.items()},'toolchain':h.toolchain_identity(TC)}
def save(): m.durable(a.output/'state.json',(json.dumps(state,indent=2)+'\n').encode())
save()
try:
 for case in ('valid','unused_error','reached_error','denied_lint','borrow_error','include_valid','include_error','include_module','macro_error','syntax_error'):
  for mode,tool in tools.items():
   name=case+'-'+mode; private=Path('/home/jeremy')/f'cf54{len(state["rows"]):02d}'; private.mkdir(mode=0o700,exist_ok=False)
   env=h.environment(private,'engine',TC); env.update(RA_JIT_DEMAND_STD='1',RA_JIT_STATS='1',RA_JIT_COLD_CHECK_OVERLAP='1' if mode=='on' else '0')
   prefix=a.output/(name+'-raw'); row={'case':case,'mode':mode,'startup_controls':{k:env[k] for k in ('RA_JIT_DEMAND_STD','RA_JIT_STATS','RA_JIT_COLD_CHECK_OVERLAP')},'private':str(private),'complete':False}; state['rows'].append(row);save()
   row.update(h.runtime.run_process(h.command(tool,'engine',operation='run',arguments=['--bin',case]),source,env,prefix,180))
   raw,text,error=m.captured(prefix);row.update(raw=raw,raw_error=error,complete=True)
   logs=list(Path(env['BLAZE_HOME']).glob('workers/*/log'));row['worker_logs']=[]
   for i,log in enumerate(logs):
    dest=a.output/(name+f'-worker-{i}.log');dest.write_bytes(log.read_bytes());row['worker_logs'].append(dest.name)
   row['post_observation_stop']=h.runtime.run_process([str(tool),'stop'],source,env,a.output/(name+'-stop'),30)
   save()
 state['status']='collected'
finally:
 state['source_after']=h.source_hashes(source);state['tools_after']={k:h.runtime.base.digest(v) for k,v in tools.items()};state['toolchain_after']=h.toolchain_identity(TC)
 before=state['source_before'];after=state['source_after'];state['source_unchanged_except_new_lock']=all(after.get(k)==v for k,v in before.items()) and set(after)-set(before)<={'Cargo.lock'}
 state['identities_unchanged']=state['tools']==state['tools_after'] and state['toolchain']==state['toolchain_after'];save()
assert state['source_unchanged_except_new_lock'] and state['identities_unchanged']
print(json.dumps({'status':state['status'],'rows':len(state['rows']),'claim':'observations only, semantic admission requires independent comparison'}))
