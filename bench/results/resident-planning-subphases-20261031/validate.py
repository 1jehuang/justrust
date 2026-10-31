#!/usr/bin/env python3
"""Standalone offline v79 attribution replay. No engine, imported driver or Git.
SHA manifest is integrity, not authenticity. Reviewed fixed historical commitments
are checked AFTER independent semantic joins, never in place of them.
"""
import argparse
import hashlib
import json
import math
import re
from pathlib import Path

OBSERVATION = '5651e2a8576de19eb717e19122edf137ea67dfa81baf41ea1f968da741beed71'
HISTORICAL = '03e21704f58058841f54bc9bcb7a846bfefca967d4d30d043745cf486f249060'
LABELS = ('client planning','client planning rustc host','client planning target cfg','client planning asked setup','client planning owned resolver')
CONTRACT = {'comparison':False,'speed_claim':False,'default_promotion':False,'noise':'unknown','inclusive':True,'quiet_machine_established':False}
NAMES = ('parses_libtest_results','shares_overlapping_time','splits_passes_without_double_counting','test_failures_are_not_compile_errors')
ADDITIONS = (
'        let host_phase = super::Phase::new("client planning rustc host");\n',
'        drop(host_phase);\n',
'        let cfg_phase = super::Phase::new("client planning target cfg");\n',
'        drop(cfg_phase);\n',
'        let setup_phase = super::Phase::new("client planning asked setup");\n',
'        drop(setup_phase);\n',
'        let _resolver_phase = super::Phase::new("client planning owned resolver");\n')

def sha(raw): return hashlib.sha256(raw).hexdigest()
def require(ok,why):
    if not ok: raise ValueError(why)
def relative(name):
    require(isinstance(name,str) and name and not Path(name).is_absolute() and '..' not in Path(name).parts and '\\' not in name,'relative path only')

def safe(root,name):
    relative(name); path=root/name
    for i in range(1,len(Path(name).parts)+1):
        require(not root.joinpath(*Path(name).parts[:i]).is_symlink(),'symlink not permitted')
    return path

def patch(original,text):
    lines=text.splitlines(True); require(lines[0].startswith('--- ') and lines[1].startswith('+++ '),'patch headers')
    old=original.splitlines(True); result=[]; cursor=0; index=2
    while index<len(lines):
        m=re.fullmatch(r'@@ -(\d+)(?:,(\d+))? \+(\d+)(?:,(\d+))? @@\n',lines[index]); require(m is not None,'patch hunk')
        pos=max(int(m[1])-1,0); require(cursor<=pos<=len(old),'ordered patch')
        result.extend(old[cursor:pos]); cursor=pos; require(len(result)==max(int(m[3])-1,0),'patch new offset'); index+=1
        removed=added=0
        while index<len(lines) and not lines[index].startswith('@@ '):
            kind=lines[index][0]; line=lines[index][1:]; require(kind in ' +-','patch prefix')
            if kind in ' -':
                require(cursor<len(old) and old[cursor]==line,'exact patch context'); cursor+=1; removed+=1
            if kind in ' +': result.append(line); added+=1
            index+=1
        require(removed==int(m[2] or 1) and added==int(m[4] or 1),'patch hunk counts')
    result.extend(old[cursor:]); return ''.join(result)

def edit(original,n):
    anchor=b'fn shares_overlapping_time() {'
    require(original.count(anchor)==1,'unique actual runtime anchor')
    body=(f'let observed = std::hint::black_box({n}u64); assert_eq!(observed, {n}u64); println!("JR_RUNTIME_EXECUTED_{n}");').encode()
    return original.replace(anchor,anchor+b'\n'+body,1)

def process(row):
    require(type(row['status']) is int and row['status']==0 and row['error'] is None and row['timed_out'] is False,'successful actual process')
    require(type(row['elapsed_seconds']) in (float,int) and math.isfinite(row['elapsed_seconds']) and row['elapsed_seconds']>0,'positive finite elapsed')

