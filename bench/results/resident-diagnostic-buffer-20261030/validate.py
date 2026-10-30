#!/usr/bin/env python3
"""Portable offline replay of rejected v20. Never runs archived commands/tools."""
import argparse,json,math,re,statistics
from pathlib import Path
from raw_checks import Evidence,InvalidEvidence,need,sha,canonical,libtest,session
import warm_checks

ENGINES={4:'deafad0c76f20059db6e1a07f4ed9665013ee61600db0089aec3667c38069df8',20:'2af3e38a8daed6f1a89827fd0176a0b05956199704f3d013dd66d0241ab97388'}
MAIN='2c1f91a8d31c7ad8c0c2c90451f5f042a6097f8a7110814d84b2917375be3405'
BASE='471f0b5c10187ecbea879acfe3e5eae52925161e32bce6feeebe49b67af0368f'
PATCH='b8aa898da735c3c42539aa9fa9a132096728e96f1b2c99c2ba30fe690b7ba8e4'
HELPERS=['tests::resident_diagnostic_buffer_large_to_small_reuses_storage','tests::resident_diagnostic_buffer_matches_owned_syntax_text']
RUNS={'20261030-113131259-2094449':('build','check',None), '20261030-113159600-2096531':('build','build',None), '20261030-113438035-2107407':('unit','test',('resident_diagnostic_buffer',2)), '20261030-113705404-2117352':('unit','test',('resident_graph_reuse',1)), '20261030-113705722-2117423':('unit','test',('the_roots_of_a_listed_crate',1)), '20261030-113705994-2117471':('unit','test',('telemetry',3)), '20261030-113913476-2300871':('unit','test',('',348))}
EXCLUDED={'manifest.json','audit.json','offline-tests.log','README.md','validate.py','test_validate.py','collect.py','seal.py','raw_checks.py','warm_checks.py'}


def inventory(e):
    declared=e.obj('manifest.json')['sha256']
    actual={str(p.relative_to(e.root)) for p in e.root.rglob('*') if p.is_file() and p.name not in EXCLUDED and '__pycache__' not in p.parts}
    need(set(declared)==actual,'manifest inventory mismatch')
    for path,digest in declared.items(): need(sha(e.read(path))==digest,'artifact hash: '+path)
    return len(declared)


def replay_patch(original,patch):
    lines=patch.splitlines(keepends=True)
    need(lines[0].startswith('--- ') and lines[1].startswith('+++ '),'patch headers')
    source=original.splitlines(keepends=True); output=[]; cursor=0; i=2
    while i<len(lines):
        m=re.fullmatch(r'@@ -(\d+)(?:,(\d+))? \+(\d+)(?:,(\d+))? @@.*\n',lines[i])
        need(m is not None,'patch hunk')
        start=int(m[1])-1
        need(start>=cursor,'patch overlap')
        output.extend(source[cursor:start]); cursor=start; old=new=0; i+=1
        while i<len(lines) and lines[i][:1] in [' ','+','-']:
            kind,text=lines[i][0],lines[i][1:]
            if kind!='+':
                need(cursor<len(source) and source[cursor]==text,'patch context')
                cursor+=1; old+=1
            if kind!='-':output.append(text); new+=1
            i+=1
        need(old==int(m[2] or 1) and new==int(m[4] or 1),'patch counts')
    output.extend(source[cursor:]); return ''.join(output)


