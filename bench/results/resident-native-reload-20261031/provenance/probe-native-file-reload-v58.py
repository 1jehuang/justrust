#!/usr/bin/env python3
"""Actual native C-file-only reload correctness with constant environment and link directives, not performance proof."""
import argparse,json,sys,uuid
from pathlib import Path
sys.path.insert(0,'/home/jeremy/justrust/bench')
import resident_engine_compare as h
import resident_state_matrix as m
p=argparse.ArgumentParser();p.add_argument('--engine',type=Path,required=True);p.add_argument('--output',type=Path,required=True);p.add_argument('--private',type=Path,required=True);a=p.parse_args()
a.output.mkdir(mode=0o700,exist_ok=False);source=a.output/'source';(source/'src').mkdir(parents=True);a.private.mkdir(mode=0o700,exist_ok=False)
files={'Cargo.toml':'[package]\nname="native-reload-probe"\nversion="0.0.0"\nedition="2021"\n[workspace]\n','src/main.rs':'extern "C"{fn native_value()->u32;}\nfn main(){println!("NATIVE_{}_{}",std::env::var("NATIVE_NONCE").unwrap(),unsafe{native_value()});}\n','a.c':'unsigned int native_value(void){return 11;}\n','b.c':'unsigned int native_value(void){return 22;}\n','build.rs':'''use std::{env,fs,path::PathBuf,process::Command};
fn main(){
 println!("cargo:rerun-if-env-changed=JR_NATIVE_CHOICE");
 println!("cargo:rerun-if-changed=a.c");println!("cargo:rerun-if-changed=b.c");
 let choice=env::var("JR_NATIVE_CHOICE").unwrap();assert!(choice=="a"||choice=="b");
 let out=PathBuf::from(env::var_os("OUT_DIR").unwrap()).join(&choice);fs::create_dir_all(&out).unwrap();
 let object=out.join("native.o");let archive=out.join("libnativechoice.a");
 assert!(Command::new("cc").args(["-c",&format!("{choice}.c"),"-o"]).arg(&object).status().unwrap().success());
 assert!(Command::new("ar").arg("crs").arg(&archive).arg(&object).status().unwrap().success());
 println!("cargo:rustc-link-search=native={}",out.display());println!("cargo:rustc-link-lib=static=nativechoice");
}
'''}
for name,text in files.items():m.durable(source/name,text.encode(),exclusive=True)
tc=Path('/home/jeremy/.blaze/toolchains/nightly-2026-10-27-x86_64-unknown-linux-gnu');env=h.environment(a.private,'engine',tc);env.update(RA_JIT_DEMAND_STD='1',RA_JIT_STATS='1')
s={'scope':__doc__,'status':'incomplete','rows':[],'source_before':h.source_hashes(source),'tool_before':h.runtime.base.digest(a.engine),'toolchain_before':h.toolchain_identity(tc),'private':str(a.private)}
def save():m.durable(a.output/'state.json',(json.dumps(s,indent=2)+'\n').encode())
save()
try:
 for index,value in enumerate((11,33,11)):
  choice='a';m.durable(source/'a.c',f'unsigned int native_value(void){{return {value};}}\n'.encode())
  nonce=uuid.uuid4().hex;prefix=a.output/f'{index}-{choice}';row={'choice':choice,'nonce':nonce,'expected':value,'source_at_call':h.source_hashes(source),'accepted':False};s['rows'].append(row);save()
  callenv=dict(env,JR_NATIVE_CHOICE=choice,NATIVE_NONCE=nonce)
  row.update(h.runtime.run_process(h.command(a.engine,'engine',operation='run',arguments=[]),source,callenv,prefix,180));raw,text,error=m.captured(prefix);row.update(raw=raw,raw_error=error);row['accepted']=row['status']==0 and not row['timed_out'] and not error and f'NATIVE_{nonce}_{row["expected"]}' in text
  row['status_receipt']=h.runtime.run_process([str(a.engine),'status'],source,env,a.output/f'{index}-{choice}-status',30)
  for j,log in enumerate(Path(env['BLAZE_HOME']).glob('workers/*/log')):(a.output/f'{index}-{choice}-worker-{j}.log').write_bytes(log.read_bytes())
  daemon=Path(env['BLAZE_HOME'])/'daemon.log'
  if daemon.exists():(a.output/f'{index}-{choice}-daemon.log').write_bytes(daemon.read_bytes())
  save()
 s['status']='passed' if all(r['accepted'] for r in s['rows']) else 'failed'
finally:
 m.durable(source/'a.c',files['a.c'].encode())
 s['source_after']=h.source_hashes(source);s['tool_after']=h.runtime.base.digest(a.engine);s['toolchain_after']=h.toolchain_identity(tc);s['source_unchanged_except_lock']=all(s['source_after'].get(k)==v for k,v in s['source_before'].items()) and set(s['source_after'])-set(s['source_before'])<={'Cargo.lock'};s['unchanged']=s['source_unchanged_except_lock'] and s['tool_before']==s['tool_after'] and s['toolchain_before']==s['toolchain_after'];s['stop']=h.runtime.run_process([str(a.engine),'stop'],source,env,a.output/'stop',30);save()
print(json.dumps({'status':s['status'],'accepted':[r['accepted'] for r in s['rows']],'unchanged':s['unchanged']}));raise SystemExit(s['status']!='passed' or not s['unchanged'])
