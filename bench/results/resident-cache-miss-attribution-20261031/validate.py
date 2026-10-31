#!/usr/bin/env python3
"""Portable offline semantic replay of actual native evidence. Never executes native code."""
import argparse
import hashlib
import importlib.util
import json
import math
from pathlib import Path
import re
import sys

sys.dont_write_bytecode = True
HERE = Path(__file__).resolve().parent
COMMITMENTS = '6661f5571b365b7ad839fb3a236ba46b948c80cfdb590a9fdb48447bfbdaec0e'
PHASES = ['cold-prepare','repeated-noop-1','manifest-feature-add','type-error','type-recovery','assert-failure','assert-recovery']
LABELS = 'key_unavailable direct_key_missing banned decode_or_sound debug_types path_remap arrived get_positive get_negative settled_positive settled_negative direct_unavailable native_imports dependency_settled_positive dependency_settled_negative dependency_unavailable dependency_native_imports transitive_failure usable_positive'.split()
FIXTURES = set('cache_miss_stats_gate_is_exact native_miss_arrival_reasons_follow_real_decode_and_remap native_cache_stats_cover_debug_imports_bans_and_positive_scc native_prefetch_missing_and_dependency_failures_are_distinct native_usable_negative_memo_is_not_a_fresh_missing_key'.split())
ENV_ALLOW = set('HOME CARGO_HOME CARGO_TARGET_DIR RUSTUP_HOME BLAZE_HOME BLAZE_CACHE BLAZE_BACKEND BLAZE_OPT RA_JIT_HOME RA_JIT_CACHE XDG_CACHE_HOME XDG_CONFIG_HOME XDG_DATA_HOME XDG_RUNTIME_DIR'.split()) | {f'CARGO_PROFILE_{p}_{k}' for p in ('DEV','TEST') for k in ('DEBUG','INCREMENTAL','OPT_LEVEL')}


def require(ok, message):
    if not ok:
        raise ValueError(message)


def sha(data):
    return hashlib.sha256(data).hexdigest()


def load(path):
    return json.loads(path.read_text())


def canonical(value):
    return json.dumps(value,sort_keys=True,separators=(',',':')).encode()


def files(root):
    result = {}
    for p in sorted(root.rglob('*')):
        require(not p.is_symlink(), 'symlink in evidence')
        if p.is_file() and p not in (root/'SHA256SUMS',root/'validation.json') and '__pycache__' not in p.parts:
            require(p.read_bytes()[:4] != b'\x7fELF', 'ELF binary retained')
            result[str(p.relative_to(root))] = sha(p.read_bytes())
    return result


def seal(root):
    (root/'SHA256SUMS').write_text(''.join(f'{digest}  {name}\n' for name,digest in sorted(files(root).items())))


def inventory(root):
    listed = {}
    for line in (root/'SHA256SUMS').read_text().splitlines():
        require(re.fullmatch(r'[0-9a-f]{64}  .+',line), 'bad transport receipt')
        digest,name = line.split('  ',1)
        require(not Path(name).is_absolute() and '..' not in Path(name).parts and name not in listed,'unsafe transport path')
        listed[name] = digest
    require(listed == files(root), 'transport inventory mismatch')


def apply_unified(original, patch):
    lines,out,cursor = original.splitlines(keepends=True),[],0
    parts,at = patch.splitlines(keepends=True),0
    while at < len(parts):
        m = re.match(r'@@ -(\d+)(?:,(\d+))? \+(\d+)(?:,(\d+))? @@',parts[at])
        if not m:
            at += 1
            continue
        start = max(0,int(m[1])-1)
        require(start >= cursor,'overlapping source patch')
        out.extend(lines[cursor:start])
        cursor,old,new = start,0,0
        at += 1
        while at < len(parts) and parts[at][:1] in (' ','+','-') and not parts[at].startswith(('--- ','+++ ')):
            mark,text = parts[at][0],parts[at][1:]
            if mark in (' ','-'):
                require(cursor < len(lines) and lines[cursor] == text,'native patch context')
                cursor += 1
                old += 1
            if mark in (' ','+'):
                out.append(text)
                new += 1
            at += 1
        require(old == int(m[2] or 1) and new == int(m[4] or 1),'patch hunk size')
    out.extend(lines[cursor:])
    return ''.join(out)


