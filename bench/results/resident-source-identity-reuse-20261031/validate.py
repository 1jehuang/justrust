#!/usr/bin/env python3
"""Portable stdlib-only replay of historical v86 evidence, never a native run.
Trust boundary: keep this validator from the reviewed commit. A local resealed
SHA manifest alone is not authenticity. The historical observation commitment
below also binds all selected raw receipts and archived sources. Semantic replay
runs before that commitment so it cannot substitute for the original oracle.
"""
import argparse
import ast
import hashlib
import json
import math
import re
import statistics
from pathlib import Path
from types import SimpleNamespace

OBSERVATIONS = '29f043be4ea16c34ae8ccdc3b7609a045ba8d229f56858a6aec725e0b5893502'
BINARY = 'e125cbe009ca7524c80169985fd1588427914eba999f72b11b4bf5c259e74aa7'
REFERENCE = 'fa622aeca09858c050264f701fbbebd1a14aa07d2b02c1eb4de511b684ba0263'
ORIGINAL = '6c225dd077fb24b8fd83afffa739044d1752bb4581fb41390ac12cfd3fb0b59b'
CANDIDATE = 'b761a33a6c7be01917fbacefd825aba0cb77a8d538371f11e7a54566203525bd'
FLAG = 'RA_JIT_SOURCE_ID_REUSE'
MARKER = 'ra-jit: source-identity-reuse-v86 batch'
CASES = ('off','on','on-repeat','off-repeat')
PHASES = ('cold-prepare','repeated-noop-1','manifest-feature-add','type-error','type-recovery','assert-failure','assert-recovery')
GATES = ('version','prepare','type-error','type-recovery','assert-failure','assert-recovery')
NAMES = ('summary::tests::parses_libtest_results','summary::tests::shares_overlapping_time',
         'summary::tests::splits_passes_without_double_counting','summary::tests::test_failures_are_not_compile_errors')
ENV = {'BLAZE_BACKEND':'cranelift','BLAZE_OPT':'0','CARGO_PROFILE_DEV_DEBUG':'0',
       'CARGO_PROFILE_DEV_INCREMENTAL':'true','CARGO_PROFILE_DEV_OPT_LEVEL':'0',
       'CARGO_PROFILE_TEST_DEBUG':'0','CARGO_PROFILE_TEST_INCREMENTAL':'true','CARGO_PROFILE_TEST_OPT_LEVEL':'0'}
ORACLES = {
 'resident_state_matrix':('138aa2fb7e1791de92d43c8855c6151abd188e5f57fdb4d28c22e326c54be738',('baseline','inputs','contract','oracle')),
 'resident_workload_compare':('e47ef667315c0044e2b0072eb43a9db68860f32b50757bdd927b49522de1e92d',('valid',)),
 'blaze_runtime_compare':('a7e2af863e2b1ea2878ae5c66a0f01aab1ad7fb9efe2e1027254a1269eefb568',('test_results','edit_bytes')),
 'blaze_edit_compare':('9c451fdb939d3a8c024ddf7e2b8c2fd3a8badc7c164a429415a7c636eb8c41a8',('succeeded',)),
 'resident_engine_compare':('7da5b024344f662743b81043f4c200a23b3a49f61356e933b48257b2860c5a80',('valid_tests',)),
 'resident_suite':('548d9dc3fd6c9d7075f576b920c833e36715bcba22f9656bfbd2a5270e5cec66',('retouch_fixture',)),
}


def sha(raw):
    return hashlib.sha256(raw).hexdigest()


def canonical(value):
    return json.dumps(value,sort_keys=True,separators=(',',':')).encode()


def require(condition,message):
    if not condition:
        raise ValueError(message)


def safe_path(root,name):
    require(isinstance(name,str) and name and not Path(name).is_absolute() and '..' not in Path(name).parts,'relative artifact path')
    path=root/name
    require(path.resolve().is_relative_to(root.resolve()) and not path.is_symlink(),'artifact path escape/symlink')
    return path