def sources(e):
    root='source-ready/'; main=root+'toolchain/resident/crates/ra-jit/src/main.rs'
    need(sha(e.read(main))==MAIN and sha(e.read(root+'main.head.rs'))==BASE,'full source identity')
    patch=e.text(root+'HEAD-to-candidate.patch')
    need(sha(patch.encode())==PATCH,'exact candidate patch')
    need(e.text(root+'HEAD.txt').strip()=='6197235ec4daeeb2173f00044645a6089f2402d5','baseline HEAD')
    sections=patch.split('diff --git ')[1:]
    need(len(sections)==2,'patch ownership')
    first=sections[0]; scoped=first[first.index('--- '):]
    need(replay_patch(e.text(root+'main.head.rs'),scoped)==e.text(main),'candidate reconstruction')
    second=sections[1]; scoped=second[second.index('--- '):]
    # New-file hunks start at old zero. Preserve exact doc bytes directly from additions.
    doc=''.join(line[1:] for line in scoped.splitlines(keepends=True)[3:] if line.startswith('+'))
    need(doc==e.text(root+'docs/resident-diagnostic-buffer.md'),'new doc reconstruction')
    sums={path:digest for digest,path in (line.split(None,1) for line in e.text('native/candidate-build-v20/source-before.sha256').splitlines())}
    need(len(sums)==11 and sums['crates/ra-jit/src/main.rs']==MAIN,'frozen source inventory')
    for path,digest in sums.items(): need(sha(e.read('native/candidate-build-v20/frozen-source/'+path))==digest,'frozen source bytes')
    expected=[path+': OK' for path in sums]
    for path in ['native/candidate-build-v20/source-after.log','native/candidate-unit-v20/source-after.log','native/candidate-unit-v20/full-native-source-after.log']:
        need(e.text(path).splitlines()==expected,'native source drift')
    for line in e.text(root+'SHA256SUMS').splitlines():
        digest,path=line.split(None,1)
        suffix=path.split('/diagnostic-buffer-v20/',1)[1]
        need(sha(e.read(root+suffix))==digest,'ready source receipt')
    need('SOURCE READY' in e.text(root+'diagnostic-buffer-source-ready-v20.txt'),'ready receipt')
    restored=e.obj('provenance/root-restoration.json')
    need(restored['head']==e.text(root+'HEAD.txt').strip() and restored['head_sha256']==restored['working_sha256']==BASE and restored['git_diff_HEAD_exit']==0 and restored['production_source_changes'] is False,'root HEAD restoration')


def native(e):
    found={}; full=[]
    for run,(kind,command,test) in RUNS.items():
        prefix=f'native/candidate-{kind}-v20/justrust-home/runs/{run}/'
        s=e.obj(prefix+'summary.json'); m=e.obj(prefix+'meta.json'); journal=e.journal(prefix+'output.jsonl')
        need(s['id']==m['id']==run and s['args']==m['args'],'native id/args')
        args=[command,'--release','--locked','-p','ra-jit','--bin','jr-engine']
        if command=='check':args+=['--tests']
        args+=['-j','6']
        if test and test[0]:args+=['--',test[0],'--nocapture']
        need(s['args']==args and s['exit']==0 and s['git']['head']=='6197235ec4da','native exact command/exit/head')
        need(m['place']=='local' and m['route']=='JUSTRUST_REMOTE=off','native controlled route')
        tc='/home/jeremy/.blaze/toolchains/nightly-2026-10-27-x86_64-unknown-linux-gnu/bin/'
        need(m['real_cargo']==tc+'cargo' and m['real_rustc']==tc+'rustc','native compiler identity')
        need(journal and all(row['s'] in ['e','o'] and math.isfinite(row['t']) for row in journal),'native raw journal')
        text='\n'.join(row['l'] for row in journal)
        if test:
            names,counts=libtest(text,native=True); n=test[1]
            need(counts==(n,0,0,0,348-n),'native exact positive count')
            need(s['tests']['passed']==n and s['tests']['failed']==0 and s['tests']['filtered_out']==348-n,'native summary count')
            if test[0]:need(all(test[0] in name for name,_ in names),'native filter raw names')
            if test[0]=='resident_diagnostic_buffer':need([name for name,_ in names]==HELPERS,'native exact helper tests')
            if not test[0]:
                full=[name for name,_ in names]
                need(set(HELPERS).issubset(full) and len(full)==348,'actual full348 plus helpers')
                need(canonical(full)=='a743cccb5d734a1df5d57993324607d9378f438050b5c4b2ed2e3cd371a0812e','exact full348 raw names')
                need(canonical([n for n in full if n not in HELPERS])=='515768528951d84f22d97493fc0bbf53e2ba30ec0fa705bff37cb10a42d668be','baseline346 raw names')
            found[test[0] or 'full-native']={'run':run,'passed':n}
    listed=e.obj('native-runs.json')
    need(len(listed)==7 and {r['id'] for r in listed}==set(RUNS),'seven exact run allowlist')
    for row in listed:
        kind=RUNS[row['id']][0]
        path=f"native/candidate-{kind}-v20/justrust-home/runs/{row['id']}/summary.json"
        need(row['summary']==path and row['args']==e.obj(path)['args'],'run allowlist summary binding')
    for filt in ['resident_diagnostic_buffer','resident_graph_reuse','the_roots_of_a_listed_crate','telemetry']:
        need(e.text('native/candidate-unit-v20/'+filt+'.exit').strip()=='0','focused native exit')
    for kind,expected in [('build',2),('unit',5)]:
        rows=[json.loads(line) for line in e.text(f'exports/v20-{kind}/2026-10.jsonl').splitlines()]
        need(len(rows)==expected and {r['id'] for r in rows}=={k for k,v in RUNS.items() if v[0]==kind},'private exported run binding')
    for path in ['native/candidate-build-v20/check-exit','native/candidate-build-v20/compile-exit','native/candidate-build-v20/exit','native/candidate-unit-v20/exit','native/candidate-unit-v20/full-native.exit','provenance/suite-v20.exit','provenance/suite-v20-baseline.exit','provenance/suite-v20-candidate.exit','provenance/final-v20.exit']:
        need(e.text(path).strip()=='0','gate exit receipt')
    admission=e.obj('native/candidate-unit-v20/positive-test-admission.json')
    need(admission['verdict']=='passed' and {r['filter']:r['passed'] for r in admission['native_test_rows']}=={'resident_diagnostic_buffer':2,'resident_graph_reuse':1,'the_roots_of_a_listed_crate':1,'telemetry':3},'positive admission')
    return found