def source_audit(root,c):
    for name,digest in c['frozen'].items():
        require(sha((root/name).read_bytes()) == digest,'independent frozen-source commitment')
    sub = 'toolchain/resident/crates/hir-ty-codegen/'
    for version in (42,43):
        base = root/f'frozen/v{version}'
        for line in (base/'SHA256SUMS').read_text().splitlines():
            digest,name = line.split('  ',1)
            require(not Path(name).is_absolute() and '..' not in Path(name).parts,'unsafe historical receipt')
            require(sha((base/name).read_bytes()) == digest,'historical frozen receipt')
        parts = re.split(r'(?=^diff --git )',(base/'candidate.patch').read_text(),flags=re.M)
        for name in ('cached.rs','mod.rs'):
            selected = next(p for p in parts if p.startswith('diff --git a/'+sub+name+' '))
            # The final mod.rs section may be followed by the new document patch.
            selected = selected.split('--- /dev/null')[0]
            original = root/'frozen/v42/original'/sub/name
            require(apply_unified(original.read_text(),selected) == (base/'candidate'/sub/name).read_text(),'full native patch replay')
    original = root/'frozen/v43/original'/sub/'cached.rs'
    repaired = root/'frozen/v43/candidate'/sub/'cached.rs'
    repair = (root/'frozen/v43/repair-from-v42.patch').read_text().split('--- a/docs/')[0]
    require(apply_unified(original.read_text(),repair) == repaired.read_text(),'exact fixture repair replay')
    insertion = '        // Reads see sealed packs, not the open pack that insert appends to.\n        // Publish through the real native boundary before exercising get/usable.\n        entries.store.seal();\n        assert_eq!(entries.store.get(&key).as_deref(), Some(bytes.as_slice()));\n'
    require(repaired.read_text().replace(insertion,'',1) == original.read_text(),'v43 changes only publication fixture precondition')
    require((root/'frozen/v42/candidate'/sub/'mod.rs').read_bytes() == (root/'frozen/v43/candidate'/sub/'mod.rs').read_bytes(),'frozen mod.rs unchanged')
    store = (root/'frozen/store-lib-cb9d8a010.rs').read_text()
    require('pub fn seal(&self)' in store and 'assert_eq!(store.get(&[1; KEY]), None);' in store,'real native sealed-store contract retained')
    for v in (42,43):
        base = root/f'gates/candidate-build-v{v}'
        table = dict((line.split('  ',1)[1],line.split('  ',1)[0]) for line in (base/'all-source-before.sha256').read_text().splitlines())
        require(len(table) == 3757,'3757-source coverage')
        for name in ('cached.rs','mod.rs'):
            require(table['crates/hir-ty-codegen/'+name] == sha((root/f'frozen/v{v}/candidate'/sub/name).read_bytes()),'native build/frozen source join')
        require(table['crates/ra-jit-store/src/lib.rs'] == sha((root/'frozen/store-lib-cb9d8a010.rs').read_bytes()),'frozen native store build join')
    after = (root/'gates/candidate-unit-v43/all-source-after.log').read_text().splitlines()
    require(len(after) == 3757 and all(x.endswith(': OK') for x in after),'3757-source after-check')
    require({x.removesuffix(': OK') for x in after} == set(table),'all after-check paths exactly match native source inputs')
    require((root/'gates/candidate-build-v42/lock-before.sha256').read_bytes() == (root/'gates/candidate-build-v43/lock-before.sha256').read_bytes(),'lock unchanged between candidate identities')
    require((root/'gates/candidate-unit-v43/lock-after.log').read_text().strip() == 'Cargo.lock: OK','lock after-check')
    require((root/'gates/candidate-build-v43/lock-before.sha256').read_text().split()[0] == sha((root/'frozen/Cargo.lock.cb9d8a010').read_bytes()),'immutable native lock bytes join')