def original_oracles(root):
    """Hash-bound pure function closure only, no runnable module imports."""
    scopes={}
    for name,(digest,wanted) in ORACLES.items():
        raw=(root/'oracles'/(name+'.py.txt')).read_bytes()
        require(sha(raw)==digest,'fixed original oracle '+name)
        tree=ast.parse(raw)
        functions=[n for n in tree.body if isinstance(n,ast.FunctionDef) and n.name in wanted]
        require({n.name for n in functions}==set(wanted),'exact pure original function closure')
        scope={'re':re,'require':require,'NAMES':list(NAMES),'EDIT_TEST':NAMES[1]}
        if name=='resident_state_matrix':
            for node in tree.body:
                if isinstance(node,ast.Assign) and len(node.targets)==1 and isinstance(node.targets[0],ast.Name) and node.targets[0].id in ('FILES','ANCHOR','PROBE','SIGNATURE','SIGNATURE_CHANGED'):
                    scope[node.targets[0].id]=ast.literal_eval(node.value)
            scope['SHIM_ANCHOR']=b'fn describes_cargo_rustc_args() {'
        exec(compile(ast.Module(body=functions,type_ignores=[]),name,'exec'),scope)
        scopes[name]=scope
    runtime=scopes['blaze_runtime_compare']; matrix=scopes['resident_state_matrix']
    matrix['shared']=SimpleNamespace(NAMES=list(NAMES),EDIT_TEST=NAMES[1])
    matrix['runtime']=SimpleNamespace(edit_bytes=runtime['edit_bytes'],base=SimpleNamespace(succeeded=scopes['blaze_edit_compare']['succeeded']))
    matrix['workload']=SimpleNamespace(valid=scopes['resident_workload_compare']['valid'])
    return scopes


def text(row):
    return row['stdout']+'\n'+'\n'.join(row['diagnostic_lines'])


def process(row):
    require(row['error'] is None and row['timed_out'] is False,'process error/timeout')
    require(type(row['elapsed_seconds']) in (int,float) and math.isfinite(row['elapsed_seconds']) and row['elapsed_seconds']>0,'positive finite timing')


def successful(row):
    process(row)
    require(row['status']==0,'successful command status')


def row_common(row,nonces=None):
    process(row)
    require(row['safe_environment']==ENV,'exact environment allowlist')
    require(row['noise_contamination']=='unknown','noise remains unknown')
    if 'accepted' in row: require(row['accepted'] is True,'accepted historical call')
    for key in ('raw_error','error'):
        if key in row: require(row[key] is None,'no hidden error')
    if 'input_unchanged' in row: require(row['input_unchanged'] is True,'per-call inputs unchanged')
    if 'failures' in row: require(row['failures']==[],'no hidden failures')
    for key in ('status_receipt','worker_status'):
        if key in row: successful(row[key])
    if nonces is not None:
        nonce=row['nonce']
        require(isinstance(nonce,str) and re.fullmatch('[0-9a-f]{32}',nonce) and nonce not in nonces,'fresh unique runtime nonce')
        nonces.add(nonce)
    require(sha(row['stdout'].encode())==row['raw_original']['stdout']['sha256'] and len(row['stdout'].encode())==row['raw_original']['stdout']['bytes'],'full raw stdout identity')


def state_common(case,toolchain):
    require(case['status']=='passed' and case['unchanged'] is True,'completed unchanged historical state')
    require(case['tool_before']==case['tool_after'],'unchanged native tool')
    require(case['toolchain_before']==case['toolchain_after']==toolchain,'unchanged pinned toolchain')


