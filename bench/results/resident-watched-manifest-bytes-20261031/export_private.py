#!/usr/bin/env python3
"""Fixed historical v76 allowlist exporter. No builds or native execution.
Only this exporter uses git read-only to retrieve immutable public text blobs.
The standalone validator never imports this module or executes subprocesses.
"""
import argparse
import base64
import difflib
import hashlib
import json
import re
import subprocess
from pathlib import Path

COMMIT = '0aa1d0727'
ENV = {'BLAZE_BACKEND': 'cranelift', 'BLAZE_OPT': '0',
       'CARGO_PROFILE_DEV_DEBUG': '0', 'CARGO_PROFILE_DEV_INCREMENTAL': 'true',
       'CARGO_PROFILE_DEV_OPT_LEVEL': '0', 'CARGO_PROFILE_TEST_DEBUG': '0',
       'CARGO_PROFILE_TEST_INCREMENTAL': 'true', 'CARGO_PROFILE_TEST_OPT_LEVEL': '0'}
CONTROLS = ('RA_JIT_DEMAND_STD', 'RA_JIT_CACHE_MISS_STATS', 'RA_JIT_STATS',
            'RA_JIT_WHOLE_RELOAD', 'RA_JIT_MANIFEST_CONTENT_GENERATION', 'RA_JIT_WATCHED_MANIFEST_BYTES')
MARKER = 'ra-jit: manifest-content-generation-v1 entered'
FIELDS = ('group', 'phase', 'replicate', 'flag', 'flags', 'nonce', 'nonce_environment', 'touches',
          'status', 'error', 'timed_out', 'elapsed_seconds', 'accepted', 'raw_error',
          'parsed', 'input_unchanged', 'failures', 'mode', 'literal', 'startup_control',
          'rust_before', 'rust_after', 'choice', 'expected', 'source_at_call', 'inputs',
          'worker_pids')


def sha(raw):
    return hashlib.sha256(raw).hexdigest()


def canonical(value):
    return json.dumps(value, sort_keys=True, separators=(',', ':')).encode()


def seal(output):
    inventory = {str(p.relative_to(output)): sha(p.read_bytes()) for p in output.rglob('*')
                 if p.is_file() and p.name != 'SHA256.json' and '__pycache__' not in p.parts}
    (output / 'SHA256.json').write_text(json.dumps(inventory, sort_keys=True, indent=2) + '\n')


def export_frozen(root, out, read, save, public_blob, data):
    """Exact sources and patches, explicitly redacted release/script context."""
    folders = ('watched-manifest-bytes-v76', 'watched-manifest-suite-v76')
    for folder in folders:
        for path in sorted((root / folder).rglob('*')):
            if not path.is_file():
                continue
            relative = path.relative_to(root)
            raw = read(path)
            if path.suffix in ('.rs', '.patch') or (path.suffix == '.py' and path.name != 'sourcechecks.py'):
                assert b'/home/jeremy' not in raw, 'exact source/patch contains private path'
                exported = raw
            else:
                exported = raw.replace(str(root).encode(), b'<EXPERIMENT>').replace(b'/home/jeremy', b'<PRIVATE_HOME>')
            save('frozen/' + str(relative), exported)
    final = root / 'watched-manifest-bytes-v76/finaldoc-regression/candidates/docs/resident-watched-manifest-bytes.md'
    accepted = public_blob(COMMIT, 'docs/resident-watched-manifest-bytes.md')
    save('frozen/root-doc/accepted.md', accepted)
    patch = ''.join(difflib.unified_diff(read(final).decode().splitlines(True), accepted.decode().splitlines(True),
                                       fromfile='a/docs/resident-watched-manifest-bytes.md',
                                       tofile='b/docs/resident-watched-manifest-bytes.md'))
    save('frozen/root-doc/lineage-clarification.patch', patch.encode())
    data['source_freeze'] = {'baseline': 'de8533b93', 'accepted': COMMIT,
                             'rust_sha256': sha(read(root / 'watched-manifest-bytes-v76/candidates/toolchain/resident/crates/ra-jit/src/inputs.rs')),
                             'original_rust_sha256': sha(read(root / 'watched-manifest-bytes-v76/originals/toolchain/resident/crates/ra-jit/src/inputs.rs')),
                             'doc_join': 'initial -> finaldoc -> finaldoc-regression -> root lineage clarification'}
    rust = 'toolchain/resident/crates/ra-jit/src/inputs.rs'
    assert public_blob('de8533b93', rust) == read(root / 'watched-manifest-bytes-v76/originals' / rust)
    assert public_blob(COMMIT, rust) == read(root / 'watched-manifest-bytes-v76/candidates' / rust)
    for filename in ('resident_suite.py', 'test_resident_manifest_retouch.py'):
        assert public_blob(COMMIT, 'bench/' + filename) == read(root / 'watched-manifest-suite-v76/candidates' / filename)