def semantic(root):
    load=lambda name:json.loads(safe(root,name).read_text())
    data=load('observations.json'); receipts=load('private-inputs.json')
    require(set(data)=={'authority','scope','contract','status','unchanged','tool_before','tool_after','prepare','stop','rows','gates','native','harness_count','ranges_us','export_original'},'closed observation allowlist')
    require(len(data['native'])==2,'exact two native gates')
    require(data['authority']=='34dc0a188' and data['contract']==CONTRACT,'bounded immutable attribution contract')
    require(data['scope']=='Actual accepted native warm-path spans, inclusive diagnostic, not a speed comparison.','original scope')
    require(data['status']=='passed' and data['unchanged'] is True,'accepted unchanged profile')
    require(data['tool_before']==data['tool_after']=='8f5b51734bdeedf6a4b0370efbe2b1f8bb916c91ff957e929e68dd707e81af6c','same native engine')
    require(all(set(data[name])=={'status','error','timed_out','elapsed_seconds'} for name in ('prepare','stop')),'closed prepare/stop allowlist')
    process(data['prepare']); process(data['stop'])
    for name,r in receipts.items():
        relative(name)  # Logical receipt names only, never read private inputs.
        require(set(r)=={'sha256','bytes'} and re.fullmatch('[0-9a-f]{64}',r['sha256']) and type(r['bytes']) is int and r['bytes']>=0,'original private input SHA/size')
    source=load('inventories/pinned-source.json')
    require(source['before']==source['after'] and len(source['before'])==63,'unchanged source63')
    tc=load('inventories/toolchain.json'); require(len(tc)==4020,'toolchain4020')
    inv=load('inventories/native-source.json'); require(len(inv)==3757,'native full source3757 inventory')
    for inventory in (source['before'],inv):
        for name,digest in inventory.items():
            relative(name); require(re.fullmatch('[0-9a-f]{64}',digest),'source inventory digest')
    for name,item in tc.items():
        relative(name); require(set(item)=={'sha256','link'} and re.fullmatch('[0-9a-f]{64}',item['sha256']),'toolchain hash shape')
        if isinstance(item['link'],str): require(not item['link'].startswith('/') and '\\' not in item['link'],'toolchain relative link')
        elif item['link'] is not None: require(set(item['link'])=={'role','sha256','bytes'} and item['link']['role']=='<TOOLCHAIN_LINK>','role-redacted absolute link')
    joins=load('inventories/joins.json')
    encoded=lambda value:(json.dumps(value,sort_keys=True,indent=2)+'\n').encode()
    require(joins['authority_commit']=='34dc0a188534edbe92d0b848b9dfa68de71d8bb3','full immutable accepted commit')
    require(joins['native_after_matches_before'] is True and joins['native_after_ok']==joins['native_before_count']==len(inv) and joins['native_before_sha256']==sha(encoded(inv)),'native3757 before/after inventory join')
    require(joins['toolchain_before_count']==joins['toolchain_after_count']==len(tc) and joins['toolchain_before_sha256']==joins['toolchain_after_sha256']==sha(encoded(tc)),'toolchain4020 before/after join')
    require(joins['pinned_before_sha256']==sha(encoded(source['before'])) and joins['pinned_after_sha256']==sha(encoded(source['after'])),'pinned63 before/after join')
    original=safe(root,'source/originals.rs').read_text(); candidate=safe(root,'source/candidates.rs').read_text()
    require(patch(original,safe(root,'source/metadata.patch').read_text())==candidate,'full exact native patch replay')
    restored=candidate
    for line in ADDITIONS:
        require(restored.count(line)==1,'exact seven unique source additions'); restored=restored.replace(line,'',1)
    require(restored==original,'seven additions recover complete original')
    require(inv['crates/ra-jit/src/cargo/metadata.rs']==sha(candidate.encode()),'native inventory joins frozen telemetry source')
    require(inv['crates/ra-jit/src/cargo/mod.rs']==sha(safe(root,'source/cargo-mod.rs').read_bytes()),'existing Phase source joins native gate')
    guard=safe(root,'source/cargo-mod.rs').read_text()
    require('telemetry_enabled().then(Instant::now)' in guard and 'if let Some(started) = self.started' in guard and 'static ENABLED: std::sync::OnceLock<bool>' in guard,'disabled lazy clock and RAII guard source')
    initial=safe(root,'source/initial-doc.md').read_text()
    before=safe(root,'source/finaldoc/originals/docs/resident-planning-subphases.md').read_text()
    after=safe(root,'source/finaldoc/candidates/docs/resident-planning-subphases.md').read_text()
    require(initial==before and patch(before,safe(root,'source/finaldoc/candidate.patch').read_text())==after,'frozen initial to finaldoc exact patch')
    require(after==safe(root,'source/accepted-doc.md').read_text(),'accepted immutable documentation join')
    base=safe(root,'runtime/source.original').read_bytes(); require(sha(base)==source['before']['src/summary.rs'],'runtime original joins source63')
    rows=data['rows']; require([(r['replicate'],r['kind']) for r in rows]==[(i,k) for i in range(4) for k in ('edit','noop')],'eight ordered actual runtime rows')
    durations={label:[] for label in LABELS}; markers=set()
    for row in rows:
        require(set(row)=={'status','error','timed_out','elapsed_seconds','kind','replicate','marker','source_sha256','trace_sha256','events','stdout','noise_contamination','public_pid_count'},'closed runtime row allowlist')
        process(row); rep=row['replicate']; kind=row['kind']; stem=f'{rep}-{kind}'
        require(row['stdout']=='runtime/'+stem+'.stdout' and row['marker']==f'JR_RUNTIME_EXECUTED_{7901+rep}','nonce and stdout row join')
        require(row['source_sha256']==sha(edit(base,7901+rep)),'exact independently reconstructed runtime source edit')
        stdout=safe(root,row['stdout']).read_bytes(); text=stdout.decode()
        receipt=receipts['planning-subphases-profile-v79/'+stem+'.stdout']
        require(receipt=={'sha256':sha(stdout),'bytes':len(stdout)},'raw actual stdout receipt')
        require(re.findall(r'JR_RUNTIME_EXECUTED_\d+',text)==[row['marker']],'exact once runtime nonce')
        markers.add(row['marker'])
        require('test result: ok. 4 passed; 0 failed; 0 ignored; 0 measured; 144 filtered out;' in text,'actual four-test runtime success')
        require(re.findall(r'test summary::tests::([a-z_]+) \.\.\.',text)==list(NAMES),'exact four named runtime tests')
        expected='\nrunning 4 tests\n'+''.join('test summary::tests::'+name+' ... '+(row['marker']+'\nok' if name=='shares_overlapping_time' else 'ok')+'\n' for name in NAMES)+'\ntest result: ok. 4 passed; 0 failed; 0 ignored; 0 measured; 144 filtered out; finished in 0.00s\n\n'
        require(text==expected,'closed actual stdout line grammar')
        require(row['noise_contamination']=='unknown' and type(row['public_pid_count']) is int and row['public_pid_count']==(3 if kind=='edit' else 2),'noise unknown and public trace process count')
        require(row['trace_sha256']==receipts['planning-subphases-profile-v79/'+stem+'.trace.json']['sha256'],'original trace SHA join')
        events=row['events']; require(len(events)==5 and {e['name'] for e in events}==set(LABELS),'each actual planning label exactly once')
        byname={e['name']:e for e in events}; parent=byname[LABELS[0]]; last=parent['ts']
        for label in LABELS:
            e=byname[label]
            require(set(e)=={'name','dur','ts','pid','tid','ph','cat'} and e['ph']=='X' and e['cat']=='step','fixed trace event projection')
            require(all(type(e[k]) is int and e[k]>0 for k in ('dur','ts','pid','tid')),'actual positive integer trace fields')
            require(e['pid']==parent['pid'] and e['tid']==parent['tid'],'same client trace thread')
            durations[label].append(e['dur'])
            if label!=LABELS[0]:
                require(parent['ts']<=e['ts'] and e['ts']+e['dur']<=parent['ts']+parent['dur'],'inclusive parent containment')
                require(e['ts']>=last,'successive nonoverlapping planning subphases'); last=e['ts']+e['dur']
        require(sum(byname[l]['dur'] for l in LABELS[1:])<=parent['dur'],'inclusive not additive parent')
    require(len(markers)==4,'four distinct runtime edit nonces, repeated by corresponding noop')
    ranges={label:[min(values),max(values)] for label,values in durations.items()}
    require(ranges==data['ranges_us'],'independently reconstructed event ranges')
    records=load('gates/recorder.json'); require(len(records)==4 and [r['subcommand'] for r in records]==['check','build','test','test'],'four original exported runs')
    for row in records: require(row['exit']==0 and row['tests']['failed']==0 and row['tests']['ignored']==0,'actual native run exit and failures')
    require([r['tests']['passed'] for r in records]==[0,0,53,373],'native53 and resident373 gates')
    require([r['args'] for r in records]==[
        ['check','--release','--locked','-p','ra-jit','--tests','-p','hir-ty-codegen','-j','6'],
        ['build','--release','--locked','-p','ra-jit','--bin','jr-engine','-j','6'],
        ['test','--release','--locked','-p','hir-ty-codegen','-j','6'],
        ['test','--release','--locked','-p','ra-jit','--bin','jr-engine','-j','6']],'exact native gate arguments')
    for gate,row,count in zip(data['native'],records[2:],(53,373)):
        require(gate['passed']==count and gate['run_id']==row['id'],'native gate and recorder join')
        lines=safe(root,'gates/'+gate['name']+'-named.txt').read_text().splitlines()
        matches=[re.fullmatch(r'test ([A-Za-z0-9_:]+) \.\.\. ok(?: <[0-9.]+s>)?',line) for line in lines]
        require(len(lines)==count and all(matches) and len({m[1] for m in matches})==count,'all exact named native PASS lines')
        text=safe(root,'gates/'+gate['name']+'.txt').read_text()
        require(f'{count} passed, 0 failed, 0 ignored, 0 filtered out' in text and row['id'] in text,'selected native report joins counts and run')
    expected={'candidate-build-v79/check.exit','candidate-build-v79/compile.exit','candidate-build-v79/orchestration-exit','candidate-unit-v79/codegen-native.exit','candidate-unit-v79/full-native.exit','candidate-unit-v79/exit'}
    require(set(data['gates'])==expected|{'candidate-build-v79/lock-before.sha256','candidate-unit-v79/lock-after.log'},'closed gate receipt set')
    require(all(data['gates'][name]=='0' for name in expected),'successful native gate receipts')
    lock=data['gates']['candidate-build-v79/lock-before.sha256']; require(re.fullmatch('[0-9a-f]{64}  Cargo.lock',lock) and data['gates']['candidate-unit-v79/lock-after.log']=='Cargo.lock: OK','native lock before/after')
    require(data['harness_count']==761 and re.fullmatch(r'Ran 761 tests in 51\.426s\n\nOK\n',safe(root,'gates/harness.txt').read_text()),'actual historical Python761 gate')
    return {'rows':8,'runtime_tests_per_row':4,'distinct_edit_nonces':4,'native_tests':[53,373],'source_files':3757,'pinned_source_files':63,'toolchain_files':4020,'harness_tests':761,'ranges_us':ranges,'contract':CONTRACT}