def strict(root,data,scopes,source,toolchain):
    matrix=scopes['resident_state_matrix']; parse=scopes['blaze_runtime_compare']['test_results']
    require(set(data['strict'])==set(CASES),'strict four counterordered states')
    nonces=set()
    for name in CASES:
        case=data['strict'][name]; enabled=name.startswith('on')
        state_common(case,toolchain)
        require(case['tool_before']==BINARY and case['source_before']==case['source_after']==source,'strict source63 and same candidate')
        require(case['controls']=={'RA_JIT_DEMAND_STD':'1','RA_JIT_CACHE_MISS_STATS':'1','RA_JIT_STATS':'1','RA_JIT_WHOLE_RELOAD':'0','RA_JIT_WATCHED_MANIFEST_BYTES':'1','RA_JIT_MANIFEST_CONTENT_GENERATION':'1',FLAG:str(int(enabled))},'strict exact startup controls')
        require([r['phase'] for r in case['rows']]==list(PHASES),'seven ordered strict states')
        originals={file:(root/'strict'/name/f'original-{i}').read_bytes() for i,file in enumerate(matrix['FILES'])}
        require(all(sha(raw)==source[file] for file,raw in originals.items()),'strict original source receipts')
        expected=None
        for row in case['rows']:
            row_common(row,nonces)
            phase=row['phase']; actual=parse(text(row))
            require(canonical(actual)==canonical(row['parsed']),'original tuple parser matches receipt')
            if expected is None:
                expected=matrix['contract'](actual,phase)
                require(expected is not None,'original cold contract')
            require(matrix['oracle'](dict(row,parsed=actual),text(row),expected,phase,row['nonce'],0)==[],'exact original strict oracle')
            inputs=dict(source)
            inputs.update({file:sha(raw) for file,raw in matrix['inputs'](originals,phase,0,row['nonce']).items()})
            require(row['inputs']==inputs,'independently derived strict input recipe')
            require(row['warm_marker_lines']==([MARKER] if enabled else []) and row['entry_marker_count']==1,'actual warm marker separate from generation entry')
    return len(nonces)


def factor_oracles(root):
    raw=(root/'oracles/v81-resident_suite.py.txt').read_bytes()
    require(sha(raw)==ORACLES['resident_suite'][0],'frozen v81 ancestor')
    driver=ast.parse((root/'drivers/suite-frozen-source-identity-v86.py.txt').read_text())
    replacements=[n for n in driver.body if isinstance(n,ast.Assign) and any(isinstance(t,ast.Name) and t.id=='REPLACEMENTS' for t in n.targets)]
    require(len(replacements)==1,'exact authorized factor replacement list')
    transformed=raw.decode()
    for old,new in ast.literal_eval(replacements[0].value): transformed=transformed.replace(old,new)
    require(sha(transformed.encode())=='cf539b9986f5b252de3c155c308af053cf770d0d76459cdcf7ffbade1493a11b','exact executed factor adapter')
    initial=(root/'drivers/suite-frozen-source-identity-v86.initial-mocked.py.txt').read_text()
    current=(root/'drivers/suite-frozen-source-identity-v86.py.txt').read_text()
    require(initial.replace('noop/clientplanning warm registry fetch factor','noop/clientplanning source identity reuse factor').replace('Warm registry fetch flag explicitly','Source identity reuse flag explicitly')==current,'only two descriptive replacement values changed')
    require(replay_patch(initial,(root/'drivers/description-only.patch').read_text())==current,'exact driver description patch')
    prior_replacements=next(n.value for n in ast.parse(initial).body if isinstance(n,ast.Assign) and any(isinstance(t,ast.Name) and t.id=='REPLACEMENTS' for t in n.targets))
    prior=raw.decode()
    for old,new in ast.literal_eval(prior_replacements):prior=prior.replace(old,new)
    require(sha(prior.encode())=='c67c9cdd107db4f45929a669424ee091e3f79a3473be393257df7f0503e1248b','exact initial mocked transform')
    for name,value in [('source-identity-factor-v86-mock-tests.log',prior),('source-identity-factor-v86-final-mock-tests.log',transformed)]:
        receipt=json.loads((root/'drivers'/(name+'.txt')).read_text().splitlines()[-1])
        require(receipt['executed_factor_sha256']==sha(value.encode()) and receipt['tests']==18 and receipt['passed'] is True and receipt['native_jobs_executed']==0,'both exact-driver18 mocked receipts')
    wanted={'check_parallel_activation','worker_key','check_retouch_process'}
    require([ast.dump(n) for n in ast.parse(prior).body if isinstance(n,ast.FunctionDef) and n.name in wanted]==[ast.dump(n) for n in ast.parse(transformed).body if isinstance(n,ast.FunctionDef) and n.name in wanted],'identical pure factor oracle ASTs')
    functions=[n for n in ast.parse(transformed).body if isinstance(n,ast.FunctionDef) and n.name in wanted]
    require({n.name for n in functions}==wanted,'original factor pure closure')
    import os
    scope={'require':require,'re':re,'Path':Path,'os':os,'math':math,'PARALLEL_MARKER':MARKER}
    exec(compile(ast.Module(body=functions,type_ignores=[]),'original-factor-oracles','exec'),scope)
    return scope