def export_aborted(folder, read, load, save, data):
    """Retain incomplete evidence without manufacturing completed receipts."""
    meta = load(folder / 'metadata.json')
    rows = load(folder / 'records.json')
    cleanup = load(folder / 'timeout-cleanup.json')
    assert meta['status'] == 'incomplete' and len(rows) == 52
    assert 'status' not in rows[-1] and not (folder / 'off.json').exists() and not (folder / 'on.json').exists()
    exported = {'accepted': False, 'status': 'INCOMPLETE', 'reason': 'outer task 973809gnbi timeout 124 at 600 seconds; not native correctness failure',
                'outer_timeout_seconds': 600, 'outer_exit': 124, 'schema': meta['schema'], 'mode': meta['mode'],
                'factor': meta['factor'], 'source_before': meta['source_before'], 'fixture_source': meta['fixture_source'],
                'rows': [], 'stops_before_timeout': [r['group'] for r in meta['stops']],
                'cleanup': {'source63_restored': cleanup['source63_restored'], 'stores_deleted': cleanup['stores_deleted'],
                            'stop': {k: cleanup['stop'][k] for k in ('status', 'error', 'timed_out', 'elapsed_seconds')}}}
    for index, row in enumerate(rows):
        item = {k: row[k] for k in ('group', 'phase', 'replicate', 'flag', 'flags', 'nonce', 'touches', 'source_before',
                                          'status', 'error', 'timed_out', 'elapsed_seconds', 'source_after') if k in row}
        item['raw_original'] = {}
        for suffix in ('stdout', 'stderr'):
            path = folder / f'call-{index:03}.{suffix}'
            if path.exists():
                raw = read(path)
                item['raw_original'][suffix] = {'sha256': sha(raw), 'bytes': len(raw)}
        exported['rows'].append(item)
    for name in ('source.original', 'source.fixture', 'Cargo.toml.original', 'Cargo.lock.original'):
        save('aborted/' + name, read(folder / name))
    for suffix in ('stdout', 'stderr'):
        read(folder / ('timeout-segment-3-stop.' + suffix))
    data['aborted'] = exported