def validate(root,overrides=None,check_inventory=True):
    e=Evidence(root,overrides)
    count=inventory(e) if check_inventory else None
    sources(e); tests=native(e)
    cases={}; metas={}; hosts={}
    for case,v in [('rsb11',4),('rsa17',20)]:
        version=e.text(f'native/candidate-build-v{v}/engine-version.txt').strip()
        meta,samples,host=session(e,case,ENGINES[v],version,v)
        cases[case]=samples
        metas[case]=meta; hosts[case]=host
    need(hosts['rsb11']==hosts['rsa17'] and hosts['rsb11']['execution']=='local native direct','paired host/execution')
    for key in ['source_before','toolchain_before','profiles','proc_macro_policy','store_scope']:
        need(metas['rsb11'][key]==metas['rsa17'][key],'paired inputs '+key)
    need(metas['rsb11']['tools_before']['reference']==metas['rsa17']['tools_before']['reference'],'paired reference identity')
    old,new=cases['rsb11'],cases['rsa17']
    delta=(new['engine']['median_seconds']/old['engine']['median_seconds']-1)*100
    normalized=((new['engine']['median_seconds']/new['reference']['median_seconds'])/(old['engine']['median_seconds']/old['reference']['median_seconds'])-1)*100
    before,after=old['engine']['median_seconds'],new['engine']['median_seconds']
    ra,rb=before/old['reference']['median_seconds'],after/new['reference']['median_seconds']
    expected=('Native resident scoped diagnostic: descriptive/uncontrolled, noise unknown.\n'
              f'Engine median: {before:.6f}s -> {after:.6f}s, delta {after-before:+.6f}s ({delta:+.2f}%).\n'
              f'Reference-normalized engine/reference ratio: {ra:.6f} -> {rb:.6f}, relative delta {normalized:+.2f}%.\n'
              'Absolute median delta does not exceed requested 10% floor. No significance or speedup verdict.\n'
              'Not a comprehensive workload or default-promotion gate.\n')
    need(e.text('provenance/suite-v20-compare.log')==expected,'raw comparison replay')
    warm=warm_checks.replay(e,ENGINES)
    return {'verdict':'REJECT','source_restored_to_HEAD':True,'source_change_shipped':False,'artifacts':count,'native':tests,'fixed_five':{'baseline':old,'candidate':new,'engine_percent':delta,'reference_normalized_percent':normalized},'interleaved':warm,'noise':'unknown','allocation_count_measured':False,'general_speed_claim':False,'default_promotion':False}

if __name__=='__main__':
    parser=argparse.ArgumentParser(description=__doc__);parser.add_argument('root',nargs='?',type=Path,default=Path(__file__).resolve().parent);args=parser.parse_args()
    print(json.dumps(validate(args.root),indent=2))