def worker_key(row,group,factor):
    w=row['worker'];pid=w['pid'];ticks=w['start_ticks'];argv=w['argv']
    require(type(pid) is int and pid>0 and type(ticks) is int and ticks>0,'native PID/start ticks')
    require(w['state'] not in ('Z','X','x') and w['exe']=='<ENGINE>' and w['exe_sha256']==BINARY,'live candidate worker executable')
    require(row['status_worker_lines']==['worker: pid '+str(pid)],'status and proc PID exact')
    require(argv[:2]==['<ENGINE>','<SOURCE>/'] and len(argv)==21,'complete normalized worker argv')
    require(argv[2:11]==['--own-target','--profile','dev','--cfg-test','--dev-deps','--asked-in','<SOURCE>','--crate','src/lib.rs'],'native configuration roles')
    values=dict(zip(argv[11::2],argv[12::2]))
    require(set(values)=={'--out','--patch','--exe','--socket','--symbols-in'},'exact worker path roles')
    for k in ('--out','--patch','--exe'):
        require(values[k].startswith('<PRIVATE_'+group+'_TARGET>/ra-jit/whole-test/') and '..' not in values[k].split('/'),'private target role')
    require(re.fullmatch(re.escape('<PRIVATE_'+group+'_HOME>/')+r'workers/[0-9a-f]+/sock',values['--socket']),'private socket role')
    require(values['--symbols-in']==values['--socket'][:-4]+'symbols','same worker symbols role')
    # Execute the original oracle on losslessly normalized path roles and the
    # selected kernel identity fields. No real /proc or native paths are read.
    home='/STATE/'+group
    actual_argv=[a.replace('<PRIVATE_'+group+'_HOME>',home).replace('<ENGINE>','/ENGINE') for a in argv]
    receipt={'pid':pid,'stat':str(pid)+' (jr-engine) '+' '.join([w['state']]+['0']*18+[str(ticks)]),
             'exe':'/ENGINE','exe_sha256':BINARY,'cmdline_hex':('\0'.join(actual_argv)+'\0').encode().hex()}
    result=factor['worker_key'](receipt,{'path':'/ENGINE','sha256':BINARY},home)
    return result,tuple(argv)


