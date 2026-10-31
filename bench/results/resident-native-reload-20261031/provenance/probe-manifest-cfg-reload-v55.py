#!/usr/bin/env python3
"""Manifest-only cfg invalidation real native error/recovery, no performance claim."""
import argparse,json,sys,uuid
from pathlib import Path
sys.path.insert(0,'/home/jeremy/justrust/bench')
import resident_engine_compare as h
import resident_state_matrix as m
p=argparse.ArgumentParser();p.add_argument('--candidate',type=Path,required=True);p.add_argument('--output',type=Path,required=True);a=p.parse_args()
ROOT=Path('/home/jeremy/.jcode/scratch/blaze-native-engine-20261030'); TC=Path('/home/jeremy/.blaze/toolchains/nightly-2026-10-27-x86_64-unknown-linux-gnu')
a.output.mkdir(mode=0o700,exist_ok=False)
s={'scope':__doc__,'status':'incomplete','rows':[],'tools':{},'toolchain':h.toolchain_identity(TC)}
def save():m.durable(a.output/'state.json',(json.dumps(s,indent=2)+'\n').encode())
for i,mode in enumerate(('original','off','on')):
 tool=ROOT/'candidate-v43-bin/jr-engine' if mode=='original' else a.candidate;s['tools'][mode]=h.runtime.base.digest(tool)
 source=a.output/mode; (source/'src').mkdir(parents=True)
 literal='JR_GENERATION_ERROR_'+uuid.uuid4().hex
 rust=('fn main(){println!("GEN_OK_{}",std::env::var("GEN_NONCE").unwrap());}\n#[cfg(feature="bad")] fn unused_bad()->u32{"'+literal+'"}\n').encode()
 m.durable(source/'src/main.rs',rust,exclusive=True)
 base='[package]\nname="generation-fixture"\nversion="0.0.0"\nedition="2021"\n[workspace]\n[features]\nbad=[]\n'
 private=Path('/home/jeremy')/f'gf55{i}';private.mkdir(mode=0o700,exist_ok=False)
 env=h.environment(private,'engine',TC);env.update(RA_JIT_DEMAND_STD='1',RA_JIT_STATS='1',RA_JIT_WHOLE_RELOAD='1' if mode=='on' else '0')
 for phase in ('valid','manifest-error','recovery'):
  m.durable(source/'Cargo.toml',(base+('default=["bad"]\n' if phase=='manifest-error' else 'default=[]\n')).encode())
  nonce=uuid.uuid4().hex;prefix=a.output/(mode+'-'+phase);row={'mode':mode,'phase':phase,'nonce':nonce,'literal':literal,'startup_control':env['RA_JIT_WHOLE_RELOAD'],'rust_before':h.runtime.base.digest(source/'src/main.rs'),'accepted':False};s['rows'].append(row);save()
  row.update(h.runtime.run_process(h.command(tool,'engine',operation='run',arguments=[]),source,dict(env,GEN_NONCE=nonce),prefix,180))
  raw,text,error=m.captured(prefix);row.update(raw=raw,raw_error=error,rust_after=h.runtime.base.digest(source/'src/main.rs'))
  row['accepted']=not row['timed_out'] and not error and row['rust_before']==row['rust_after'] and ((row['status']==101 and 'E0308' in text and literal in text and 'GEN_OK_' not in text) if phase=='manifest-error' else (row['status']==0 and 'GEN_OK_'+nonce in text))
  row['status_receipt']=h.runtime.run_process([str(tool),'status'],source,env,a.output/(mode+'-'+phase+'-status'),30)
  for j,log in enumerate(Path(env['BLAZE_HOME']).glob('workers/*/log')):(a.output/(mode+'-'+phase+f'-worker-{j}.log')).write_bytes(log.read_bytes())
  daemon=Path(env['BLAZE_HOME'])/'daemon.log'
  if daemon.exists():(a.output/(mode+'-'+phase+'-daemon.log')).write_bytes(daemon.read_bytes())
  save()
 s.setdefault('stops',[]).append(h.runtime.run_process([str(tool),'stop'],source,env,a.output/(mode+'-stop'),30));save()
s['tools_after']={mode:h.runtime.base.digest(ROOT/'candidate-v43-bin/jr-engine' if mode=='original' else a.candidate) for mode in ('original','off','on')};s['toolchain_after']=h.toolchain_identity(TC);s['unchanged']=s['tools']==s['tools_after'] and s['toolchain']==s['toolchain_after'];s['status']='passed' if s['unchanged'] and all(r['accepted'] for r in s['rows']) else 'failed';save()
print(json.dumps({'status':s['status'],'rows':len(s['rows']),'accepted':[r['accepted'] for r in s['rows']]}));raise SystemExit(s['status']!='passed')
