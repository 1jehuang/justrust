#!/usr/bin/env python3
"""Explicit v85 historical export. Read-only private inputs, no native workloads.
Only read-only git show retrieves public immutable source63. Replay is separate.
"""
import argparse
import hashlib
import json
import re
import subprocess
from pathlib import Path

ENV = {'BLAZE_BACKEND':'cranelift','BLAZE_OPT':'0',
       'CARGO_PROFILE_DEV_DEBUG':'0','CARGO_PROFILE_DEV_INCREMENTAL':'true','CARGO_PROFILE_DEV_OPT_LEVEL':'0',
       'CARGO_PROFILE_TEST_DEBUG':'0','CARGO_PROFILE_TEST_INCREMENTAL':'true','CARGO_PROFILE_TEST_OPT_LEVEL':'0'}
CONTROLS = ('RA_JIT_DEMAND_STD','RA_JIT_CACHE_MISS_STATS','RA_JIT_STATS','RA_JIT_WHOLE_RELOAD',
            'RA_JIT_WATCHED_MANIFEST_BYTES','RA_JIT_MANIFEST_CONTENT_GENERATION','RA_JIT_WARM_REGISTRY_FETCH')
FIELDS = ('group','phase','replicate','flag','flags','nonce','nonce_environment','touches','status','error',
          'timed_out','elapsed_seconds','accepted','raw_error','parsed','input_unchanged','failures',
          'inputs','source_before','source_after','schema','run_id','rules')
WARM = 'ra-jit: warm-registry-fetch-v85 batch'
ENTRY = 'ra-jit: manifest-content-generation-v1 entered'

def sha(raw):
    return hashlib.sha256(raw).hexdigest()

def canonical(value):
    return json.dumps(value,sort_keys=True,separators=(',',':')).encode()