def pinned(root,data,scopes,source,toolchain,factor,run_key):
    p=data[run_key];rows=p['records'];runtime=scopes['blaze_runtime_compare']
    require(p['schema']=='native_resident_source_identity_retouch/v1' and p['pin']=='f97fcc2','distinct factor schema/pin')
    require(p['order']==['off','on','on','off'] and p['samples']==5,'exact ABBA5 design')
    require(p['source_before']==p['source_after']==source and p['status']=='complete' and p['source_restored'] is True and p['toolchain_links_valid'] is True,'source restoration')
    require(p['tools']=={'reference':REFERENCE,'engine':BINARY} and p['toolchain']==toolchain,'same ELF/reference/toolchain')
    require(p['profiles']=={k:v for k,v in ENV.items() if k.startswith('CARGO_PROFILE_')},'profiles')
    require(p['store_scope']=='unseeded private cold stores, no automatic seeding','no seeded stores')
    groups=['reference']+[f'segment-{i}' for i in range(4)]
    require([s['group'] for s in p['stops']]==groups,'five ordered native stops')
    for stop in p['stops']:
        successful(stop['process'])
        if 'stdout' in stop:require(stop['stdout']=='daemon: stopped\n','actual native stop stdout')
    require(set(p['controls'])==set(groups),'all explicit group controls')
    for group in groups:
        reference=group=='reference';enabled=group in ('segment-1','segment-2')
        require(p['controls'][group]=={'RA_JIT_MANIFEST_CONTENT_GENERATION':'0' if reference else '1','RA_JIT_WATCHED_MANIFEST_BYTES':'0' if reference else '1',FLAG:str(int(enabled)),'BLAZE_REMOTE':'off','RA_JIT_REMOTE':'off','BLAZE_PROC_MACRO_SERVER':'0','RA_JIT_PROC_MACRO_SERVER':'0'},'exact three factor flags/local controls')
    for key,roles in p['private_states'].items():
        require(set(roles)==set(groups) and len(set(roles.values()))==5 and all(roles[g]=='<PRIVATE_'+g+'_'+key+'>' for g in groups),'distinct private state roles')
    original=(root/'pinned/source.original').read_bytes();require(sha(original)==source['src/summary.rs'],'original pinned source')
    overlay=scopes['resident_suite']['retouch_fixture'](original)
    require(overlay==(root/'pinned/source.fixture').read_bytes(),'exact original runtime nonce overlay')
    require(p['fixture_source']==dict(source,**{'src/summary.rs':sha(overlay)}),'fixed overlaid source')
    wanted=[('reference',phase,0) for phase in GATES]
    for i in range(4):wanted.extend([(f'segment-{i}',phase,0) for phase in GATES]+[(f'segment-{i}','warmup',0)]+[(f'segment-{i}','retouch',n) for n in range(5)])
    require([(r['group'],r['phase'],r['replicate']) for r in rows]==wanted,'all54 ordered actual calls')
    nonces=set();workers={};samples={'off':[],'on':[]};first={};expected=None
    for row in rows:
        row_common(row);g=row['group'];phase=row['phase'];enabled=g in ('segment-1','segment-2');reference=g=='reference'
        require(row['flag']==str(int(not reference)),'generation flag stays fixed ON')
        flags={k:p['controls'][g][k] for k in ('RA_JIT_MANIFEST_CONTENT_GENERATION','RA_JIT_WATCHED_MANIFEST_BYTES',FLAG)}
        require(row['flags']==flags and row['schema']==p['schema'] and row['run_id']==p['run_id'] and row['rules']==p['rules'],'exact per-call factor/run/rules')
        factor['check_parallel_activation'](row['stdout'],'\n'.join(row['warm_marker_lines'])+('\n' if row['warm_marker_lines'] else ''),str(int(enabled)),phase)
        contents=original
        if phase in ('type-error','assert-failure'):contents=runtime['edit_bytes'](original,kind='type-error' if phase=='type-error' else 'test-failure')
        elif phase in ('warmup','retouch'):contents=overlay
        inputs=dict(source,**{'src/summary.rs':sha(contents)})
        require(row['source_before']==row['source_after']==inputs,'independently derived per-call source63')
        parsed=runtime['test_results'](text(row))
        if phase=='version':
            successful(row);require(row['stdout'].startswith('blaze 0.20261027.0 (' if reference else 'jr-engine experimental 0.20261027.0 ('),'actual native version')
        elif phase=='type-error':require(row['status']==101 and parsed is None and 'E0308' in text(row) and 'JR_RUNTIME_TYPE_ERROR' in text(row),'real type error')
        else:
            failing=phase=='assert-failure'
            require(row['status']==(101 if failing else 0) and scopes['resident_engine_compare']['valid_tests'](parsed,expected,failing),'original named libtest parity')
            require(parsed['filtered']==144,'original exact filtered count')
            if expected is None:expected=parsed
            if failing:require('JR_RUNTIME_FAILURE' in text(row),'real assertion failure')
        if phase in ('warmup','retouch'):
            nonce=row['nonce'];require(re.fullmatch('[0-9a-f]{32}',nonce) and nonce not in nonces,'fresh runtime nonce');nonces.add(nonce)
            require(row['nonce_environment']=={'JR_MANIFEST_RETOUCH_NONCE':nonce} and re.findall(r'JR_MANIFEST_RETOUCH_[^\s]+',text(row))==['JR_MANIFEST_RETOUCH_'+nonce],'exact reachable runtime challenge')
            require(row['entry_marker_count']==1,'fixed generation entry, not reuse proof')
            identity=worker_key(row,g,factor)
            if phase=='retouch':
                require(identity==workers[g],'same PID/start ticks/exe/socket BOTH candidate arms')
                require([t['file'] for t in row['touches']]==['Cargo.toml','Cargo.lock'],'combined retouch fileset')
                for touch in row['touches']:
                    before,after=touch['before'],touch['after'];raw=(root/'pinned'/(touch['file']+'.original')).read_bytes()
                    require(before['sha256']==after['sha256']==sha(raw)==source[touch['file']] and before['size']==after['size']==len(raw),'same bytes and sizes')
                    require(type(before['mtime_ns']) is int and type(after['mtime_ns']) is int and after['mtime_ns']>before['mtime_ns'],'strictly increasing observed mtime')
                samples['on' if enabled else 'off'].append(row['elapsed_seconds'])
                if row['replicate']==0:first[g]=row['elapsed_seconds']
            else:require(row['touches']==[],'warmup not timed touch')
            workers[g]=identity
        else:require(row['nonce'] is None and row['nonce_environment']=={} and row['touches']==[],'gates not measured samples')
    measurements={arm:{'samples_seconds':v,'median_seconds':statistics.median(v)} for arm,v in samples.items()}
    for arm in ('off','on'):
        require(p[arm]['schema']==p['schema'] and p[arm]['arm']==arm and p[arm]['noise']=='unknown','compatible same-candidate arms')
        require(len(samples[arm])==10 and p[arm]['measurements']==measurements,'all20 raw times recompute both arm summaries')
    old,new=measurements['off']['median_seconds'],measurements['on']['median_seconds'];delta=100*(new/old-1)
    require(f'OFF median: {old:.6f}s -> ON median: {new:.6f}s, delta {new-old:+.6f}s ({delta:+.2f}%).' in p['printed_compare'],'exact printed comparison arithmetic')
    require((abs(delta)>10 if run_key=='pinned' else abs(delta)<10),'separate first and independent repeat floors')
    require(('does not exceed requested 10% floor' if run_key=='repeat' else 'exceeds requested 10% floor') in p['printed_compare'],'unpooled printed floor')
    require(len(nonces)==24 and len(first)==4,'all first touches and24 nonces retained')
    return {'calls':54,'measured':20,'runtime_nonces':24,'off_median_seconds':old,'on_median_seconds':new,'delta_percent':delta,'first_touch_seconds':first}