def native_audit(root,c):
    observed = []
    for gate in c['gates']:
        run = root/'journals'/gate['run']
        meta,s = load(run/'meta.json'),load(run/'summary.json')
        require(meta['id'] == s['id'] == gate['run'],'native run id')
        require(s['exit'] == gate['exit'] and s['tests'] == gate['tests'],'actual native verdict join')
        require(set(meta.get('env',{})) <= {'JCODE_SESSION_ID','CARGO_TARGET_DIR','JCODE_SCRATCH_DIR'},'native environment allowlist')
        base = root/'gates'/gate['group']
        require(int((base/(gate['receipt']+'.exit')).read_text()) == s['exit'],'native exit receipt')
        slim = load(root/'slim'/(gate['run']+'.json'))
        require(all(slim[k] == s[k] for k in ('id','args','subcommand','start','wall','exit','tests','route')),'slim/raw summary join')
        text = '\n'.join(json.loads(x)['l'] for x in (run/'output.jsonl').read_text().splitlines())
        passed = re.findall(r'^test (\S+) \.\.\. ok(?: <[^>]+>)?$',text,re.M)
        failed = re.findall(r'^test (\S+) \.\.\. FAILED(?: <[^>]+>)?$',text,re.M)
        require(len(passed) == len(set(passed)) == s['tests']['passed'],'actual passing test names')
        require(len(failed) == len(set(failed)) == s['tests']['failed'],'actual failing test names')
        if gate['group'] == 'candidate-unit-v42':
            require(s['exit'] == 101 and len(passed) == 50 and len(failed) == 3,'v42 failed gate retained')
            require({x.rsplit('::',1)[-1] for x in failed} == FIXTURES - {'cache_miss_stats_gate_is_exact','native_miss_arrival_reasons_follow_real_decode_and_remap'},'exact failed v42 fixtures')
            require(text.count('left: 0') == 3 and text.count('right: 1') == 3,'v42 failed counter assertions')
        elif gate['group'] == 'candidate-unit-v43':
            require(s['exit'] == 0 and not failed and len(passed) in (53,360),'v43 full native gates')
            if len(passed) == 53:
                require(FIXTURES <= {x.rsplit('::',1)[-1] for x in passed},'all five executed native fixtures')
        else:
            require(s['exit'] == 0,'actual check/build failure')
        observed.append(dict(run=gate['run'],passed=len(passed),failed=len(failed),exit=s['exit']))
    for v in (42,43):
        base = root/f'gates/candidate-build-v{v}'
        require((base/'engine-identity.sha256').read_text().split()[0] == c['native'][str(v)]['engine_sha'],'engine digest receipt')
        require((base/'engine-version.txt').read_text().strip() == c['native'][str(v)]['version'],'version receipt')
        require(re.search(r'build resident-v1-[0-9a-f]{64}\)$',c['native'][str(v)]['version']),'native build identity version')
    require(re.search(r'Ran 733 tests in [0-9.]+s\s+OK\s*$',(root/'gates/v43-bench-regression.log').read_text()),'actual 733 regression gate')
    return observed


def helpers(root,c):
    for name,digest in c['helpers'].items():
        require(sha((root/name).read_bytes()) == digest,'frozen helper commitment')
    for name in ('blaze_edit_compare','blaze_runtime_compare','resident_engine_compare','resident_workload_compare','resident_state_matrix'):
        spec = importlib.util.spec_from_file_location(name,root/'provenance'/(name+'.py'))
        module = importlib.util.module_from_spec(spec)
        sys.modules[name] = module
        spec.loader.exec_module(module)
    return sys.modules['resident_engine_compare'],sys.modules['resident_state_matrix']


def reports(text):
    result = []
    for line in text.splitlines():
        if 'ra-jit cache-miss-stats v1 ' not in line:
            continue
        require(line.startswith('ra-jit cache-miss-stats v1 ') and len(line) < 1000,'bounded aggregate-only native report')
        fields = line.split('ra-jit cache-miss-stats v1 ',1)[1].split()
        require(len(fields) == len(LABELS),'all nineteen native counters')
        values = {}
        for field in fields:
            m = re.fullmatch(r'([a-z_]+)=([0-9]+)',field)
            require(m is not None and m[1] not in values,'private or duplicate native report field')
            values[m[1]] = int(m[2])
        require(list(values) == LABELS,'exact native counter schema')
        result.append(values)
    return result