def export_script_manifest(root, read, load, save, row_export, process, data):
    folder=root/'manifest-script-retouch-v77-actual'
    summary=load(folder/'summary.json')
    result={k:summary[k] for k in ('accepted','results','engine_sha256','caches_retained','interpretation')}
    result['attribution']='root task979857g6da, supplemental correctness only, no new timing estimator'
    result['arms']={}
    def snap(obj):
        return {k:obj[k] for k in ('sha256','size','mtime_ns','dev','ino','mode') if k in obj}
    for arm in ('off','on'):
        state=load(folder/arm/'state.json')
        case={k:state[k] for k in ('arm','status','source_restored','manifest_mtime_restored','stop_accepted',
                                  'identities_unchanged','caches_retained','clock_setup','seconds','oracle_limitation')}
        case['source_before']={k:snap(v) for k,v in state['source_before'].items()}
        case['source_after']={k:snap(v) for k,v in state['source_after'].items()}
        for when in ('before','after'):
            case['engine_'+when]=snap(state['engine_'+when])
            case['toolchain_'+when]=sha(canonical(state['toolchain_'+when]['files']))
            case['native_tools_'+when]={k:snap(v) for k,v in state['native_tools_'+when].items()}
        flags=('RA_JIT_MANIFEST_CONTENT_GENERATION','RA_JIT_WATCHED_MANIFEST_BYTES')
        case['flags']={k:state['environment'][k] for k in flags}
        case['stop']=process(state['stop'])
        case['rows']=[]
        for row in state['rows']:
            index=row['index']; stem=folder/arm/f'{index}-run'
            native=dict(row['process'],nonce=row['nonce'],raw=row['raw'],raw_error=row['raw_error'],accepted=row['accepted'])
            item=row_export(native,stem,False)
            for key in ('index','mtime_seconds','expected','clock_before_ns','clock_launch_ns','clock_after_ns',
                        'expected_marker','marker_lines','source_bytes_unchanged','script_invoked_for_expected_mtime'):
                item[key]=row[key]
            item['manifest_before']=snap(row['manifest_before']); item['manifest_after']=snap(row['manifest_after'])
            item['source_before']={k:snap(v) for k,v in row['source_before'].items()}
            item['flags']={k:row['actual_launch']['environment'][k] for k in flags}
            item['script_proof_before']=base64.b64decode(row['script_proof_before_base64'],validate=True).decode('ascii')
            item['script_proof_after']=base64.b64decode(row['script_proof_after_base64'],validate=True).decode('ascii')
            item['status_receipt']=process(row['status_receipt'])
            for suffix in ('stdout','stderr'): read(folder/arm/f'{index}-status.{suffix}')
            case['rows'].append(item)
        proof=read(folder/arm/'build-script-invocations.log')
        save('script-manifest/'+arm+'/build-script-invocations.log',proof)
        for name in ('Cargo.toml','Cargo.lock','src/main.rs','build.rs'):
            save('script-manifest/'+arm+'/source/'+name,read(folder/arm/'source'/name))
        for suffix in ('stdout','stderr'): read(folder/arm/f'stop.{suffix}')
        result['arms'][arm]=case
    for path in sorted((root/'manifest-script-retouch-v77-future').rglob('*')):
        if path.is_file():
            raw=read(path)
            raw=raw.replace(str(root).encode(),b'<EXPERIMENT>').replace(b'/home/jeremy',b'<PRIVATE_HOME>')
            save('frozen/'+str(path.relative_to(root)),raw)
    raw=read(root/'probe-manifest-script-retouch-v77.py')
    assert sha(raw)=='1b833741cfeb059e4e47a1e84e73caf596083e6a2c4d37623c17be7435a1aa6f'
    result['driver_original_sha256']=sha(raw)
    result['driver_export']='known authoring-path replacement only; original SHA/size preserved, never imported'
    save('drivers/probe-manifest-script-retouch-v77.py.txt',raw.replace(str(root).encode(),b'<EXPERIMENT>').replace(b'/home/jeremy',b'<PRIVATE_HOME>'))
    data['script_manifest']=result