def replay_patch(original,patch):
    lines=patch.splitlines(keepends=True);require(lines[0].startswith('--- ') and lines[1].startswith('+++ '),'patch headers')
    old=original.splitlines(keepends=True);result=[];cursor=0;at=2
    while at<len(lines):
        m=re.match(r'@@ -(\d+)(?:,\d+)? \+(\d+)(?:,\d+)? @@',lines[at]);require(m is not None,'patch hunk')
        position=max(int(m[1])-1,0);require(cursor<=position<=len(old),'patch order')
        result.extend(old[cursor:position]);cursor=position;at+=1
        while at<len(lines) and not lines[at].startswith('@@ '):
            kind,line=lines[at][0],lines[at][1:];require(kind in ' +-','patch prefix')
            if kind in ' -':require(cursor<len(old) and old[cursor]==line,'exact patch context');cursor+=1
            if kind in ' +':result.append(line)
            at+=1
    result.extend(old[cursor:]);return ''.join(result)


def frozen_native(root,data):
    name='toolchain/resident/crates/blaze-metadata/src/lib.rs';f=root/'frozen/source-identity-v86'
    original=(f/'original-lib.rs').read_bytes();candidate=(f/'candidate-lib.rs').read_bytes()
    require(sha(original)==ORIGINAL and sha(candidate)==CANDIDATE,'exact full original/candidate native source')
    require((f/'accepted-HEAD-lib.rs').read_bytes()==original,'accepted original identity')
    chunks=(f/'candidate.patch').read_text().split('diff --git ')[1:]
    patches=[chunk[chunk.index('--- '):] for chunk in chunks]
    require(len(patches)==2 and replay_patch(original.decode(),patches[0])==candidate.decode(),'complete original to candidate Rust patch')
    require(replay_patch('',patches[1])==(f/'candidate-resident-source-identity-reuse.md').read_text(),'complete normalized historical doc patch')
    require((root/'frozen/source-identity-v86-rollback/live'/name).read_bytes()==candidate,'exact preserved rollback source')
    require(data['engine_identity']==BINARY and data['unit_exit']==0 and set(data['build_exits'].values())=={0},'actual native build/check/test statuses')
    inv=json.loads((root/'inventories/native-source3757.json').read_text())
    require(data['native_source_count']==3757 and len(inv)==3757,'full native source3757 inventory')
    require(inv['crates/blaze-metadata/src/lib.rs']==CANDIDATE,'tested candidate source binding')
    for kind,count in [('registry',36),('manifest',22),('metadata',4),('codegen',53),('full',373)]:
        receipt=json.loads((root/'native'/(kind+'.json')).read_text())
        require(receipt==data['native'][kind] and receipt['status']==0,'native named outcomes receipt '+kind)
        require(sum(map(int,re.findall(r'test result: ok\. (\d+) passed;', '\n'.join(receipt['lines']))))==count==receipt['passed'],'complete native counts '+kind)
        require(not any('FAILED' in line for line in receipt['lines']),'no hidden native failures')
        require(sum(bool(re.fullmatch(r'test [a-zA-Z0-9_:]+ \.\.\. ok(?: <[0-9.]+s>)?',line)) for line in receipt['lines'])==count,'exact individual named native count '+kind)
    tests=re.findall(r'#\[test\]\s*fn ([a-z_]+)\(',candidate.decode().split('mod source_identity_v86 {')[1])
    require(len(tests)==4,'exact four source-identity fixtures')
    for test in tests:
        require(any(test+' ... ok' in line for line in data['native']['metadata']['lines']),'actual new filesystem test outcome '+test)
    require(len(data['historical_harness_regression']['lines'])==2 and re.fullmatch(r'Ran 761 tests in [0-9.]+s',data['historical_harness_regression']['lines'][0]) and data['historical_harness_regression']['lines'][1]=='OK','restored full761 regression')
    mock=(root/'drivers/source-identity-factor-v86-mock-tests.log.txt').read_text()
    require('Ran 18 tests in ' in mock and json.loads(mock.splitlines()[-1])['passed'] is True,'18 mocked adapters separate from native')