def collect(root,pinned,repo,out):
    out.mkdir(parents=True,exist_ok=True)
    originals,provenance,artifacts = {},{},{}
    def read(path):
        path=Path(path); raw=path.read_bytes()
        label=('pinned/'+str(path.relative_to(pinned)) if path.is_relative_to(pinned)
               else 'experiment/'+str(path.relative_to(root)) if path.is_relative_to(root)
               else 'repository/'+str(path.relative_to(repo)))
        provenance[label]={'sha256':sha(raw),'bytes':len(raw)}
        originals[path]=sha(raw)
        return raw
    def load(path): return json.loads(read(path))
    def save(name,raw):
        path=out/name; path.parent.mkdir(parents=True,exist_ok=True); path.write_bytes(raw)
        artifacts[name]=sha(raw)
    def jsave(name,value): save(name,(json.dumps(value,sort_keys=True,indent=2)+'\n').encode())
    def text(raw):
        value=raw.decode().replace(str(root),'<EXPERIMENT>').replace(str(pinned),'<PINNED_RUN>').replace(str(repo.parent),'<PRIVATE_HOME>')
        assert str(repo.parent) not in value
        return value.encode()
    def process(row): return {k:row[k] for k in ('status','error','timed_out','elapsed_seconds')}
    def stdout(raw):
        value=raw.decode()
        patterns=(r' running [0-9]+ tests',r'running [0-9]+ tests',r'\s*',r'ok',r'FAILED',r'failures:',
          r'    summary::tests::shares_overlapping_time',
          r'test summary::tests::[a-z_]+ \.\.\. (?:ok|FAILED|JR_(?:MATRIX_RUNTIME|MANIFEST_RETOUCH)_[0-9a-f]{32}|JR_RUNTIME_EXECUTED_[0-9]+)',
          r'test result: (?:ok|FAILED)\. [0-9]+ passed; [0-9]+ failed; 0 ignored; 0 measured; [0-9]+ filtered out; finished in [0-9.]+s',
          r'blaze 0\.20261027\.0 \([0-9a-f]{40}\)',
          r'jr-engine experimental 0\.20261027\.0 \(Blaze-derived origin [0-9a-f]{40}; build resident-v1-[0-9a-f]{64}\)',
          r'JR_RUNTIME_EXECUTED_[0-9]+')
        assert all(any(re.fullmatch(p,line) for p in patterns) for line in value.splitlines()),'unknown stdout grammar '+repr(value)
        return value
    def raw(stem,row=None):
        a,b=read(str(stem)+'.stdout'),read(str(stem)+'.stderr')
        if row and 'raw' in row:
            for k,v in [('stdout',a),('stderr',b)]:
                assert row['raw'][k]['sha256']==sha(v) and row['raw'][k]['bytes']==len(v)
        diagnostics=[]
        for line in b.decode().splitlines():
            if any(re.fullmatch(pattern,line) for pattern in (
                r"error\[E0308\]: expected u(?:32|64), found &'static str",
                r'\s*\d+ \| let _: u64 = "JR_(?:MATRIX_TYPE_[0-9a-f]{32}|RUNTIME_TYPE_ERROR)";',
                r'assertion `left == right` failed: JR_(?:MATRIX_FAILURE_[0-9a-f]{32}|RUNTIME_FAILURE)')):
                diagnostics.append(line)
        assert all('/home/' not in line for line in diagnostics)
        result={'stdout':stdout(a),'diagnostic_lines':diagnostics,
                'raw_original':{k:{'sha256':sha(v),'bytes':len(v)} for k,v in [('stdout',a),('stderr',b)]},
                'entry_marker_count':b.decode().splitlines().count(ENTRY),
                'warm_marker_lines':[line for line in b.decode().splitlines() if line==WARM]}
        assert b.decode().count('ra-jit: warm-registry-fetch-')==len(result['warm_marker_lines'])
        if Path(str(stem)+'-status.stdout').exists():
            status=read(str(stem)+'-status.stdout').decode();read(str(stem)+'-status.stderr')
            result['status_worker_lines']=['worker: pid '+n for n in re.findall(r'worker: pid (\d+)',status)]
        return result
    def export_row(row,stem):
        item={k:row[k] for k in FIELDS if k in row};item.update(raw(stem,row))
        item['safe_environment']={k:row['environment'][k] for k in ENV}
        assert item['safe_environment']==ENV
        item['omitted_environment_entries']=len(row['environment'])-len(ENV)
        item['noise_contamination']=row['noise']['contamination']
        for k in ('worker_status','status_receipt'):
            if k in row:item[k]=process(row[k])
        return item
    data={'schema':'resident-warm-registry-evidence/v1','decision':'rejected-below-10-percent-floor',
          'noise':'unknown','default_promotion':False,'general_speed_claim':False,'source_restored':True}
    meta=load(pinned/'metadata.json')
    assert meta['source_before']==meta['source_after'] and meta['tools_before']==meta['tools_after']
    assert meta['toolchain_before']==meta['toolchain_after'] and meta['host_before']==meta['host_after']
    for label,inv in [('pinned-source63',meta['source_before']),('toolchain4020',meta['toolchain_before']['files'])]:
        jsave('inventories/'+label+'.json',inv)
    for name,digest in meta['source_before'].items():
        blob=subprocess.check_output(['git','-C',str(repo),'show','f97fcc2:'+name])
        assert sha(blob)==digest,name
        save('pinned-source/'+name,blob)
    for name in ('source.original','source.fixture','Cargo.toml.original','Cargo.lock.original'):
        save('pinned/'+name,read(pinned/name))
    p={k:meta[k] for k in ('schema','pin','order','samples','source_before','source_after','fixture_source','status',
         'source_restored','toolchain_links_valid','profiles','scope','store_scope','rules','run_id','factor','mode')}
    p['tools']={k:v['sha256'] for k,v in meta['tools_before'].items()}
    p['toolchain']=sha(canonical(meta['toolchain_before']['files']))
    p['controls']={g:{k:v for k,v in env.items() if k in CONTROLS or k in ('BLAZE_REMOTE','RA_JIT_REMOTE','BLAZE_PROC_MACRO_SERVER','RA_JIT_PROC_MACRO_SERVER')} for g,env in meta['environments'].items()}
    p['host']={k:meta['host_before'][k] for k in ('machine','cpu','ncpu') if k in meta['host_before']}
    p['private_states']={}
    for key in ('RA_JIT_HOME','CARGO_HOME','CARGO_TARGET_DIR','RA_JIT_CACHE','BLAZE_CACHE','RUSTUP_HOME'):
        places=[env[key] for env in meta['environments'].values()]
        assert len(set(places))==5 and all(Path(v).is_relative_to(pinned) for v in places)
        p['private_states'][key]={g:'<PRIVATE_'+g+'_'+key+'>' for g in meta['environments']}
    p['stops']=[{'group':s['group'],'process':process(s['process'])} for s in meta['stops']]
    for stop in meta['stops']:
        for suffix in ('.stdout','.stderr'):read(pinned/(stop['group']+'-stop'+suffix))
    p['records']=[]
    for index,row in enumerate(load(pinned/'records.json')):
        item=export_row(row,pinned/f'call-{index:03}')
        if 'worker' in row:
            w=row['worker'];argv=bytes.fromhex(w['cmdline_hex']).decode().split('\0');assert argv.pop()==''
            assert argv[0]==meta['tools_before']['engine']['path']==w['exe'] and argv[1].endswith('/source63/')
            normalized=['<ENGINE>','<SOURCE>/'];args=iter(argv[2:])
            for arg in args:
                if arg in ('--own-target','--cfg-test','--dev-deps'):normalized.append(arg)
                elif arg in ('--profile','--asked-in','--crate','--out','--patch','--exe','--socket','--symbols-in'):
                    value=next(args);normalized.append(arg)
                    if arg=='--profile':assert value=='dev';normalized.append(value)
                    elif arg=='--crate':assert value=='src/lib.rs';normalized.append(value)
                    elif arg=='--asked-in':assert value.endswith('/source63');normalized.append('<SOURCE>')
                    elif arg in ('--socket','--symbols-in'):
                        rel=str(Path(value).relative_to(row['environment']['RA_JIT_HOME']))
                        assert re.fullmatch(r'workers/[0-9a-f]+/(?:sock|symbols)',rel)
                        normalized.append('<PRIVATE_'+row['group']+'_HOME>/'+rel)
                    else:normalized.append('<PRIVATE_'+row['group']+'_TARGET>/'+str(Path(value).relative_to(row['environment']['CARGO_TARGET_DIR'])))
                else:raise ValueError('unknown worker arg '+arg)
            fields=w['stat'].rsplit(') ',1)[1].split()
            item['worker']={'pid':w['pid'],'state':fields[0],'start_ticks':int(fields[19]),'exe':'<ENGINE>',
                            'exe_sha256':w['exe_sha256'],'argv':normalized,
                            'original_stat':{'sha256':sha(w['stat'].encode()),'bytes':len(w['stat'].encode())},
                            'original_cmdline':{'sha256':sha(bytes.fromhex(w['cmdline_hex'])),'bytes':len(bytes.fromhex(w['cmdline_hex']))}}
        p['records'].append(item)
    for arm in ('off','on'):
        result=load(pinned/(arm+'.json'));assert result['metadata']==meta
        p[arm]={k:result[k] for k in ('schema','arm','measurements','noise','scope')}
    p['printed_compare']=read(root/'warm-registry-fetch-v85-compare.txt').decode().strip()
    data['pinned']=p
    data['strict']={}
    for case in ('off','on','on-repeat','off-repeat'):
        folder=root/('warm-registry-fetch-seven-v85b-'+case);state=load(folder/'state.json')
        s={k:state[k] for k in ('status','unchanged','source_before','source_after','tool_before','tool_after')}
        s['controls']=state['startup_controls']
        assert set(s['controls'])==set(CONTROLS)
        for k in ('toolchain_before','toolchain_after'):s[k]=sha(canonical(state[k]['files']))
        s['rows']=[]
        for row in state['rows']:
            stem=Path(row['raw']['stdout']['path']).with_suffix('');assert stem.parent==folder
            s['rows'].append(export_row(row,stem))
        data['strict'][case]=s
        for i in range(3):save(f'strict/{case}/original-{i}',read(folder/f'original-{i}'))
    profile=root/'resolver-subphases-profile-v85';state=load(profile/'state.json')
    data['profile']={k:state[k] for k in ('status','unchanged','source_before','source_after','tool_before','tool_after')}
    data['profile']['rows']=[]
    save('profile/source.original',read(profile/'source.original'))
    for row in state['rows']:
        stem=profile/f"{row['replicate']}-{row['kind']}"
        trace=load(str(stem)+'.trace.json');assert all(event in trace['traceEvents'] for event in row['events'])
        events=[]
        for event in row['events']:
            assert set(event)=={'cat','dur','name','ph','pid','tid','ts'}
            assert re.fullmatch(r'[a-zA-Z0-9 /↔_-]+',event['name'])
            events.append(event)
        item={k:row[k] for k in ('kind','replicate','marker','source_sha256','trace_sha256','resolver_labels_observed')}
        item.update(export_row(row,stem));item['events']=events
        data['profile']['rows'].append(item)
    data['profile']['stop']=process(state['stop'])
    data['cleanup']={'stores_deleted':False,'rows':[]}
    cleanup=load(root/'completed-warm-registry-fetch-v85-stop/receipt.json');assert cleanup['stores_deleted'] is False
    for row in cleanup['rows']:
        item={k:row[k] for k in ('case','post_measurement_only','status','error','timed_out','elapsed_seconds')}
        item['noise_contamination']=row['noise']['contamination'];data['cleanup']['rows'].append(item)
    data['native']={}
    runs=root/'candidate-unit-v85/justrust-home/runs'
    expected=[('registry',41),('manifest',22),('metadata',0),('codegen',53),('full',373)]
    for (kind,total),journal in zip(expected,sorted(runs.glob('*/output.jsonl'))):
        lines=[json.loads(line)['l'] for line in read(journal).decode().splitlines()]
        selected=[line for line in lines if re.fullmatch(r'test [a-zA-Z0-9_:]+ \.\.\. (?:ok|ignored|FAILED)(?: <[0-9.]+s>)?',line) or re.fullmatch(r'test result: .*',line)]
        assert all('/home/' not in line for line in selected)
        passed=sum(int(n) for n in re.findall(r'test result: ok\. (\d+) passed;', '\n'.join(selected)))
        assert passed==total,(kind,passed)
        receipt={'lines':selected,'passed':passed,'status':int(read(root/f'candidate-unit-v85/{kind}-native.exit'))}
        jsave(f'native/{kind}.json',receipt);data['native'][kind]=receipt
    before=read(root/'candidate-build-v85/all-source-before.sha256').decode()
    inv={line.split('  ',1)[1]:line.split('  ',1)[0] for line in before.splitlines()}
    after=read(root/'candidate-unit-v85/all-source-after.log').decode().splitlines()
    assert after==[name+': OK' for name in inv]
    jsave('inventories/native-source3757.json',inv)
    lock_before=read(root/'candidate-build-v85/lock-before.sha256').decode().split('  ',1)[0]
    assert read(root/'candidate-unit-v85/lock-after.log').decode()=='Cargo.lock: OK\n'
    data['native_source_count']=len(inv);data['root_lock_sha256']=lock_before
    data['build_exits']={n:int(read(root/f'candidate-build-v85/{n}')) for n in ('check.exit','compile.exit','orchestration-exit')}
    data['unit_exit']=int(read(root/'candidate-unit-v85/exit'))
    data['engine_identity']=read(root/'candidate-build-v85/engine-identity.sha256').decode().split('  ',1)[0]
    save('native/engine-version.txt',read(root/'candidate-build-v85/engine-version.txt'))
    # Fixed individual historical inputs only, never recursively sweep private directories.
    frozen_names={
       'warm-registry-fetch-v84':['original/toolchain/resident/crates/blaze-registry/src/download.rs','candidate/toolchain/resident/crates/blaze-registry/src/download.rs','candidate/docs/resident-warm-registry-fetch.md','candidate.patch','checks.json','RELEASE','STOPEDITS','SHA256SUMS'],
       'warm-registry-fetch-v85':['original/toolchain/resident/crates/blaze-registry/src/download.rs','candidate/toolchain/resident/crates/blaze-registry/src/download.rs','candidate/docs/resident-warm-registry-fetch-readonly.md','candidate.patch','checks.json','RELEASE','STOPEDITS','SHA256SUMS','source_checks.py'],
       'warm-registry-fetch-v85-rollback':['live/toolchain/resident/crates/blaze-registry/src/download.rs','live/docs/resident-warm-registry-fetch-readonly.md','live/docs/resident-warm-registry-fetch.md']}
    for folder,names in frozen_names.items():
        for name in names:
            value=read(root/folder/name)
            save('frozen/'+folder+'/'+name,value if name.endswith('.rs') else text(value))
    drivers=['suite-frozen-warm-registry-v85.py','profile-warm-registry-fetch-v85.py','probe-warm-registry-fetch-seven-v85.py',
             'probe-warm-registry-fetch-seven-v85.initial-argparse-failure.py','build-test-candidate-v85.sh','cleanup-completed-warm-registry-fetch-v85.py',
             'warm-registry-factor-v85-mock-tests.log']
    for name in drivers:save('drivers/'+name+'.txt',text(read(root/name)))
    for name in ['resident_suite.py','test_resident_parallel_manifest.py']:
        save('oracles/v81-'+name+'.txt',read(root/'parallel-manifest-harness-v81/candidates/bench'/name))
        if name == 'resident_suite.py':
            save('oracles/resident_suite.py.txt',read(root/'parallel-manifest-harness-v81/candidates/bench'/name))
    for name in ['resident_state_matrix','resident_engine_compare','resident_workload_compare','blaze_runtime_compare','blaze_edit_compare']:
        save('oracles/'+name+'.py.txt',read(repo/'bench'/(name+'.py')))
    for name in ['LICENSE-APACHE','LICENSE-MIT','README.md','Cargo.toml']:
        save('lineage/'+name,read(repo/'toolchain/resident'/name))
    data['launch_failures']={}
    for task in ['968422e0vc','403413eyir']:
        raw_failure=read(root/('v85-root-failure-'+task+'.log'))
        selected=[line for line in text(raw_failure).decode().splitlines() if
                  (line.startswith('[stderr] cmp: <EXPERIMENT>/warm-registry-fetch-v85/candidate/download.rs: No such file or directory')
                   or line.startswith("[stderr] AttributeError: 'Namespace' object has no attribute 'native_warm_registry'")
                   or line.startswith('--- Command finished with exit code: '))]
        assert len(selected)==2
        data['launch_failures'][task]={'lines':selected,'native_jobs_started':0}
    regression=read(root/'v85-restored-full-harness-regression.log').decode()
    data['historical_harness_regression']={'lines':[line for line in regression.splitlines() if re.fullmatch(r'Ran 761 tests in [0-9.]+s|OK',line)]}
    assert len(data['historical_harness_regression']['lines'])==2
    data['artifacts']=dict(artifacts)
    data['private_originals']=provenance
    for path,digest in originals.items():assert sha(path.read_bytes())==digest,'private input changed'
    jsave('observations.json',data)
    print(json.dumps({'observations_sha256':sha((out/'observations.json').read_bytes()),'artifacts':len(artifacts),'native':{k:v['passed'] for k,v in data['native'].items()},'native_source_count':len(inv)},indent=2))

if __name__=='__main__':
    p=argparse.ArgumentParser();p.add_argument('--root',type=Path,required=True);p.add_argument('--pinned',type=Path,required=True);p.add_argument('--repo',type=Path,required=True);p.add_argument('--out',type=Path,required=True)
    a=p.parse_args();collect(a.root,a.pinned,a.repo,a.out)
