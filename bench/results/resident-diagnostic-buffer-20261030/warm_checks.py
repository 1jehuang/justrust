"""Independent offline replay of separate 42-triple warm-store observation."""
from decimal import Decimal, localcontext
import itertools
import json
import math
import re
import statistics
from raw_checks import need, sha, canonical, libtest, edit_bytes, NAMES, PROFILES, REFERENCE_HASH


def replay(e, engine_hashes):
    prefix = 'observations/interleaved-v20/'
    state = e.obj(prefix+'state.json')
    need(state['status'] == 'passed' and all(state[k] is True for k in ['source_restored','identities_unchanged','toolchain_unchanged']), 'warm terminal/source/identity gate')
    need(state['scope'] == 'extended interleaved warm-store diagnostic, not cold support, comprehensive gate, significance or default promotion', 'warm scope overclaim')
    need(state['sampling'] == '42 identical-byte edits across all six stack permutations, no sample exclusion', 'warm sampling changed')
    need(state['source_before'] == state['source_after'] and len(state['source_before']) == 63 and canonical(state['source_before']) == '79c53ff6e87a22c84c9bc7bac6917c69b3f8d0fbc29b107f734cc00cc39a64df', 'warm source63 identity')
    need(state['toolchain_before'] == state['toolchain_after'] and canonical(state['toolchain_before']) == '8bdbccdd1ca3fd7f2935cfb5667047eb4c46824928823e96311d8eb5ec0a6371', 'warm toolchain identity')
    original = e.read(prefix+'original.rs')
    need(sha(original) == state['source_before']['src/summary.rs'], 'warm original source')
    configs = state['configs']; need(set(configs) == {'old','new','blaze'}, 'warm stacks changed')
    for stack, case, kind, digest in [('old','rsb11','engine',engine_hashes[4]), ('new','rsa17','engine',engine_hashes[20]), ('blaze','rsa17','reference',REFERENCE_HASH)]:
        cfg = configs[stack]; meta = e.obj(f'sessions/{case}/metadata.json')
        need(cfg['sha256'] == digest == meta['tools_before'][kind]['sha256'] and cfg['tool'] == meta['tools_before'][kind]['path'] and cfg['kind'] == kind, 'warm executable identity')
        expected_env = {k:value for k,value in meta['environments'][kind].items() if k not in PROFILES and k not in {'BLAZE_REMOTE','RA_JIT_REMOTE','BLAZE_PROC_MACRO_SERVER','RA_JIT_PROC_MACRO_SERVER','JR_ENGINE_HOME'}}
        expected_env.update({f'XDG_{key}':f'/home/jeremy/{case}/{kind}/{suffix}' for key,suffix in [('CACHE_HOME','cache'),('CONFIG_HOME','config'),('DATA_HOME','data'),('RUNTIME_DIR','runtime')]})
        need(cfg['environment'] == expected_env, 'warm private namespace')
        need(cfg['expected'] == meta['expected_tests'], 'warm test expectation')
    order = [(s,p,0) for s in ['old','new','blaze'] for p in ['prepare','type-error','type-recovery','assert-failure','assert-recovery']]
    permutations = list(itertools.permutations(['old','new','blaze']))
    order += [(s,'edit',rep) for rep in range(42) for s in permutations[rep%6]]
    rows = state['rows']
    need(len(rows) == 141 and [(r['stack'],r['phase'],r['replicate']) for r in rows] == order, 'warm 141 rows/permutation ordering')
    for row in rows:
        stack,phase,rep = row['stack'],row['phase'],row['replicate']; cfg = configs[stack]
        need(row['accepted'] is True and row['error'] is None and row['timed_out'] is False, 'warm process admission')
        need(math.isfinite(row['elapsed_seconds']) and row['elapsed_seconds'] > 0 and row['noise']['contamination'] == 'unknown', 'warm timing/noise guarantee')
        env = dict(cfg['environment'], **PROFILES)
        need(row['environment'] == env, 'warm row environment')
        command = [cfg['tool']] + (['cargo'] if stack == 'blaze' else []) + ['test','--lib','--','summary::tests::','--test-threads=1','--nocapture']
        need(row['command'] == command, 'warm row command')
        edited = edit_bytes(original,phase,rep+800 if phase=='edit' else 0) if phase in ['edit','type-error','assert-failure'] else original
        need(sha(edited) == row['edit_sha256'], 'warm edit bytes')
        streams = []
        for stream in ['stdout','stderr']:
            contents = e.read(prefix+f'{phase}-{rep}-{stack}.{stream}')
            need(sha(contents) == row[stream+'_sha256'], 'warm raw stream hash')
            streams.append(contents.decode())
        text = '\n'.join(streams)
        if phase == 'type-error':
            need(row['status'] == 101 and 'E0308' in text and 'JR_RUNTIME_TYPE_ERROR' in text and row['parsed'] is None, 'warm type-error gate')
        else:
            names,counts = libtest(text)
            parsed = row['parsed']
            need(parsed['tests'] == [list(x) for x in names] and (parsed['passed'],parsed['failed'],parsed['ignored'],0,parsed['filtered']) == counts, 'warm parsed raw mismatch')
            if phase == 'assert-failure':
                need(row['status'] == 101 and counts == (3,1,0,0,144) and set(n for n,_ in names) == set(NAMES) and 'JR_RUNTIME_FAILURE' in text, 'warm assertion gate')
            else:
                need(row['status'] == 0 and names == [(n,'ok') for n in NAMES] and counts == (4,0,0,0,144), 'warm positive gate')
                markers = re.findall(r'JR_RUNTIME_EXECUTED_[^\s]*',text)
                need(markers == ([f'JR_RUNTIME_EXECUTED_{rep+801}'] if phase=='edit' else []), 'warm runtime marker')
    medians = {s:statistics.median(r['elapsed_seconds'] for r in rows if r['stack']==s and r['phase']=='edit') for s in configs}
    need(medians == state['medians_seconds'], 'warm raw median mismatch')
    exact_rows = json.loads(e.text(prefix+'state.json'),parse_float=Decimal)['rows']
    exact = {s:statistics.median(r['elapsed_seconds'] for r in exact_rows if r['stack']==s and r['phase']=='edit') for s in configs}
    with localcontext() as context:
        context.prec = 50
        delta = (exact['new']/exact['old']-1)*100
        need(3 < delta < 4, 'warm no-gain observation contradicted')
        return {'rows':141,'timed_triples':42,'permutation_repetitions':7,'medians_seconds':medians,
                'exact_medians':{s:str(x) for s,x in exact.items()},'exact_new_vs_old_percent':str(delta),
                'noise':'unknown','promotion':False,'scope':state['scope']}