def incomplete(data,source):
    p=data['incomplete'];rows=p['records']
    require(p['status']=='incomplete' and len(rows)==8 and p['source_restored'] is True and p['toolchain_links_valid'] is True,'incomplete8 preserved and restored')
    require([(r['group'],r['phase']) for r in rows]==[('reference',phase) for phase in GATES]+[('segment-0','version'),('segment-0','prepare')],'incomplete exact ordered prefix')
    require(p['controls']['segment-0'][FLAG]=='0','initial failure factor OFF')
    require(all(c['BLAZE_PROC_MACRO_SERVER']==c['RA_JIT_PROC_MACRO_SERVER']=='<OPTIONAL_TOOLCHAIN_PROC_SERVER>' for c in p['controls'].values()),'initial optional explicit server retained')
    require(p['tools']['engine']==BINARY and 'proc-macro-server' in p['tools'],'initial candidate and optional server identity')
    last=rows[-1];process(last)
    require(last['status']==101 and last['stdout']=='' and len(last['diagnostic_lines'])==6,'original OFF build failure retained')
    require(any('num-traits' in line for line in last['diagnostic_lines']) and any('autocfg' in line for line in last['diagnostic_lines']) and any('could not execute rustc' in line for line in last['diagnostic_lines']),'initial unproven buildscript failure detail')
    require([s['group'] for s in p['stops']]==['reference','segment-0'],'initial two stops retained')
    for stop in p['stops']:
        successful(stop['process'])
        if 'stdout' in stop:require(stop['stdout']=='daemon: stopped\n','actual native stop stdout')
    require(p['task_lines']==['[stderr] ValueError: native resident: original semantic libtest parity failed','--- Command finished with exit code: 1 ---'],'initial failed task output boundary')
    require(p['error']=="ValueError('native resident: original semantic libtest parity failed')",'initial driver error retained without causal assertion')