def collect(root, pinned, repo, out):
    out.mkdir(parents=True, exist_ok=True)
    provenance, originals = {}, {}

    def read(path):
        path = Path(path)
        raw = path.read_bytes()
        label = ('pinned/' + str(path.relative_to(pinned)) if path.is_relative_to(pinned)
                 else 'aborted/' + str(path.relative_to(pinned.parent / 'wm76a')) if path.is_relative_to(pinned.parent / 'wm76a') else 'experiment/' + str(path.relative_to(root)))
        provenance[label] = {'sha256': sha(raw), 'bytes': len(raw)}
        originals[path] = sha(raw)
        return raw

    def load(path):
        return json.loads(read(path))

    def save(name, raw):
        p = out / name
        p.parent.mkdir(parents=True, exist_ok=True)
        p.write_bytes(raw)

    def jsave(name, value):
        save(name, (json.dumps(value, sort_keys=True, indent=2) + '\n').encode())

    def public_blob(rev, path):
        raw = subprocess.check_output(['git', '-C', str(repo), 'show', rev + ':' + path])
        provenance['git/' + rev + '/' + path] = {'sha256': sha(raw), 'bytes': len(raw)}
        return raw

    def text(raw):
        # Only selected text sources/scripts, never an environment or proc dump.
        return raw.decode().replace(str(root), '<EXPERIMENT>').replace(str(pinned), '<PINNED_RUN>').replace('/home/jeremy', '<PRIVATE_HOME>')

    def process(p):
        return {k: p[k] for k in ('status', 'error', 'timed_out', 'elapsed_seconds')}

    def selected_stdout(raw):
        value = raw.decode()
        patterns = (r' running [0-9]+ tests', r'running 4 tests', r'\s*', r'ok', r'FAILED',
                    r'failures:', r'    summary::tests::shares_overlapping_time',
                    r'test summary::tests::[a-z_]+ \.\.\. (?:ok|FAILED|JR_(?:MATRIX_RUNTIME|MANIFEST_RETOUCH)_[0-9a-f]{32})',
                    r'test result: (?:ok|FAILED)\. [0-4] passed; [01] failed; 0 ignored; 0 measured; 144 filtered out; finished in [0-9.]+s',
                    r'(?:CONTENT_GENERATION|GEN_OK)_[0-9a-f]{32}', r'(?:NATIVE|MANIFEST_NATIVE)_[0-9a-f]{32}_[0-9]+',
                    r'blaze 0\.20261027\.0 \([0-9a-f]{40}\)',
                    r'jr-engine experimental 0\.20261027\.0 \(Blaze-derived origin [0-9a-f]{40}; build resident-v1-[0-9a-f]{64}\)')
        assert all(any(re.fullmatch(p, line) for p in patterns) for line in value.splitlines()), 'unrecognized stdout grammar'
        return value

    def diagnostics(raw):
        # Exact full-line selectors, no arbitrary private diagnostic context.
        patterns = (r"error\[E0308\]: expected u(?:32|64), found &'static str",
                    r'\s*\d+ \| let _: u64 = "JR_(?:MATRIX_TYPE_[0-9a-f]{32}|RUNTIME_TYPE_ERROR)";',
                    r'\s*\d+ \| #\[cfg\(feature="bad"\)\] fn unused_bad\(\)->u32\{"JR_GENERATION_ERROR_[0-9a-f]{32}"\}',
                    r'assertion `left == right` failed: JR_(?:MATRIX_FAILURE_[0-9a-f]{32}|RUNTIME_FAILURE)')
        result = [line for line in raw.decode().splitlines() if any(re.fullmatch(p, line) for p in patterns)]
        assert all('/home/' not in line and 'Authorization' not in line for line in result)
        return result

    def raw_receipt(stem, row=None, worker=False):
        rawout, rawerr = read(str(stem) + '.stdout'), read(str(stem) + '.stderr')
        if row and 'raw' in row:
            for k, raw in [('stdout', rawout), ('stderr', rawerr)]:
                assert row['raw'][k]['sha256'] == sha(raw) and row['raw'][k]['bytes'] == len(raw)
        result = {'stdout': selected_stdout(rawout), 'diagnostic_lines': diagnostics(rawerr),
                  'raw_original': {k: {'sha256': sha(v), 'bytes': len(v)} for k,v in [('stdout',rawout),('stderr',rawerr)]}, 'entry_marker_count': rawerr.decode().splitlines().count(MARKER)}
        if worker:
            rawworker = read(str(stem) + '-worker-0.log')
            result['marker_lines'] = [line for line in rawworker.decode().splitlines() if line == MARKER]
            assert rawworker.count(MARKER.encode()) == len(result['marker_lines'])
        if Path(str(stem) + '-status.stdout').exists():
            status = read(str(stem) + '-status.stdout').decode()
            read(str(stem) + '-status.stderr')
            result['status_worker_lines'] = ['worker: pid ' + n for n in re.findall(r'worker: pid (\d+)', status)]
        return result

    def row_export(row, stem, worker=True):
        result = {k: row[k] for k in FIELDS if k in row}
        command = row['command']
        assert Path(command[0]).is_absolute() and all('/home/' not in arg for arg in command[1:])
        result['command_arguments'] = command[1:]
        result.update(raw_receipt(stem,row,worker))
        result['safe_environment'] = {k:row['environment'][k] for k in ENV}
        assert result['safe_environment'] == ENV
        result['omitted_environment_entries'] = len(row['environment']) - len(ENV)
        result['noise_contamination'] = row['noise']['contamination']
        for key in ('status_receipt','worker_status'):
            if key in row: result[key] = process(row[key])
        return result

    def state_export(folder):
        state = load(root / folder / 'state.json')
        result = {k:state[k] for k in ('status','unchanged','source_unchanged_except_lock','source_before','source_after','tool_before','tool_after') if k in state}
        if 'startup_controls' in state:
            assert set(state['startup_controls']) <= set(CONTROLS)
            result['controls'] = state['startup_controls']
        for key in ('toolchain_before','toolchain_after'):
            if key in state: result[key] = sha(canonical(state[key]['files']))
        result['rows'] = []
        for row in state['rows']:
            original = Path(row['raw']['stdout']['path'])
            assert original.parent == root / folder
            result['rows'].append(row_export(row,original.with_suffix('')))
        if 'stop' in state: result['stop'] = process(state['stop'])
        return result

    data = {'schema':'resident-watched-manifest-evidence/v1', 'commit':COMMIT,
            'decision':'accepted-default-off-scoped-combined-retouch', 'default_promotion':False,
            'general_speed_claim':False, 'causal_claim':False, 'independent_manifest_lock_claim':False,
            'noise':'unknown', 'native':{}, 'strict':{}, 'native_aba':{}}
    meta = load(pinned / 'metadata.json')
    assert meta['tools_before'] == meta['tools_after'] and meta['toolchain_before'] == meta['toolchain_after']
    assert meta['source_before'] == meta['source_after'] and len(meta['source_before']) == 63
    for name,inv in [('source63',meta['source_before']),('toolchain4020',meta['toolchain_before']['files'])]: jsave('inventories/'+name+'.json',inv)
    for path,digest in meta['source_before'].items():
        raw = public_blob('f97fcc2',path)
        assert sha(raw) == digest
        save('pinned-source/'+path,raw)
    for file in ('source.original','source.fixture','Cargo.toml.original','Cargo.lock.original'):
        save('pinned/'+file,read(pinned/file))
    data['pinned'] = {k:meta[k] for k in ('schema','mode','factor','pin','order','samples','source_before','source_after','fixture_source','status','source_restored','toolchain_links_valid','profiles','scope','store_scope')}
    data['pinned']['tools'] = {k:v['sha256'] for k,v in meta['tools_before'].items()}
    data['pinned']['toolchain'] = sha(canonical(meta['toolchain_before']['files']))
    data['pinned']['controls'] = {g:{k:v for k,v in env.items() if k in CONTROLS or k in ('BLAZE_REMOTE','RA_JIT_REMOTE','BLAZE_PROC_MACRO_SERVER','RA_JIT_PROC_MACRO_SERVER')} for g,env in meta['environments'].items()}
    data['pinned']['private_states'] = {}
    for key in ('RA_JIT_HOME','CARGO_HOME','CARGO_TARGET_DIR','RA_JIT_CACHE','BLAZE_CACHE','RUSTUP_HOME'):
        values = [env[key] for env in meta['environments'].values()]
        assert len(set(values)) == 5 and all(Path(v).is_relative_to(pinned) for v in values)
        data['pinned']['private_states'][key] = {g:'<PRIVATE_'+g+'_'+key+'>' for g in meta['environments']}
    data['pinned']['host'] = {k:meta['host_before'][k] for k in ('machine','cpu','ncpu') if k in meta['host_before']}
    assert meta['host_before'] == meta['host_after']
    data['pinned']['stops'] = [{'group':s['group'],'process':process(s['process'])} for s in meta['stops']]
    for stop in meta['stops']:
        for suffix in ('.stdout','.stderr'): read(pinned / (stop['group']+'-stop'+suffix))
    records = load(pinned/'records.json')
    data['pinned']['records'] = []
    for index,row in enumerate(records):
        item = row_export(row,pinned/f'call-{index:03}',False)
        item.update(source_before=row['source_before'],source_after=row['source_after'])
        if 'worker' in row:
            w = row['worker']; argv = bytes.fromhex(w['cmdline_hex']).decode().split('\0'); assert argv[-1] == ''; argv.pop()
            assert argv[0] == meta['tools_before']['engine']['path'] and w['exe'] == argv[0]
            # Only known command roles are exported, never raw proc argv/path dumps.
            normalized = ['<ENGINE>','<SOURCE>/']
            assert argv[1].endswith('/source63/')
            args = iter(argv[2:])
            for arg in args:
                if arg in ('--own-target','--cfg-test','--dev-deps'): normalized.append(arg)
                elif arg in ('--profile','--asked-in','--crate','--out','--patch','--exe','--socket','--symbols-in'):
                    value = next(args); normalized.append(arg)
                    if arg == '--profile': assert value == 'dev'; normalized.append(value)
                    elif arg == '--crate': assert value == 'src/lib.rs'; normalized.append(value)
                    elif arg == '--asked-in': assert value.endswith('/source63'); normalized.append('<SOURCE>')
                    else:
                        home = row['environment']['RA_JIT_HOME']; target = row['environment']['CARGO_TARGET_DIR']
                        if arg in ('--socket','--symbols-in'):
                            rel = str(Path(value).relative_to(home)); assert re.fullmatch(r'workers/[0-9a-f]+/(?:sock|symbols)',rel)
                            normalized.append('<PRIVATE_'+row['group']+'_HOME>/'+rel)
                        else: normalized.append('<PRIVATE_'+row['group']+'_TARGET>/'+str(Path(value).relative_to(target)))
                else: raise AssertionError('unknown worker argument')
            item['worker'] = {'pid':w['pid'],'stat':w['stat'],'exe':'<ENGINE>','exe_sha256':w['exe_sha256'],'argv':normalized}
        data['pinned']['records'].append(item)
    for arm in ('off','on'):
        result=load(pinned/(arm+'.json'))
        assert result['metadata'] == meta
        data['pinned'][arm] = {k:result[k] for k in ('schema','arm','measurements','noise','scope')}
    data['pinned']['comparison_provenance'] = 'independently recomputed from original records; not a preserved console transcript'
    data['pinned']['root_compare_witness'] = {
        'attribution': 'coordinator skunk posthoc message, not a raw transcript file',
        'tool_time_utc': '2026-10-31T15:19:17.896Z', 'exit': 0,
        'reported_off_seconds': '2.457933', 'reported_on_seconds': '0.054515',
        'reported_delta_percent': '-97.78'}

    for case in ('off','on','on-repeat','off-repeat'):
        folder='watched-manifest-v76-'+case+'-seven'
        data['strict'][case]=state_export(folder)
        for i in range(3): save(f'strict/{case}/original-{i}',read(root/folder/f'original-{i}'))
    for case in ('env','file'):
        folder='watched-manifest-v76-native-'+case+'-aba'
        data['native_aba'][case]=state_export(folder)
        for file in ('Cargo.toml','Cargo.lock','src/main.rs','build.rs','a.c','b.c'): save('native-aba/'+case+'/'+file,read(root/folder/'source'/file))
    cfg=load(root/'watched-manifest-v76-cfg-nine/state.json')
    data['changed_manifest']={k:cfg[k] for k in ('status','unchanged')}
    assert cfg['tools']==cfg['tools_after'] and cfg['toolchain']==cfg['toolchain_after']
    data['changed_manifest']['tools']=cfg['tools']
    data['changed_manifest']['toolchain']=sha(canonical(cfg['toolchain']['files']))
    data['changed_manifest']['rows']=[]
    for row in cfg['rows']:
        stem=root/'watched-manifest-v76-cfg-nine'/f"{row['mode']}-{row['phase']}"
        data['changed_manifest']['rows'].append(row_export(row,stem))
    for mode in ('original','off','on'):
        for file in ('Cargo.toml','Cargo.lock','src/main.rs'): save('changed-manifest/'+mode+'/'+file,read(root/'watched-manifest-v76-cfg-nine'/mode/file))
    data['changed_manifest']['stops']=[process(p) for p in cfg['stops']]

    for gate,folder in [('check','candidate-build-v76'),('compile','candidate-build-v76'),('codegen-native','candidate-unit-v76'),('full-native','candidate-unit-v76')]:
        log=read(root/folder/(gate+'.log')).decode()
        data['native'][gate]={'exit':int(read(root/folder/(gate+'.exit'))),'summaries':[line for line in log.splitlines() if line.startswith('justrust: ok:') or line.startswith('  tests   ')]}
    for name,run in [('codegen','20261031-075838161-2334781'),('resident','20261031-075838665-2334948')]:
        raw=read(root/'candidate-unit-v76/justrust-home/runs'/run/'output.jsonl')
        lines=[json.loads(line)['l'] for line in raw.splitlines()]
        selected=[line for line in lines if re.fullmatch(r'test [A-Za-z0-9_:]+ \.\.\. (?:ok|FAILED) <[0-9.]+s>',line) or re.fullmatch(r'test result: ok\. \d+ passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in [0-9.]+s',line)]
        jsave('native/'+name+'-results.json',selected)
        data['native'][name+'_run_id']=run
    before=read(root/'candidate-build-v76/all-source-before.sha256').decode().splitlines()
    after=read(root/'candidate-unit-v76/all-source-after.log').decode().splitlines()
    inv={line[66:]:line[:64] for line in before}; jsave('inventories/native-source3757.json',inv)
    assert len(before)==len(after)==len(inv)==3757 and after==[name+': OK' for name in inv]
    save('native/all-source-before.sha256', read(root/'candidate-build-v76/all-source-before.sha256'))
    save('native/all-source-after.log', read(root/'candidate-unit-v76/all-source-after.log'))
    save('native/lock-before.sha256', read(root/'candidate-build-v76/lock-before.sha256'))
    save('native/lock-after.log', read(root/'candidate-unit-v76/lock-after.log'))
    data['native'].update(source_count=len(inv),source_check_ok=len(before)==len(after) and all(line.endswith(': OK') for line in after),source_inventory_identity=sha(canonical(inv)),lock_check_ok=all(line.endswith(b': OK') for line in read(root/'candidate-unit-v76/lock-after.log').splitlines()),lock_sha256=read(root/'candidate-build-v76/lock-before.sha256').decode().split()[0],engine_sha256=read(root/'candidate-build-v76/engine-identity.sha256').decode().split()[0],engine_version=read(root/'candidate-build-v76/engine-version.txt').decode().strip())
    regression=read(root/'v76-full-harness-regression.log').decode()
    data['python_regression']={'tests':761,'summary':re.findall(r'Ran 761 tests in [0-9.]+s\n\nOK',regression)}
    save('regression/full761.log',text(regression.encode()).encode())
    save('native/root-Cargo.lock',public_blob(COMMIT,'toolchain/resident/Cargo.lock'))
    export_frozen(root, out, read, save, public_blob, data)
    for name in ('resident_state_matrix.py','resident_engine_compare.py','resident_workload_compare.py','blaze_runtime_compare.py','blaze_edit_compare.py','resident_suite.py','test_resident_manifest_retouch.py'):
        save('oracles/'+name+'.txt',public_blob(COMMIT,'bench/'+name))
    for name in ('LICENSE-APACHE','LICENSE-MIT','README.md','Cargo.toml'):
        save('lineage/'+name,public_blob(COMMIT,'toolchain/resident/'+name))
    for name in ('build-test-candidate-v76.sh','probe-manifest-cfg-reload-watched-v76.py','probe-native-link-reload-watched-v76.py','probe-native-file-reload-watched-v76.py','probe-watched-manifest-seven-v76.py','run-pinned-watched-manifest-v76.sh','cleanup-completed-watched-manifest-v76.py'):
        save('drivers/'+name+'.txt',text(read(root/name)).encode())
    cleanup=load(root/'completed-watched-manifest-v76-stop/receipt.json')
    data['cleanup']={'stores_deleted':cleanup['stores_deleted'],'rows':[{k:r[k] for k in ('post_measurement_only','status','error','timed_out')} for r in cleanup['rows']]}
    export_aborted(pinned.parent/'wm76a', read, load, save, data)
    export_script_manifest(root,read,load,save,row_export,process,data)
    data['privacy']={'method':'fixed fields and full-line output allowlists; known command roles, not raw argv/environment dumps','environment_allowlist':ENV,'excluded':['ambient env','private paths','binaries','raw proc argv','raw daemon logs','credentials']}
    data['private_originals']=provenance
    for p,digest in originals.items(): assert sha(p.read_bytes())==digest,'private original changed'
    data['artifacts'] = {str(p.relative_to(out)):sha(p.read_bytes()) for p in out.rglob('*') if p.is_file() and str(p.relative_to(out)).split('/')[0] in ('pinned-source','inventories','pinned','strict','native-aba','changed-manifest','native','frozen','oracles','lineage','drivers','analysis','aborted','regression','script-manifest')}
    jsave('observations.json',data)
    return data


if __name__ == '__main__':
    parser=argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--root',type=Path)
    parser.add_argument('--pinned',type=Path)
    parser.add_argument('--repo',type=Path)
    parser.add_argument('--manifest-only',action='store_true')
    args=parser.parse_args(); output=Path(__file__).resolve().parent
    if not args.manifest_only: collect(args.root,args.pinned,args.repo,output)
    seal(output)
