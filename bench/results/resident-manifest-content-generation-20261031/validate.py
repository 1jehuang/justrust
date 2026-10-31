#!/usr/bin/env python3
"""Portable stdlib-only replay of historical v74 evidence, never a native run.
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

OBSERVATIONS = '2f4fa2234922fded685d3c94b6b5e0d56417dcdce722dc75838e6af641489474'
BINARY = '9cb7bff510a19eedca12e3c38f7085d5c494aac4e71800ecdb05b02f2e01ca1b'
REFERENCE = 'fa622aeca09858c050264f701fbbebd1a14aa07d2b02c1eb4de511b684ba0263'
ORIGINAL = '34b391dd07b92e735b7f640641c8298897c4ec9d3c239493682985c43a473b81'
CANDIDATE = '1c0e4612fdc8f926f44c5237b8551a2f0207d4a3dc06840ee774c4e2613cbd75'
FLAG = 'RA_JIT_MANIFEST_CONTENT_GENERATION'
MARKER = 'ra-jit: manifest-content-generation-v1 entered'
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
 'resident_suite':('1508a424fde23d7093ae004bc476abcf95d7681e36ed61d9d5c923de9f9cb2df',('retouch_fixture',)),
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
        require(case['controls']=={'RA_JIT_DEMAND_STD':'1','RA_JIT_CACHE_MISS_STATS':'1','RA_JIT_STATS':'1','RA_JIT_WHOLE_RELOAD':'0',FLAG:str(int(enabled))},'strict exact startup controls')
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
            require(row['marker_lines']==[] and row['entry_marker_count']==int(enabled),'once-process entry, not reuse proof')
    return len(nonces)


def worker_key(row,group):
    w=row['worker']; pid=w['pid']
    require(type(pid) is int and pid>0 and w['exe']=='<ENGINE>' and w['exe_sha256']==BINARY,'candidate worker executable')
    require(w['stat'].startswith(str(pid)+' ('),'proc stat PID')
    tail=w['stat'].rsplit(') ',1)[1].split()
    ticks=int(tail[19]); require(ticks>0,'positive proc start ticks')
    require(row['status_worker_lines']==['worker: pid '+str(pid)],'status and proc PID exact')
    argv=w['argv']
    require(argv[:2]==['<ENGINE>','<SOURCE>/'] and len(argv)==21,'normalized complete worker argv')
    require(argv[2:11]==['--own-target','--profile','dev','--cfg-test','--dev-deps','--asked-in','<SOURCE>','--crate','src/lib.rs'],'worker native configuration')
    values=dict(zip(argv[11::2],argv[12::2]))
    require(set(values)=={'--out','--patch','--exe','--socket','--symbols-in'},'exact worker path roles')
    for k in ('--out','--patch','--exe'):
        require(values[k].startswith('<PRIVATE_'+group+'_TARGET>/ra-jit/whole-test/') and '..' not in values[k].split('/'),'private target role')
    require(re.fullmatch(re.escape('<PRIVATE_'+group+'_HOME>/')+r'workers/[0-9a-f]+/sock',values['--socket']),'private worker socket role')
    require(values['--symbols-in']==values['--socket'][:-4]+'symbols','same worker symbols role')
    return pid,ticks


def pinned(root,data,scopes,source,toolchain):
    p=data['pinned']; rows=p['records']; runtime=scopes['blaze_runtime_compare']
    require(p['schema']=='native_resident_manifest_retouch/v1' and p['pin']=='f97fcc2','distinct scoped schema/pin')
    require(p['order']==['off','on','on','off'] and p['samples']==5,'samecandidate ABBA5 design')
    require(p['source_before']==p['source_after']==source and p['status']=='complete' and p['source_restored'] is True and p['toolchain_links_valid'] is True,'pristine pinned source restoration')
    require(p['tools']=={'reference':REFERENCE,'engine':BINARY} and p['toolchain']==toolchain,'actual candidate/reference/toolchain identities')
    require(p['profiles']=={k:v for k,v in ENV.items() if k.startswith('CARGO_PROFILE_')},'effective pinned profiles')
    require(p['store_scope']=='unseeded private cold stores, no automatic seeding','private cold state design')
    groups=['reference']+[f'segment-{i}' for i in range(4)]
    require(list(s['group'] for s in p['stops'])==groups,'all five native stops')
    for s in p['stops']: successful(s['process'])
    require(set(p['controls'])==set(groups),'all explicit per-group flags')
    for g,arm in zip(groups,['off','off','on','on','off']):
        require(p['controls'][g]=={FLAG:str(int(arm=='on')),'BLAZE_REMOTE':'off','RA_JIT_REMOTE':'off','BLAZE_PROC_MACRO_SERVER':'0','RA_JIT_PROC_MACRO_SERVER':'0'},'exact OFF/ON startup flag, local and compatibility proc-server controls')
    for key,roles in p['private_states'].items():
        require(set(roles)==set(groups) and len(set(roles.values()))==5 and all(roles[g]=='<PRIVATE_'+g+'_'+key+'>' for g in groups),'nonaliasing private state roles')
    original=(root/'pinned/source.original').read_bytes()
    require(sha(original)==source['src/summary.rs'],'original pinned source')
    overlay=scopes['resident_suite']['retouch_fixture'](original)
    require(overlay==(root/'pinned/source.fixture').read_bytes(),'runtime-reachable immutable nonce overlay')
    overlaid=dict(source,**{'src/summary.rs':sha(overlay)})
    require(p['fixture_source']==overlaid,'only summary runtime overlay')
    for file in ('Cargo.toml','Cargo.lock'):
        require(sha((root/'pinned'/(file+'.original')).read_bytes())==source[file],'original manifest lock backups')
    wanted=[('reference',phase,0) for phase in GATES]
    for i in range(4): wanted.extend([(f'segment-{i}',phase,0) for phase in GATES]+[(f'segment-{i}','warmup',0)]+[(f'segment-{i}','retouch',n) for n in range(5)])
    require([(r['group'],r['phase'],r['replicate']) for r in rows]==wanted,'all54 exact ordered real calls')
    nonces=set(); identities={}; samples={'off':[],'on':[]}; first=[]; expected=None
    for row in rows:
        row_common(row)
        group=row['group']; phase=row['phase']; enabled=group in ('segment-1','segment-2')
        require(row['flag']==str(int(enabled)),'per-call exact flag receipt')
        contents=original
        if phase in ('type-error','assert-failure'):
            contents=runtime['edit_bytes'](original,kind='type-error' if phase=='type-error' else 'test-failure')
        elif phase in ('warmup','retouch'): contents=overlay
        inputs=dict(source,**{'src/summary.rs':sha(contents)})
        require(row['source_before']==row['source_after']==inputs,'independently derived per-call pinned source63')
        parsed=runtime['test_results'](text(row))
        if phase=='version':
            successful(row)
            require(row['stdout'].startswith('blaze 0.20261027.0 (' if group=='reference' else 'jr-engine experimental 0.20261027.0 ('),'actual version output')
        elif phase=='type-error':
            require(row['status']==101 and parsed is None and 'E0308' in text(row) and 'JR_RUNTIME_TYPE_ERROR' in text(row),'real original unused type failure')
        else:
            failing=phase=='assert-failure'
            require(row['status']==(101 if failing else 0) and scopes['resident_engine_compare']['valid_tests'](parsed,expected,failing),'original named libtest parity')
            require(parsed['filtered']==144,'original exact filtered count')
            if expected is None: expected=parsed
            if failing: require('JR_RUNTIME_FAILURE' in text(row),'real original assertion failure')
        if phase in ('warmup','retouch'):
            nonce=row['nonce']; require(re.fullmatch('[0-9a-f]{32}',nonce) and nonce not in nonces,'fresh measured runtime nonce'); nonces.add(nonce)
            require(row['nonce_environment']=={'JR_MANIFEST_RETOUCH_NONCE':nonce},'runtime nonce environment challenge')
            require(re.findall(r'JR_MANIFEST_RETOUCH_[^\s]+',text(row))==['JR_MANIFEST_RETOUCH_'+nonce],'exact reachable runtime witness')
            require(row['entry_marker_count']==int(enabled),'actual opt-in entry receipt only')
            identity=worker_key(row,group)
            if phase=='retouch':
                require((identity==identities[group])==enabled,'samePID plus starttick ON, new generation OFF')
                require([t['file'] for t in row['touches']]==['Cargo.toml','Cargo.lock'],'same original combined watched fileset')
                for t in row['touches']:
                    before,after=t['before'],t['after']; raw=(root/'pinned'/(t['file']+'.original')).read_bytes()
                    require(before['sha256']==after['sha256']==sha(raw) and before['size']==after['size']==len(raw),'same-byte hash/size proof')
                    require(type(before['mtime_ns']) is int and type(after['mtime_ns']) is int and after['mtime_ns']>before['mtime_ns'],'observed strictly changed mtime')
                samples['on' if enabled else 'off'].append(row['elapsed_seconds'])
                if enabled and row['replicate']==0: first.append(row['elapsed_seconds'])
            else: require(row['touches']==[],'warmup no measured touch')
            identities[group]=identity
        else:
            require(row['nonce'] is None and row['nonce_environment']=={} and row['touches']==[],'semantic gates not measured nonce edits')
            if not enabled or phase=='version': require(row['entry_marker_count']==0,'OFF/version no marker')
    measurements={a:{'samples_seconds':v,'median_seconds':statistics.median(v)} for a,v in samples.items()}
    for arm in ('off','on'):
        result=p[arm]
        require(result['schema']==p['schema'] and result['arm']==arm and result['noise']=='unknown' and result['scope']==p['scope'],'compatible scoped arms, unknown noise')
        require(result['measurements']==measurements and len(samples[arm])==10,'recomputed all20 samples/medians, first touches retained')
    require(len(nonces)==24 and len(first)==2 and all(x>max(samples['off']) for x in first),'both slower first ON costs retained')
    old,new=measurements['off']['median_seconds'],measurements['on']['median_seconds']
    require(f'OFF median: {old:.6f}s -> ON median: {new:.6f}s, delta {new-old:+.6f}s ({100*(new/old-1):+.2f}%).' in p['printed_compare'],'original printed compare arithmetic')
    require('noise unknown' in p['printed_compare'] and 'not separate manifest/lock attribution or default promotion' in p['printed_compare'],'original scoped compare nonclaims')
    return {'off_median_seconds':old,'on_median_seconds':new,'delta_percent':100*(new/old-1),'first_on_seconds':first,'calls':len(rows),'measured':20,'runtime_nonces':len(nonces)}


def targeted(root,data,toolchain):
    require(set(data['targeted'])=={'original-off','candidate-off','candidate-on','candidate-on-repeat','candidate-off-repeat'},'targeted original and counterordered samecandidate cases')
    nonces=set()
    for name,case in data['targeted'].items():
        state_common(case,toolchain); enabled='-on' in name
        if name!='original-off': require(case['tool_before']==BINARY,'targeted same candidate')
        require(case['controls']=={'RA_JIT_DEMAND_STD':'1','RA_JIT_STATS':'1',FLAG:str(int(enabled))},'targeted exact controls')
        require([r['phase'] for r in case['rows']]==['cold','noop','manifest-same-bytes','lock-same-bytes','both-same-bytes','noop-final'],'all30 original targeted phases')
        source={f:sha((root/'targeted'/name/f).read_bytes()) for f in ('Cargo.toml','Cargo.lock','src/main.rs')}
        require(case['source_after']==source and case['source_before']=={k:v for k,v in source.items() if k!='Cargo.lock'},'targeted original source and initial missing lock')
        require((root/'targeted'/name/'src/main.rs').read_bytes()==b'fn main(){println!("CONTENT_GENERATION_{}",std::env::var("CONTENT_NONCE").unwrap());}\n','targeted actual runtime source')
        identities=[]
        for i,row in enumerate(case['rows']):
            row_common(row,nonces); successful(row)
            require(row['stdout']=='CONTENT_GENERATION_'+row['nonce']+'\n','fresh targeted runtime result')
            require(row['inputs']==(source if i else case['source_before']),'targeted exact original pre-call inputs, lock first created by cold call')
            pids=[int(v.rsplit(' ',1)[1]) for v in row['status_worker_lines']]
            require(row['worker_pids']==pids and len(pids)==1,'targeted native PID receipt')
            identities.append(pids[0])
            files=[[],[],['Cargo.toml'],['Cargo.lock'],['Cargo.toml','Cargo.lock'],[]][i]
            require([t['file'] for t in row['touches']]==files,'targeted exact retouch set')
            for t in row['touches']:
                require(t['bytes_unchanged'] is True and t['mtime_after']>t['mtime_before'],'targeted actual samebytes mtime change')
            require(row['marker_lines']==[] and row['entry_marker_count']==int(enabled),'targeted exact entry marker')
        require(identities[0]==identities[1] and identities[4]==identities[5],'no-op keeps worker')
        require((len(set(identities))==1) if enabled else all(identities[i]!=identities[i-1] for i in (2,3,4)),'targeted ON retention/OFF restart, not changed-content reuse')
        successful(case['stop'])
    return len(nonces)


def changed_manifest(root,data,toolchain):
    c=data['changed_manifest']; require(c['status']=='passed' and c['unchanged'] is True,'changed manifest completed')
    require(c['toolchain']==toolchain and c['tools']['off']==c['tools']['on']==BINARY,'changed manifest same toolchain/candidate')
    require([(r['mode'],r['phase']) for r in c['rows']]==[(m,p) for m in ('original','off','on') for p in ('valid','manifest-error','recovery')],'all9 ordered changed manifest states')
    nonces=set()
    for row in c['rows']:
        row_common(row,nonces)
        mode=row['mode']; raw=(root/'changed-manifest'/mode/'src/main.rs').read_bytes()
        require(sha(raw)==row['rust_before']==row['rust_after'],'unused function source unchanged')
        recipe=('fn main(){println!("GEN_OK_{}",std::env::var("GEN_NONCE").unwrap());}\n#[cfg(feature="bad")] fn unused_bad()->u32{"'+row['literal']+'"}\n').encode()
        require(raw==recipe,'independent actual unused function source')
        require(row['startup_control']==('1' if mode=='on' else '0'),'changed manifest startup control')
        if row['phase']=='manifest-error':
            require(row['status']==101 and not row['stdout'] and 'E0308' in text(row) and row['literal'] in text(row),'changed manifest activates unused E0308 with no runtime')
        else:
            successful(row); require(row['stdout']=='GEN_OK_'+row['nonce']+'\n','changed manifest recovery fresh runtime')
    require(len(c['stops'])==3,'three cfg stops')
    for stop in c['stops']: successful(stop)
    return len(nonces)


def native_aba(root,data,toolchain):
    require(set(data['native_aba'])=={'env','file'},'both native build-script guards')
    nonces=set()
    for name,case in data['native_aba'].items():
        state_common(case,toolchain); require(case['tool_before']==BINARY,'native ABA same candidate')
        folder=root/'native-aba'/name
        source={f:sha((folder/f).read_bytes()) for f in case['source_before']}
        require(source==case['source_before'],'actual native build-script sources')
        require(case['source_after']==dict(source,**{'Cargo.lock':sha((folder/'Cargo.lock').read_bytes())}),'native ABA exact final source restoration plus lock')
        main=b'extern "C"{fn native_value()->u32;}\nfn main(){println!("NATIVE_{}_{}",std::env::var("NATIVE_NONCE").unwrap(),unsafe{native_value()});}\n'
        require((folder/'src/main.rs').read_bytes()==main,'actual linked native function runtime source')
        require(len(case['rows'])==3,'three ABA observations')
        for index,row in enumerate(case['rows']):
            row_common(row,nonces); successful(row)
            choice=('a','b','a')[index] if name=='env' else 'a'
            require(row['choice']==choice,'actual build-script env ABA choice')
            c=(folder/(choice+'.c')).read_bytes()
            if name=='file' and index==1: c=c.replace(b'return 11;',b'return 33;')
            match=re.fullmatch(rb'unsigned int native_value\(void\)\{return ([0-9]+);\}\n',c)
            require(match is not None,'actual native C source recipe')
            value=int(match[1])
            require(row['expected']==value and row['stdout']=='NATIVE_'+row['nonce']+'_'+str(value)+'\n','independently derived native runtime result')
            if name=='file':
                inventory=dict(source,**{'a.c':sha(c)})
                if index: inventory['Cargo.lock']=sha((folder/'Cargo.lock').read_bytes())
                require(row['source_at_call']==inventory,'actual file ABA source inventory')
        successful(case['stop'])
    return len(nonces)


def replay_patch(original,patch):
    """Exact-context unified patch application in memory, no external tools."""
    if not patch: return original
    lines=patch.splitlines(keepends=True)
    require(lines[0].startswith('--- ') and lines[1].startswith('+++ '),'patch headers')
    old=original.splitlines(keepends=True); result=[]; cursor=0; at=2
    while at<len(lines):
        m=re.match(r'@@ -(\d+)(?:,\d+)? \+(\d+)(?:,\d+)? @@',lines[at]); require(m is not None,'patch hunk')
        position=max(int(m[1])-1,0); require(cursor<=position<=len(old),'patch order')
        result.extend(old[cursor:position]); cursor=position; at+=1
        while at<len(lines) and not lines[at].startswith('@@ '):
            kind,line=lines[at][0],lines[at][1:]; require(kind in ' +-','patch prefix')
            if kind in ' -': require(cursor<len(old) and old[cursor]==line,'patch exact context'); cursor+=1
            if kind in ' +': result.append(line)
            at+=1
    result.extend(old[cursor:]); return ''.join(result)


def frozen_and_native(root,data):
    f=root/'frozen/manifest-content-generation-v74'; original=(f/'whole.original.rs').read_bytes(); candidate=(f/'whole.candidate.rs').read_bytes()
    require(sha(original)==ORIGINAL and sha(candidate)==CANDIDATE,'immutable original/candidate whole source')
    native_patches=re.split(r'(?=^--- )',(f/'candidate.patch').read_text(),flags=re.M)[1:]
    require(len(native_patches)==2 and replay_patch(original.decode(),native_patches[0])==candidate.decode(),'full native patch replay')
    require(replay_patch('',native_patches[1])==(f/'resident-manifest-content-generation.md').read_text(),'initial source-only doc patch replay')
    doc=root/'frozen/manifest-content-generation-v74-final-doc'
    require(replay_patch((doc/'doc.original.md').read_text(),(doc/'doc.patch').read_text())==(doc/'doc.candidate.md').read_text(),'full final acceptance doc patch replay')
    h=root/'frozen/manifest-retouch-harness-v74'
    require((h/'resident_suite.candidate.py').read_bytes()==(root/'oracles/resident_suite.py.txt').read_bytes(),'immutable harness candidate')
    require((h/'test_resident_manifest_retouch.candidate.py').read_bytes()==(root/'oracles/test_resident_manifest_retouch.py.txt').read_bytes(),'immutable13 harness tests')
    harness_patches=re.split(r'(?=^--- )',(h/'candidate.patch').read_text(),flags=re.M)[1:]
    require(len(harness_patches)==2,'both original harness patches')
    for patch,name in zip(harness_patches,('resident_suite','test_resident_manifest_retouch')):
        require(replay_patch((h/(name+'.original.py')).read_text(),patch)==(h/(name+'.candidate.py')).read_text(),'full harness patch replay '+name)
    for gate in ('check','compile','codegen-native','full-native'): require(data['native'][gate]['exit']==0,'native exit gate')
    for name,count in (('codegen',53),('resident',369)):
        lines=json.loads((root/'native'/(name+'-results.json')).read_text())
        matches=[re.fullmatch(r'test ([A-Za-z0-9_:]+) \.\.\. ok <[0-9.]+s>',line) for line in lines[:-1]]
        require(len(matches)==count and all(matches) and len({m[1] for m in matches})==count,'complete actual native named results')
        require(re.fullmatch(r'test result: ok\. '+str(count)+r' passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in [0-9.]+s',lines[-1]),'complete actual native raw summary')
        if name=='resident': require(sum('cargo::whole::manifest_generation_tests::' in m[1] for m in matches)==5,'five executed native manifest helper fixtures')
    n=data['native']
    require(n['source_count']==3757 and n['source_check_ok'] is True and n['lock_check_ok'] is True and n['engine_sha256']==BINARY,'native source/lock/executable gates')
    require(n['lock_sha256']=='bf3307542b2b5c0e7a4b8c5b1a1e650bba9afece80284cc497a3800560a0c2bc','native original lock')
    require(data['python_regression']['tests']==746 and data['python_regression']['summary']==['Ran 746 tests in 41.936s\n\nOK'],'full746 actual regression journal')


def validate_package(root,manifest=False):
    root=root.resolve(); raw=(root/'observations.json').read_bytes(); data=json.loads(raw)
    require(data['schema']=='resident-manifest-content-evidence/v1' and data['commit']=='de8533b93','historical v74 schema/commit')
    require(data['decision']=='accepted-default-off-scoped-combined-retouch' and all(data[k] is False for k in ('default_promotion','general_speed_claim','causal_claim','independent_manifest_lock_claim')) and data['noise']=='unknown','strictly scoped default-OFF acceptance, noise unknown')
    inventories={n:json.loads((root/'inventories'/(n+'.json')).read_text()) for n in ('source63','toolchain4020','native-source3757')}
    require([len(v) for v in inventories.values()]==[63,4020,3757],'full original inventories63/4020/3757')
    for inv in inventories.values():
        require(all(not Path(p).is_absolute() and '..' not in Path(p).parts for p in inv),'relative native inventory privacy')
    source=inventories['source63']; toolchain=sha(canonical(inventories['toolchain4020']))
    require(all(sha(safe_path(root,'pinned-source/'+p).read_bytes())==digest for p,digest in source.items()),'all63 actual pinned public source blobs')
    native=inventories['native-source3757']; require(native['crates/ra-jit/src/cargo/whole.rs']==CANDIDATE and sha(canonical(native))==data['native']['source_inventory_identity'],'actual native source3757 binds candidate')
    tc=inventories['toolchain4020']; require(tc['lib/rustlib/multirust-channel-manifest.toml']['sha256']=='34f3d9bf04640a082e561310f3f258846397f4cf64a2bcfebcea96ebf18e2572','pinned toolchain manifest')
    scopes=original_oracles(root)
    counts={'strict':strict(root,data,scopes,source,toolchain),'targeted':targeted(root,data,toolchain),'changed_manifest':changed_manifest(root,data,toolchain),'native_aba':native_aba(root,data,toolchain)}
    require(counts=={'strict':28,'targeted':30,'changed_manifest':9,'native_aba':6},'complete correctness counts')
    timing=pinned(root,data,scopes,source,toolchain)
    frozen_and_native(root,data)
    cleanup=data['cleanup']; require(cleanup['stores_deleted'] is False and len(cleanup['rows'])==4,'non-destructive four-state cleanup')
    require(all(r=={'post_measurement_only':True,'status':0,'error':None,'timed_out':False} for r in cleanup['rows']),'actual cleanup statuses')
    require('/home/' not in raw.decode() and 'Authorization' not in raw.decode() and 'cmdline_hex' not in raw.decode(),'no private environment/proc path dump')
    require(all(set(v)=={'sha256','bytes'} and re.fullmatch('[0-9a-f]{64}',v['sha256']) and type(v['bytes']) is int and v['bytes']>=0 for v in data['private_originals'].values()),'original private SHA/size receipts')
    for name,digest in data['artifacts'].items(): require(sha(safe_path(root,name).read_bytes())==digest,'historical artifact commitment '+name)
    require(sha(raw)==OBSERVATIONS,'fixed historical observation commitment, resealing is not authenticity')
    if manifest:
        expected=json.loads((root/'SHA256.json').read_text())
        actual={str(p.relative_to(root)):sha(p.read_bytes()) for p in root.rglob('*') if p.is_file() and p.name!='SHA256.json' and '__pycache__' not in p.parts}
        require(expected==actual,'full package checksum manifest')
    return {'decision':data['decision'],'correctness':counts,'native':[53,369],'timing':timing,'noise':'unknown','default_promotion':False,'native_jobs_executed':0}


if __name__=='__main__':
    parser=argparse.ArgumentParser(description=__doc__); parser.add_argument('--manifest',action='store_true'); args=parser.parse_args()
    print(json.dumps(validate_package(Path(__file__).resolve().parent,args.manifest),sort_keys=True))