def validate_package(root,manifest=False):
    root=Path(root).resolve();raw=(root/'observations.json').read_bytes();data=json.loads(raw)
    source=json.loads((root/'inventories/pinned-source63.json').read_text());toolchain=json.loads((root/'inventories/toolchain4020.json').read_text())
    require(len(source)==63 and len(toolchain)==4020,'complete pinned and toolchain identities')
    for path,digest in source.items():require(sha(safe_path(root,'pinned-source/'+path).read_bytes())==digest,'full public pinned blob '+path)
    scopes=original_oracles(root);factor=factor_oracles(root)
    timing=pinned(root,data,scopes,source,sha(canonical(toolchain)),factor,'pinned')
    repeat=pinned(root,data,scopes,source,sha(canonical(toolchain)),factor,'repeat')
    require(data['pinned']['run_id']!=data['repeat']['run_id'],'independent repeat run identity')
    incomplete(data,source)
    strict_nonces=strict(root,data,scopes,source,sha(canonical(toolchain)))
    frozen_native(root,data)
    require(data['profile']['status']=='passed' and data['profile']['unchanged'] is True and len(data['profile']['rows'])==8,'actual profile8')
    for row in data['profile']['rows']:
        row_common(row);successful(row)
        require(row['warm_marker_lines']==[MARKER] and row['stdout'].count(row['marker'])==1,'actual profile marker plus runtime execution')
        require(scopes['resident_engine_compare']['valid_tests'](scopes['blaze_runtime_compare']['test_results'](text(row)),None,False),'profile original native tests')
        require(all(set(e)=={'cat','dur','name','ph','pid','tid','ts'} and isinstance(e['dur'],int) and e['dur']>=0 for e in row['events']),'fixed numeric trace receipts')
    require(data['cleanup']['stores_deleted'] is False and len(data['cleanup']['rows'])==4,'four stops stores retained')
    for row in data['cleanup']['rows']:successful(row);require(row['post_measurement_only'] is True and row['stdout']=='daemon: stopped\n','post-measurement cleanup')
    require(set(data['launch_failures'])=={'127214ykpx'} and all(r['native_jobs_started']==0 for r in data['launch_failures'].values()),'initial historical pre-native path failure')
    require(data['decision']=='rejected-below-10-percent-floor' and data['source_restored'] is True and data['noise']=='unknown' and data['default_promotion'] is False and data['general_speed_claim'] is False,'rejected promotion and exact nonclaims')
    for name,digest in data['artifacts'].items():require(sha(safe_path(root,name).read_bytes())==digest,'historical artifact '+name)
    require(sha(raw)==OBSERVATIONS,'trusted historical observation commitment')
    if manifest:
        inventory=json.loads((root/'SHA256.json').read_text())
        for name,digest in inventory.items():require(sha(safe_path(root,name).read_bytes())==digest,'export inventory '+name)
    return {'decision':data['decision'],'correctness':{'strict':strict_nonces,'profile':8},'native':[36,22,4,53,373],'timing':timing,'repeat':repeat,'incomplete_calls':8,'noise':'unknown','source_restored':True}


if __name__=='__main__':
    p=argparse.ArgumentParser();p.add_argument('--manifest',action='store_true');a=p.parse_args()
    print(json.dumps(validate_package(Path(__file__).resolve().parent,a.manifest),indent=2,sort_keys=True))
