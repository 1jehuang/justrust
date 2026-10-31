#!/usr/bin/env python3
"""Read-only authoring export. Not used by portable replay. Explicit inputs only."""
import argparse
import ast
import hashlib
import json
import re
import subprocess
from pathlib import Path

AUTHORITY = '34dc0a188'

def sha(b): return hashlib.sha256(b).hexdigest()
def encode(x): return (json.dumps(x, sort_keys=True, indent=2)+'\n').encode()
def main():
    p=argparse.ArgumentParser()
    for name in ('root','source','toolchain','bench','state'): p.add_argument('--'+name,type=Path,required=True)
    a=p.parse_args()
    root=a.root; out=Path(__file__).resolve().parent; receipts={}
    def read(path):
        b=path.read_bytes(); receipts[str(path.relative_to(root))]={'sha256':sha(b),'bytes':len(b)}; return b
    def put(name,b):
        target=out/name; target.parent.mkdir(parents=True,exist_ok=True); target.write_bytes(b)
    def git(path): return subprocess.check_output(['git','show',AUTHORITY+':'+path])
    state=json.loads(read(root/'planning-subphases-profile-v79/state.json'))
    # Only explicit fixed source files, not an imported source snapshot.
    metadata='toolchain/resident/crates/ra-jit/src/cargo/metadata.rs'
    for role in ('originals','candidates'):
        put('source/'+role+'.rs',read(root/'planning-subphases-v79'/role/metadata))
    assert (out/'source/candidates.rs').read_bytes()==git(metadata)
    put('source/cargo-mod.rs',git('toolchain/resident/crates/ra-jit/src/cargo/mod.rs'))
    patch=read(root/'planning-subphases-v79/candidate.patch').decode().split('--- /dev/null')[0]
    put('source/metadata.patch',patch.encode())
    put('source/sourcechecks.txt',read(root/'planning-subphases-v79/sourcechecks.txt'))
    # Preserve full frozen documentation chain, no private-path source rewrite.
    for name in ('candidate.patch','RELEASE.txt','SHA256SUMS'):
        read(root/'planning-subphases-v79'/name)
    for role in ('originals','candidates'):
        for f in sorted((root/'planning-subphases-v79/finaldoc'/role).rglob('*')):
            if f.is_file(): put('source/finaldoc/'+role+'/'+f.relative_to(root/'planning-subphases-v79/finaldoc'/role).as_posix(),read(f))
    put('source/finaldoc/candidate.patch',read(root/'planning-subphases-v79/finaldoc/candidate.patch'))
    put('source/initial-doc.md',read(root/'planning-subphases-v79/candidates/docs/resident-planning-subphases.md'))
    put('source/accepted-doc.md',git('docs/resident-planning-subphases.md'))
    for path in ('toolchain/resident/LICENSE-MIT','toolchain/resident/LICENSE-APACHE','toolchain/resident/JUSTRUST-SOURCE.md'):
        put('lineage/'+Path(path).name,git(path))
    put('runtime/source.original',read(root/'planning-subphases-profile-v79/source.original'))
    recipe=git('bench/blaze_runtime_compare.py').decode(); tree=ast.parse(recipe)
    fn=next(n for n in tree.body if isinstance(n,ast.FunctionDef) and n.name=='edit_bytes')
    put('source/edit_bytes.py.txt',ast.get_source_segment(recipe,fn).encode()+b'\n')
    # Recipe is documentation only, never executed by replay. Replace exactly known private roots.
    driver=read(root/'profile-planning-subphases-v79.py').decode()
    replacements={str(a.root):'<ROOT>',str(a.source):'<SOURCE>',str(a.toolchain):'<TOOLCHAIN>',str(a.bench):'<BENCH>',str(a.state):'<STATE>'}
    for old,new in replacements.items(): driver=driver.replace(old,new)
    assert ('/'+'home/') not in driver
    put('source/profile-recipe.py.txt',driver.encode())
    def process(row): return {k:row[k] for k in ('status','error','timed_out','elapsed_seconds')}
    rows=[]
    labels=['client planning','client planning rustc host','client planning target cfg','client planning asked setup','client planning owned resolver']
    for row in state['rows']:
        stem=f"{row['replicate']}-{row['kind']}"; base=root/'planning-subphases-profile-v79'
        trace=read(base/(stem+'.trace.json')); obj=json.loads(trace)
        assert sha(trace)==row['trace_sha256']
        actual=[e for e in obj['traceEvents'] if e.get('ph')=='X']
        assert actual==row['events']
        stdout=read(base/(stem+'.stdout')); stderr=read(base/(stem+'.stderr'))
        assert ('/'+'home/').encode() not in stdout
        names=('parses_libtest_results','shares_overlapping_time','splits_passes_without_double_counting','test_failures_are_not_compile_errors')
        expected='\nrunning 4 tests\n'+''.join('test summary::tests::'+name+' ... '+(row['marker']+'\nok' if name=='shares_overlapping_time' else 'ok')+'\n' for name in names)+'\ntest result: ok. 4 passed; 0 failed; 0 ignored; 0 measured; 144 filtered out; finished in 0.00s\n\n'
        assert stdout==expected.encode()
        put('runtime/'+stem+'.stdout',stdout)
        selected=[{k:e[k] for k in ('name','dur','ts','pid','tid','ph','cat')} for e in actual if e['name'] in labels]
        item=process(row); item.update({k:row[k] for k in ('kind','replicate','marker','source_sha256','trace_sha256')})
        item.update(events=selected,stdout='runtime/'+stem+'.stdout',noise_contamination=row['noise']['contamination'],public_pid_count=len({e['pid'] for e in actual}))
        rows.append(item)
    for stem in ('prepare','stop'):
        for ext in ('stdout','stderr'): read(root/'planning-subphases-profile-v79'/(stem+'.'+ext))
    source_before=state['source_before']; source_after=state['source_after']
    assert state['toolchain_before']==state['toolchain_after']
    tc=state['toolchain_before']['files']
    # Role-redact link values only if absolute, keeping their original raw identity.
    for name,item in tc.items():
        if item.get('link') and item['link'].startswith('/'):
            item['link']={'role':'<TOOLCHAIN_LINK>','sha256':sha(item['link'].encode()),'bytes':len(item['link'].encode())}
    put('inventories/pinned-source.json',encode({'before':source_before,'after':source_after}))
    put('inventories/toolchain.json',encode(tc))
    before=read(root/'candidate-build-v79/all-source-before.sha256').decode().splitlines()
    after=read(root/'candidate-unit-v79/all-source-after.log').decode().splitlines()
    inv={line.split('  ',1)[1]:line.split('  ',1)[0] for line in before}
    assert len(inv)==3757 and len(after)==3757 and set(after)=={k+': OK' for k in inv}
    put('inventories/native-source.json',encode(inv))
    put('inventories/joins.json',encode({'authority_commit':subprocess.check_output(['git','rev-parse',AUTHORITY]).decode().strip(),'native_after_ok':len(after),'native_before_count':len(inv),'native_before_sha256':sha(encode(inv)),'native_after_matches_before':True,'toolchain_before_sha256':sha(encode(tc)),'toolchain_after_sha256':sha(encode(tc)),'toolchain_before_count':len(tc),'toolchain_after_count':len(tc),'pinned_before_sha256':sha(encode(source_before)),'pinned_after_sha256':sha(encode(source_after))}))
    gates={}
    for group,names in {'candidate-build-v79':['check.exit','compile.exit','orchestration-exit','lock-before.sha256'],'candidate-unit-v79':['codegen-native.exit','full-native.exit','exit','lock-after.log']}.items():
        for name in names: gates[group+'/'+name]=read(root/group/name).decode().strip()
    native=[]
    exports=Path('data/runs/resident-planning-v79/2026-10.jsonl').read_bytes()
    records=[json.loads(line) for line in exports.splitlines()]
    put('gates/recorder.json',encode([{k:d[k] for k in ('id','args','exit','tests','subcommand')} for d in records]))
    for stem,count in (('codegen-native',53),('full-native',373)):
        log=read(root/'candidate-unit-v79'/(stem+'.log')).decode()
        selected=[line for line in log.splitlines() if line.startswith(('justrust: ok:','  tests   ','  more    '))]
        put('gates/'+stem+'.txt',('\n'.join(selected)+'\n').encode())
        run_id=re.search(r'justrust log ([0-9-]+)',log)[1]
        journal=read(root/'candidate-unit-v79/justrust-home/runs'/run_id/'output.jsonl').decode()
        named=[]
        for record in map(json.loads,journal.splitlines()):
            for value in record.values():
                if isinstance(value,str):
                    named.extend(line for line in value.splitlines() if re.fullmatch(r'test [A-Za-z0-9_:]+ \.\.\. ok(?: <[0-9.]+s>)?',line))
        assert len(named)==count and len(set(named))==count
        put('gates/'+stem+'-named.txt',('\n'.join(named)+'\n').encode())
        native.append({'name':stem,'passed':count,'run_id':run_id})
    for stem in ('check','compile'): read(root/'candidate-build-v79'/(stem+'.log'))
    harness=read(root/'v79-full-harness-regression.log').decode()
    m=re.search(r'Ran (\d+) tests in ([0-9.]+)s\n\nOK',harness); assert m and int(m[1])==761
    put('gates/harness.txt',(m[0]+'\n').encode())
    data={'authority':AUTHORITY,'scope':state['scope'],'contract':{'comparison':False,'speed_claim':False,'default_promotion':False,'noise':'unknown','inclusive':True,'quiet_machine_established':False},'status':state['status'],'unchanged':state['unchanged'],'tool_before':state['tool_before'],'tool_after':state['tool_after'],'prepare':process(state['prepare']),'stop':process(state['stop']),'rows':rows,'gates':gates,'native':native,'harness_count':761,'ranges_us':{label:[min(e['dur'] for row in rows for e in row['events'] if e['name']==label),max(e['dur'] for row in rows for e in row['events'] if e['name']==label)] for label in labels},'export_original':{'sha256':sha(exports),'bytes':len(exports)}}
    put('observations.json',encode(data)); put('private-inputs.json',encode(receipts))
    for name,receipt in receipts.items():
        raw=(root/name).read_bytes(); assert {'sha256':sha(raw),'bytes':len(raw)}==receipt
    print('Exported explicit projections, originals unchanged. observation SHA256',sha((out/'observations.json').read_bytes()))
if __name__=='__main__': main()