def validate(root,manifest=True,historical=True):
    root=root.resolve()
    files={p.relative_to(root).as_posix() for p in root.rglob('*') if p.is_file()}
    require(not any(p.is_symlink() for p in root.rglob('*')),'no archive symlinks')
    require(all(p.is_file() or p.is_dir() for p in root.rglob('*')),'no special archive files')
    if manifest:
        entries=json.loads((root/'SHA256.json').read_text()); require(set(entries)==files-{'SHA256.json'},'complete closed archive seal')
        for name,receipt in entries.items():
            raw=safe(root,name).read_bytes(); require(receipt=={'sha256':sha(raw),'bytes':len(raw)},'sealed file '+name)
    # Privacy is checked on every text artifact including executable authoring tools.
    private_path=re.compile(r'(?<![A-Za-z0-9_./-])/(?:home|Users|tmp|root)/')
    forbidden=('BEGIN '+'PRIVATE KEY','AWS_SECRET_'+'ACCESS_KEY=','environment'+chr(34)+':','BLAZE_CACHE'+chr(34)+':','CARGO_HOME'+chr(34)+':')
    for name in files:
        text=safe(root,name).read_text(); require(not private_path.search(text) and not any(s in text for s in forbidden),'privacy allowlist '+name)
    result=semantic(root)
    if historical:
        require(sha((root/'observations.json').read_bytes())==OBSERVATION,'fixed historical observation commitment')
        evidence={name:sha(safe(root,name).read_bytes()) for name in files if Path(name).parts[0] in ('source','runtime','lineage','inventories','gates') or name=='private-inputs.json'}
        require(sha(json.dumps(evidence,sort_keys=True,separators=(',',':')).encode())==HISTORICAL,'fixed historical source/receipt commitment')
    return result

def main():
    parser=argparse.ArgumentParser(); parser.add_argument('--manifest',action='store_true'); args=parser.parse_args()
    print(json.dumps(validate(Path(__file__).resolve().parent,manifest=args.manifest),sort_keys=True,indent=2))
if __name__=='__main__': main()
