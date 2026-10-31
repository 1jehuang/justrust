#!/usr/bin/env python3
"""Portable stdlib-only replay of historical v76 evidence, never a native run.
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

OBSERVATIONS = '72772554dbf30ef8464cbc0aa93f3769770308a23a91048117c772fef8a3e63b'
BINARY = '7421e5f8ea67efb0e74f63ae7b60673aa7b3064ad04c56c34d374aab180d8719'
REFERENCE = 'fa622aeca09858c050264f701fbbebd1a14aa07d2b02c1eb4de511b684ba0263'
ORIGINAL = 'e8c984ea20e4376c21307ee67a2306851945b80b32f4807439c05ddc09b0d798'
CANDIDATE = 'f9abc7bd51719736080cbd707faae275793e5d74ba8e6c6be520b3c9e6e491d2'
FLAG = 'RA_JIT_MANIFEST_CONTENT_GENERATION'
WATCHED = 'RA_JIT_WATCHED_MANIFEST_BYTES'
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
 'resident_suite':('a2630be644b20dd91b81de8fdf1132adcc2dd8277e085f891c630b222f2065f5',('retouch_fixture',)),
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
        require(case['controls']=={'RA_JIT_DEMAND_STD':'1','RA_JIT_CACHE_MISS_STATS':'1','RA_JIT_STATS':'1','RA_JIT_WHOLE_RELOAD':'0',FLAG:'1',WATCHED:str(int(enabled))},'strict exact startup controls')
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
            require(row['marker_lines']==[] and row['entry_marker_count']==1,'once-process entry, not reuse proof')
    return len(nonces)


def worker_key(row,group):
    w=row['worker']; pid=w['pid']
    require(type(pid) is int and pid>0 and w['exe']=='<ENGINE>' and w['exe_sha256']==BINARY,'candidate worker executable')
    require(w['stat'].startswith(str(pid)+' ('),'proc stat PID')
    tail=w['stat'].rsplit(') ',1)[1].split()
    require(len(tail)>=20 and tail[0] not in ('Z','X','x') and tail[19].isdigit(),'live native proc start identity')
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
    require(p['schema']=='native_resident_watched_manifest_retouch/v1' and p['mode']=='watched-manifest-retouch' and p['factor']==WATCHED and p['pin']=='f97fcc2','distinct scoped schema/pin')
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
        require(p['controls'][g]=={FLAG:'0' if g=='reference' else '1',WATCHED:str(int(arm=='on')),'BLAZE_REMOTE':'off','RA_JIT_REMOTE':'off','BLAZE_PROC_MACRO_SERVER':'0','RA_JIT_PROC_MACRO_SERVER':'0'},'exact OFF/ON startup flag, local and compatibility proc-server controls')
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
        expected_command=['--version'] if phase=='version' else ['test','--lib','--','summary::tests::','--test-threads=1','--nocapture']
        if group=='reference' and phase!='version': expected_command.insert(0,'cargo')
        require(row['command_arguments']==expected_command,'exact frozen pinned command arguments')
        require(row['flag']==('0' if group=='reference' else '1') and row['flags']=={FLAG:row['flag'],WATCHED:str(int(enabled))},'per-call both exact flags, fixed whole-generation ON')
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
            require(row['entry_marker_count']==1,'actual whole-generation entry in BOTH watcher arms')
            identity=worker_key(row,group)
            if phase=='retouch':
                require(identity==identities[group],'samePID plus starttick in BOTH watcher arms')
                require([t['file'] for t in row['touches']]==['Cargo.toml','Cargo.lock'],'same original combined watched fileset')
                for t in row['touches']:
                    before,after=t['before'],t['after']; raw=(root/'pinned'/(t['file']+'.original')).read_bytes()
                    require(before['sha256']==after['sha256']==sha(raw) and before['size']==after['size']==len(raw),'same-byte hash/size proof')
                    require(type(before['mtime_ns']) is int and type(after['mtime_ns']) is int and after['mtime_ns']>before['mtime_ns'],'observed strictly changed mtime')
                samples['on' if enabled else 'off'].append(row['elapsed_seconds'])
                if row['replicate']==0: first.append({'group':group,'arm':'on' if enabled else 'off','seconds':row['elapsed_seconds']})
            else: require(row['touches']==[],'warmup no measured touch')
            identities[group]=identity
        else:
            require(row['nonce'] is None and row['nonce_environment']=={} and row['touches']==[],'semantic gates not measured nonce edits')
            if group=='reference' or phase=='version': require(row['entry_marker_count']==0,'reference/version no marker')
    measurements={a:{'samples_seconds':v,'median_seconds':statistics.median(v)} for a,v in samples.items()}
    for arm in ('off','on'):
        result=p[arm]
        require(result['schema']==p['schema'] and result['arm']==arm and result['noise']=='unknown' and result['scope']==p['scope'],'compatible scoped arms, unknown noise')
        require(result['measurements']==measurements and len(samples[arm])==10,'recomputed all20 samples/medians, first touches retained')
    require(len(nonces)==24 and len(first)==4,'all four segment first touches and24 fresh nonces retained')
    require([r['group'] for r in first]==['segment-0','segment-1','segment-2','segment-3'],'first touches in ABBA order')
    old,new=measurements['off']['median_seconds'],measurements['on']['median_seconds']
    require(p['comparison_provenance']=='independently recomputed from original records; not a preserved console transcript','honest comparison provenance')
    witness=p['root_compare_witness']
    require(witness['attribution']=='coordinator skunk posthoc message, not a raw transcript file'
            and witness['tool_time_utc']=='2026-10-31T15:19:17.896Z' and witness['exit']==0
            and witness['reported_off_seconds']==f'{old:.6f}' and witness['reported_on_seconds']==f'{new:.6f}'
            and witness['reported_delta_percent']==f'{100*(new/old-1):.2f}','attributed posthoc root comparison arithmetic')
    return {'off_median_seconds':old,'on_median_seconds':new,'delta_percent':100*(new/old-1),'all_first_touches':first,'calls':len(rows),'measured':20,'runtime_nonces':len(nonces)}



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
        m=re.match(r'@@ -(\d+)(?:,(\d+))? \+(\d+)(?:,(\d+))? @@',lines[at]); require(m is not None,'patch hunk')
        position=max(int(m[1])-1,0); require(cursor<=position<=len(old),'patch order')
        result.extend(old[cursor:position]); cursor=position; at+=1
        require(max(int(m[3])-1,0)==len(result),'patch new hunk offset')
        old_count,new_count=0,0
        while at<len(lines) and not lines[at].startswith('@@ '):
            kind,line=lines[at][0],lines[at][1:]; require(kind in ' +-','patch prefix')
            if kind in ' -': require(cursor<len(old) and old[cursor]==line,'patch exact context'); cursor+=1; old_count+=1
            if kind in ' +': result.append(line); new_count+=1
            at+=1
        require(old_count==int(m[2] or 1) and new_count==int(m[4] or 1),'patch exact hunk lengths')
    result.extend(old[cursor:]); return ''.join(result)


def patches(path):
    lines=[line for line in path.read_text().splitlines(True)
           if not line.startswith(('diff --git ', 'index '))]
    return re.split(r'(?=^--- )',''.join(lines),flags=re.M)[1:]


def frozen_and_native(root,data):
    f=root/'frozen/watched-manifest-bytes-v76'
    rust='toolchain/resident/crates/ra-jit/src/inputs.rs'
    original=(f/'originals'/rust).read_bytes(); candidate=(f/'candidates'/rust).read_bytes()
    require(sha(original)==ORIGINAL and sha(candidate)==CANDIDATE,'immutable original/candidate Inputs source')
    require(data['source_freeze']=={'baseline':'de8533b93','accepted':'0aa1d0727',
            'rust_sha256':CANDIDATE,'original_rust_sha256':ORIGINAL,
            'doc_join':'initial -> finaldoc -> finaldoc-regression -> root lineage clarification'},'exact freeze ancestry')
    pieces=patches(f/'candidate.patch')
    require(len(pieces)==2 and replay_patch(original.decode(),pieces[0])==candidate.decode(),'native full patch replay')
    doc=(f/'candidates/docs/resident-watched-manifest-bytes.md').read_text()
    require(replay_patch('',pieces[1])==doc,'initial doc full patch replay')
    for folder in ('finaldoc','finaldoc-regression'):
        d=f/folder
        require((d/'originals/docs/resident-watched-manifest-bytes.md').read_text()==doc,'consecutive exact doc join '+folder)
        pieces=patches(d/'candidate.patch'); require(len(pieces)==1,'single final doc patch')
        doc=replay_patch(doc,pieces[0])
        require(doc==(d/'candidates/docs/resident-watched-manifest-bytes.md').read_text(),'full final doc patch replay '+folder)
    accepted=root/'frozen/root-doc'
    doc=replay_patch(doc,(accepted/'lineage-clarification.patch').read_text())
    require(doc==(accepted/'accepted.md').read_text() and 'licensed Blaze-derived' in doc,'root lineage clarification exact join')
    h=root/'frozen/watched-manifest-suite-v76'
    pieces=patches(h/'candidate.patch'); require(len(pieces)==2,'both full harness patches')
    for piece,name in zip(pieces,('resident_suite.py','test_resident_manifest_retouch.py')):
        before=(h/'originals'/name).read_text(); after=(h/'candidates'/name).read_text()
        require(replay_patch(before,piece)==after,'full harness patch replay '+name)
        require(after==(root/'oracles'/(name+'.txt')).read_text(),'exact accepted frozen harness '+name)
        if name=='resident_suite.py':
            require(sha(before.encode())=='1508a424fde23d7093ae004bc476abcf95d7681e36ed61d9d5c923de9f9cb2df','original v74 harness SHA')
        else:
            def klass(raw):
                return next(n for n in ast.parse(raw).body if isinstance(n,ast.ClassDef) and n.name=='ManifestRetouchTests')
            require(ast.dump(klass(before))==ast.dump(klass(after)),'original13 test class unchanged')
    for gate in ('check','compile','codegen-native','full-native'):
        require(data['native'][gate]['exit']==0,'native exit gate')
    fixtures={'inputs::tests::equal_manifest_bytes_continue_flags_script_env_and_script_file_proofs',
              'inputs::tests::manifest_and_lock_retouch_only_reuses_selected_checkout',
              'inputs::tests::manifest_bytes_selector_preserves_original_off_and_detects_hidden_changes',
              'inputs::tests::manifest_bytes_missing_errors_and_symlink_identity'}
    for name,count in (('codegen',53),('resident',373)):
        lines=json.loads((root/'native'/(name+'-results.json')).read_text())
        matches=[re.fullmatch(r'test ([A-Za-z0-9_:]+) \.\.\. ok <[0-9.]+s>',line) for line in lines[:-1]]
        require(len(matches)==count and all(matches) and len({m[1] for m in matches})==count,'complete actual native named results')
        require(re.fullmatch(r'test result: ok\. '+str(count)+r' passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in [0-9.]+s',lines[-1]),'complete actual native raw summary')
        if name=='resident':
            require(fixtures<={m[1] for m in matches},'all four executed watcher fixtures')
    n=data['native']
    require(n['source_count']==3757 and n['source_check_ok'] is True and n['lock_check_ok'] is True and n['engine_sha256']==BINARY,'native source/lock/executable gates')
    inventory=json.loads((root/'inventories/native-source3757.json').read_text())
    before=(root/'native/all-source-before.sha256').read_text().splitlines()
    after=(root/'native/all-source-after.log').read_text().splitlines()
    require({line[66:]:line[:64] for line in before}==inventory and len(before)==3757,'actual full3757 source-before journal')
    require(after==[line[66:]+': OK' for line in before],'actual full3757 source-after journal')
    lock=(root/'native/root-Cargo.lock').read_bytes()
    require(sha(lock)==n['lock_sha256']=='bf3307542b2b5c0e7a4b8c5b1a1e650bba9afece80284cc497a3800560a0c2bc','native original root lock blob')
    require((root/'native/lock-before.sha256').read_text()==sha(lock)+'  Cargo.lock\n'
            and (root/'native/lock-after.log').read_text()=='Cargo.lock: OK\n','actual root lock journals')
    require(data['python_regression']=={'tests':761,'summary':['Ran 761 tests in 51.106s\n\nOK']},'full761 actual regression journal')
    require('Ran 761 tests in 51.106s\n\nOK' in (root/'regression/full761.log').read_text(),'archived actual761 regression output')


def aborted(root,data,source):
    a=data['aborted']; rows=a['rows']
    require(a['accepted'] is False and a['status']=='INCOMPLETE' and a['outer_timeout_seconds']==600
            and a['outer_exit']==124,'aborted outer-timeout is never accepted')
    require(a['schema']=='native_resident_watched_manifest_retouch/v1' and a['mode']=='watched-manifest-retouch'
            and a['factor']==WATCHED and a['source_before']==source,'aborted separate schema/source identity')
    require(len(rows)==52 and sum('status' in r for r in rows)==51 and 'status' not in rows[-1],
            'honest52 rows only51 completed calls; no fabricated end')
    require((rows[-1]['group'],rows[-1]['phase'],rows[-1]['replicate'])==('segment-3','retouch',2),'exact interrupted52nd call')
    require(a['stops_before_timeout']==['reference','segment-0','segment-1','segment-2'],'four pre-timeout stops only')
    cleanup=a['cleanup']; require(cleanup['source63_restored'] is True and cleanup['stores_deleted'] is False,'timeout cleanup pristine63 proof and no deletion')
    successful(cleanup['stop'])
    for name,file in (('source.original','src/summary.rs'),('Cargo.toml.original','Cargo.toml'),('Cargo.lock.original','Cargo.lock')):
        require(sha((root/'aborted'/name).read_bytes())==source[file],'aborted original backup bound to pin63')
    require((root/'aborted/source.fixture').read_bytes()==(root/'pinned/source.fixture').read_bytes(),'same fixed fixture, never merged samples')
    require(not any(k in a for k in ('off','on','measurements','median_seconds')),'incomplete run has no accepted estimator')


def script_manifest(root,data,toolchain):
    s=data['script_manifest']
    require(s['accepted'] is True and s['results']=={'off':True,'on':True} and s['engine_sha256']==BINARY
            and s['caches_retained'] is True,'supplemental both-arm actual correctness summary')
    require(s['driver_original_sha256']=='1b833741cfeb059e4e47a1e84e73caf596083e6a2c4d37623c17be7435a1aa6f',
            'supplemental executed future-clock driver identity')
    raw=(root/'drivers/probe-manifest-script-retouch-v77.py.txt').read_text()
    tree=ast.parse(raw)
    files=ast.literal_eval(next(n.value for n in tree.body if isinstance(n,ast.Assign)
                               and any(isinstance(t,ast.Name) and t.id=='FILES' for t in n.targets)))
    require(set(files)=={'Cargo.toml','Cargo.lock','src/main.rs','build.rs'},'exact supplemental source roles')
    require(b'cargo:rerun-if-changed=Cargo.toml' in files['build.rs']
            and b'Command::new(cc)' in files['build.rs'] and b'Command::new(ar)' in files['build.rs'],
            'real manifest-watching native compiler/archive script')
    f=root/'frozen/manifest-script-retouch-v77-future'
    candidate=(f/'candidates/probe-manifest-script-retouch-v77.py').read_text()
    require(candidate==raw,'exact archived executed driver joins supplemental freeze')
    pieces=patches(f/'candidate.patch')
    require(len(pieces)==1 and replay_patch((f/'originals/probe-manifest-script-retouch-v77.py').read_text(),pieces[0])==candidate,
            'supplemental future-clock patch join, never executes older proposal')
    require(set(s['arms'])=={'off','on'},'both supplemental arms')
    nonces=set(); script_pids=set()
    tools={'cc':'8aac907d6fbf40394b424b5fa4a2ad1aa291dac7d3a0315c5b415c1f5ace1287',
           'ar':'41861189eb10fb9fe73ffcb8c82bdbbe5f6084de78e03f15acddd1d33071c3a4'}
    for arm,case in s['arms'].items():
        require(case['arm']==arm and case['status']=='passed' and all(case[k] is True for k in
                ('source_restored','manifest_mtime_restored','stop_accepted','identities_unchanged','caches_retained')),
                'supplemental restored source/stops and identity checks')
        flags={FLAG:'1',WATCHED:'1' if arm=='on' else '0'}
        require(case['flags']==flags,'supplemental fixed whole-generation and watcher factor')
        require(case['engine_before']['sha256']==case['engine_after']['sha256']==BINARY
                and case['toolchain_before']==case['toolchain_after']==toolchain,'supplemental same ELF/toolchain')
        for role,digest in tools.items():
            require(case['native_tools_before'][role]['sha256']==case['native_tools_after'][role]['sha256']==digest,
                    'supplemental real cc/ar identity')
        for when in ('before','after'):
            require(set(case['source_'+when])==set(files),'supplemental all source files recorded')
            for name,contents in files.items():
                require((root/'script-manifest'/arm/'source'/name).read_bytes()==contents
                        and case['source_'+when][name]['sha256']==sha(contents)
                        and case['source_'+when][name]['size']==len(contents),'supplemental independently derived original source bytes')
        clock=case['clock_setup']; base=((clock['clock_ns']//10**9)//1000+2)*1000
        require(clock['base']==base and clock['formula']=='((time.time_ns()//1_000_000_000)//1000+2)*1000'
                and case['seconds']==[base+11,base+22,base+11],'supplemental exact controlled-future A/B/A recipe')
        require(len(case['rows'])==3,'supplemental all three calls per arm')
        previous=''
        for index,row in enumerate(case['rows']):
            row_common(row,nonces); successful(row)
            seconds=case['seconds'][index]
            require(row['index']==index and row['mtime_seconds']==seconds and row['expected']==seconds%1000,
                    'supplemental timestamp-derived native result')
            require(row['flags']==flags and row['command_arguments']==['run'],'supplemental explicit flags and real run command')
            require(clock['clock_ns']<=row['clock_before_ns']<=row['clock_launch_ns']<seconds*10**9
                    and row['clock_after_ns']>=row['clock_launch_ns'],'supplemental unexpired future-clock guards')
            for snap in (row['manifest_before'],row['manifest_after']):
                require(snap['sha256']==sha(files['Cargo.toml']) and snap['size']==len(files['Cargo.toml'])
                        and snap['mtime_ns']==seconds*10**9,'supplemental same manifest bytes with observed chosen mtime')
            for name,contents in files.items():
                require(row['source_before'][name]['sha256']==sha(contents)
                        and row['source_before'][name]['size']==len(contents),'supplemental percall unchanged source bytes')
            marker='MANIFEST_NATIVE_'+row['nonce']+'_'+str(seconds%1000)
            require(row['stdout']==marker+'\n' and row['expected_marker']==marker and row['marker_lines']==[marker]
                    and row['source_bytes_unchanged'] is True,'supplemental fresh linked native runtime witness')
            before,after=row['script_proof_before'],row['script_proof_after']
            require(before==previous and after.startswith(before),'supplemental append-only actual script proof')
            added=after[len(before):]; match=re.fullmatch(r'(\d+) (\d+) (\d+)\n',added)
            require(match is not None and int(match[1])==seconds and int(match[2])==seconds%1000
                    and int(match[3])>0 and int(match[3]) not in script_pids
                    and row['script_invoked_for_expected_mtime'] is True,'supplemental actual script invocation for EACH retouch')
            script_pids.add(int(match[3])); previous=after
        require((root/'script-manifest'/arm/'build-script-invocations.log').read_text()==previous,
                'supplemental full append journal retained')
        successful(case['stop'])
    require(len(nonces)==6 and len(script_pids)==6,'supplemental six fresh native/script invocations')
    return 6


def validate_package(root,manifest=False):
    root=root.resolve(); raw=(root/'observations.json').read_bytes(); data=json.loads(raw)
    require(data['schema']=='resident-watched-manifest-evidence/v1' and data['commit']=='0aa1d0727','historical v76 schema/commit')
    require(data['decision']=='accepted-default-off-scoped-combined-retouch' and all(data[k] is False for k in ('default_promotion','general_speed_claim','causal_claim','independent_manifest_lock_claim')) and data['noise']=='unknown','strictly scoped default-OFF acceptance, noise unknown')
    inventories={n:json.loads((root/'inventories'/(n+'.json')).read_text()) for n in ('source63','toolchain4020','native-source3757')}
    require([len(v) for v in inventories.values()]==[63,4020,3757],'full original inventories63/4020/3757')
    for inv in inventories.values():
        require(all(not Path(p).is_absolute() and '..' not in Path(p).parts for p in inv),'relative native inventory privacy')
    source=inventories['source63']; toolchain=sha(canonical(inventories['toolchain4020']))
    require(all(sha(safe_path(root,'pinned-source/'+p).read_bytes())==digest for p,digest in source.items()),'all63 actual pinned public source blobs')
    native=inventories['native-source3757']; require(native['crates/ra-jit/src/inputs.rs']==CANDIDATE and sha(canonical(native))==data['native']['source_inventory_identity'],'actual native source3757 binds candidate')
    tc=inventories['toolchain4020']; require(tc['lib/rustlib/multirust-channel-manifest.toml']['sha256']=='34f3d9bf04640a082e561310f3f258846397f4cf64a2bcfebcea96ebf18e2572','pinned toolchain manifest')
    scopes=original_oracles(root)
    counts={'strict':strict(root,data,scopes,source,toolchain),'changed_manifest':changed_manifest(root,data,toolchain),'native_aba':native_aba(root,data,toolchain)}
    require(counts=={'strict':28,'changed_manifest':9,'native_aba':6},'complete correctness counts')
    timing=pinned(root,data,scopes,source,toolchain)
    frozen_and_native(root,data)
    aborted(root,data,source)
    supplemental=script_manifest(root,data,toolchain)
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
    return {'decision':data['decision'],'correctness':counts,'supplemental_manifest_script':supplemental,'native':[53,373],'timing':timing,'noise':'unknown','default_promotion':False,'native_jobs_executed':0}


if __name__=='__main__':
    parser=argparse.ArgumentParser(description=__doc__); parser.add_argument('--manifest',action='store_true'); args=parser.parse_args()
    print(json.dumps(validate_package(Path(__file__).resolve().parent,args.manifest),sort_keys=True))