def runtime_audit(root,c,shared,matrix):
    pinned = {str(p.relative_to(root/'input/source63')):sha(p.read_bytes()) for p in (root/'input/source63').rglob('*') if p.is_file()}
    require(pinned == c['source_before'] and len(pinned) == 63,'exact pinned source63')
    original_validation = load(root/'root-validation.json')
    require(original_validation['status'] == 'passed','root replay verdict')
    require(original_validation['native'] == [dict(run_id=g['run'],passed=g['tests']['passed']) for g in c['gates'] if g['group'] == 'candidate-unit-v43'],'actual root native summary join')
    result = {}
    for mode in ('on','off'):
        base = root/'runtime'/mode
        state = load(base/'state.json')
        require(state['status'] == 'passed' and state['unchanged'],'state verdict/restoration')
        require(state['source_before'] == state['source_after'] == pinned,'source restoration')
        require(state['tool_before'] == state['tool_after'] == c['native']['43']['engine_sha'],'native engine/runtime join')
        require(state['toolchain_before'] == state['toolchain_after'] and sha(canonical(state['toolchain_before'])) == c['toolchain_sha256'],'toolchain identity/restoration')
        require(len(state['toolchain_before']['files']) == 4020,'4020 toolchain inputs')
        require(state['startup_controls'] == dict(RA_JIT_DEMAND_STD='1',RA_JIT_CACHE_MISS_STATS='1' if mode == 'on' else '0',RA_JIT_STATS='1'),'exact startup opt-in')
        require([r['phase'] for r in state['rows']] == PHASES,'ordered strict seven-state coverage')
        originals = {file:(base/f'original-{i}').read_bytes() for i,file in enumerate(matrix.FILES)}
        require(all(sha(data) == pinned[file] for file,data in originals.items()),'durable original inputs')
        observations,expected = [],None
        for i,row in enumerate(state['rows']):
            phase = row['phase']
            require(row['accepted'] and not row['failures'] and row['input_unchanged'],'accepted and unchanged state')
            require(math.isfinite(row['elapsed_seconds']) and row['elapsed_seconds'] > 0,'positive finite elapsed')
            require(row['noise']['contamination'] == 'unknown','uncontrolled noise claim')
            require(set(row['environment']) <= ENV_ALLOW,'runtime environment allowlist')
            wanted = matrix.inputs(originals,phase,0,row['nonce'])
            recipe = dict(pinned)
            recipe.update({file:sha(data) for file,data in wanted.items()})
            require(row['inputs'] == recipe,'actual invocation-specific input recipe')
            text = ''
            for stream in ('stdout','stderr'):
                data = (base/f'{i:02}-{phase}.{stream}').read_bytes()
                require(row['raw'][stream]['bytes'] == len(data) and row['raw'][stream]['sha256'] == sha(data),'nested raw receipt')
                text += data.decode(errors='replace')+'\n'
            if mode == 'off':
                require(not reports(text),'disabled stdout/stderr zero reports')
            parsed = shared.runtime.test_results(text)
            require(canonical(parsed) == canonical(row['parsed']),'actual raw test parser join')
            if i == 0:
                expected = matrix.contract(parsed,phase)
                require(expected is not None,'actual four-test cold contract')
            require(not matrix.oracle(dict(row,parsed=parsed),text,expected,phase,row['nonce'],0),'strict actual state oracle')
            snapshots = []
            for log in sorted(base.glob(f'{i:02}-*-worker-*.log')):
                snapshots.extend(reports(log.read_text()))
            require(bool(snapshots) == (mode == 'on'),'enabled reports / disabled zero reports')
            observation = dict(phase=phase,status=row['status'],elapsed_seconds=row['elapsed_seconds'],report_count_in_cumulative_log=len(snapshots),last_report=snapshots[-1] if snapshots else None)
            require(observation == original_validation['modes'][mode][i] == c['observations'][mode][i],'whole nineteen-counter root/raw independent observation join')
            observations.append(observation)
        if mode == 'on':
            require(observations[4]['last_report'] == observations[5]['last_report'] == observations[6]['last_report'],'last recovery snapshot repeated, not fresh assertion observations')
            for item in observations:
                require(all(item['last_report'][k] == 0 for k in ('decode_or_sound','debug_types','path_remap','banned')),'observed no rejection in these populations')
        result[mode] = observations
    require(result['on'][2]['last_report']['direct_key_missing'] == 45508 and result['on'][4]['last_report']['direct_key_missing'] == 34863,'actual manifest/recovery observations')
    return result


def validate(root=HERE):
    root = Path(root).resolve()
    inventory(root)
    require(sha((root/'commitments.json').read_bytes()) == COMMITMENTS,'independent historical commitments')
    c = load(root/'commitments.json')
    source_audit(root,c)
    native = native_audit(root,c)
    shared,matrix = helpers(root,c)
    runtime = runtime_audit(root,c,shared,matrix)
    return dict(status='passed',native_gates=native,runtime_rows=14,runtime=runtime,new_native_calls=0,scope=c['scope'],noise='unknown/uncontrolled',fresh_assertion_phase_observation=False,unique_function_loss_inferred=False,speedup_claimed=False)


if __name__ == '__main__':
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--seal',action='store_true',help='Regenerate transport checksums, not authenticate changed observations')
    args = parser.parse_args()
    if args.seal:
        seal(HERE)
    report = validate()
    (HERE/'validation.json').write_text(json.dumps(report,indent=2,sort_keys=True)+'\n')
    print(json.dumps(dict(status=report['status'],native_gates=len(report['native_gates']),runtime_rows=14,new_native_calls=0)))
