#!/usr/bin/env python3
"""Portable, offline root-snapshot evidence replay. Never executes archived commands."""
import argparse
from decimal import Decimal, localcontext
import json
import math
from pathlib import Path
import re
import statistics
import warm_checks

from raw_checks import Evidence, InvalidEvidence, need, sha, libtest, session, NAMES

EXCLUDED = {'collect.py', 'validate.py', 'test_validate.py', 'README.md',
            'manifest.json', 'audit.json', 'offline-tests.log'}
PAIRS = [('v16', 'rsb05', 'rsa11', 15, 16),
         ('v17', 'rsb06', 'rsa12', 16, 17),
         ('v17-cumulative', 'rsb07', 'rsa13', 4, 17),
         ('v18', 'rsb08', 'rsa14', 17, 18),
         ('v19', 'rsb09', 'rsa15', 4, 19),
         ('v19-reverse', 'rsb10', 'rsa16', 4, 19)]
ENGINE_HASHES = {
    4: 'deafad0c76f20059db6e1a07f4ed9665013ee61600db0089aec3667c38069df8',
    15: '0e138c556368ab4f87a5b19b9c8bc0a3b0b4462807711c63f28d80bc7c00718a',
    16: '070ee57c03efa330d76896d3a75f2169a2293fab514d56c8c3e7741d8758f19e',
    17: 'd95e3441b899d14c9fcb081980044fa1a0565ecc09e84bedf708ecac72627851',
    18: '05ee6d282103fb376b4a3e1b4d9d9648bb525945ccf2d3c82d0f00dbc60a6e42',
    19: 'e591a4250e687213f2db3a7c62311071ca1fb664785c41fbd7340c1ac589d84f',
}
MAIN_HASHES = {
    16: '45935441f5747374933bd268cbe791238e435731a333f64e0766b9139aaeeda9',
    17: 'e84f89498d956e702eb618de4dd52713aa70e58aa2b406cc19908f924b0b1686',
    18: 'ec124a9863e76b8b7826e018d7493d5c78170c1eb294654cf2fcabe375d1f3fb',
    19: '206f6545e73f7ae05b013d8c49582fee3806679a3b99ba0e7a1add8a128fc911',
}
CG_HASH = '0a50cf5d1abf3d812a4b942b0935810276ae0f90bfcb2b34b9b991c165e89d16'
SIG_HASH = '724793541da40fe85a8f2fa738e39ae6bae70608c0b43e4b2ece80fe772c31a3'
FILTERS = {'resident_root_snapshot': 4, 'resident_signature_flags': 1,
           'resident_graph_reuse': 1, 'the_roots_of_a_listed_crate': 1, 'telemetry': 3}
PHASES = ('worker roots', 'worker apply', 'worker check', 'client planning', 'client command')


def inventory(e):
    files = e.obj('manifest.json')['sha256']
    actual = {str(p.relative_to(e.root)) for p in e.root.rglob('*')
              if p.is_file() and p.name not in EXCLUDED and '__pycache__' not in p.parts}
    need(set(files) == actual, 'missing/extra manifest artifact')
    for path, digest in files.items():
        need(sha(e.read(path)) == digest, f'artifact checksum: {path}')
    return len(files)


def sums(text):
    pairs = [line.split(None, 1) for line in text.splitlines()]
    need(all(len(x) == 2 and re.fullmatch('[a-f0-9]{64}', x[0]) for x in pairs), 'bad source hashes')
    need(len({x[1] for x in pairs}) == len(pairs), 'duplicate source hash path')
    return {path: digest for digest, path in pairs}


def replay_patch(original, patch):
    """Single-file original unified patch, replayed in memory without external tools."""
    lines = patch.splitlines(keepends=True)
    need(len(lines) > 2 and lines[0].startswith('--- ') and lines[1].startswith('+++ '), 'missing patch headers')
    source, output, cursor, i = original.splitlines(keepends=True), [], 0, 2
    while i < len(lines):
        match = re.fullmatch(r'@@ -(\d+)(?:,(\d+))? \+(\d+)(?:,(\d+))? @@.*\n', lines[i])
        need(match is not None, 'malformed scoped hunk')
        start = int(match[1]) - 1
        need(start >= cursor, 'overlapping scoped hunk')
        output.extend(source[cursor:start]); cursor = start
        old = new = 0; i += 1
        while i < len(lines) and lines[i][:1] in [' ', '+', '-']:
            prefix, text = lines[i][0], lines[i][1:]
            if prefix != '+':
                need(cursor < len(source) and source[cursor] == text, 'scoped patch context mismatch')
                cursor += 1; old += 1
            if prefix != '-':
                output.append(text); new += 1
            i += 1
        need(old == int(match[2] or 1) and new == int(match[4] or 1), 'scoped hunk counts mismatch')
    output.extend(source[cursor:])
    return ''.join(output)


def sources(e):
    for version in [16, 17, 18, 19]:
        root = f'source-ready/v{version}/'
        for filename, digest in [('main.source-ready.rs', MAIN_HASHES[version]),
                                 ('codegen.source-ready.rs', CG_HASH),
                                 ('signatures.source-ready.rs', SIG_HASH)]:
            need(sha(e.read(root + filename)) == digest, f'wrong full source v{version} {filename}')
        for path, digest in sums(e.text(root + 'SHA256SUMS')).items():
            need(sha(e.read(root + Path(path).name)) == digest, 'original ready receipt mismatch')
        parent = 17 if version == 19 else version - 1
        old = 'main.base.rs' if version == 16 else f'main.v{parent}.rs'
        patch = 'main.scoped.patch' if version == 16 else f'main.v{parent}-v{version}.patch'
        need(replay_patch(e.text(root + old), e.text(root + patch)) == e.text(root + 'main.source-ready.rs'),
             'main scoped patch replay mismatch')
        if version > 16:
            need(e.read(root + old) == e.read(f'source-ready/v{parent}/main.source-ready.rs'), 'source lineage fork')
            olddoc = f'doc.v{version-1}.md'
            need(replay_patch(e.text(root + olddoc), e.text(root + f'doc.v{version-1}-v{version}.patch')) ==
                 e.text(root + 'doc.source-ready.md'), 'doc scoped patch replay mismatch')
            prior_doc = 'resident-root-snapshot.source-ready.md' if version == 17 else 'doc.source-ready.md'
            need(e.read(root + olddoc) == e.read(f'source-ready/v{version-1}/' + prior_doc), 'documentation lineage fork')
        if version == 19:
            need(e.read(root + 'main.v18.rs') == e.read('source-ready/v18/main.source-ready.rs'), 'lost rejected v18 bytes')
            need(replay_patch(e.text(root + 'main.v18.rs'), e.text(root + 'main.v18-v19.patch')) == e.text(root + 'main.source-ready.rs'), 'v18-to-v19 restoration patch mismatch')
    # Full hashes pin all providers, original oracle and four fixtures. Also check
    # unchanged tail and tracked code independently across the bounded deltas.
    mains = {v: e.text(f'source-ready/v{v}/main.source-ready.rs').replace('#[cfg(test)]\nfn exported(', 'fn exported(') for v in [16, 17, 18, 19]}
    for marker in ['    fn original(', 'fn root_declarations(', 'mod tests {']:
        tails = [text[text.index(marker):] for text in mains.values()]
        need(all(tail == tails[0] for tail in tails), 'oracle/provider/test tail changed across bounded warmup edits')
    for version in [16, 17, 18, 19]:
        prefix = f'native/candidate-build-v{version}/'
        hashes = sums(e.text(prefix + 'source-before.sha256'))
        need(len(hashes) == 11, 'missing native source identity files')
        for path, digest in hashes.items():
            need(sha(e.read(prefix + 'frozen-source/' + path)) == digest, 'native frozen source mismatch')
        for path, digest in [('crates/ra-jit/src/main.rs', MAIN_HASHES[version]),
                             ('crates/hir-ty-codegen/mod.rs', CG_HASH),
                             ('vendor/rust-analyzer/crates/hir-def/src/signatures.rs', SIG_HASH)]:
            need(hashes[path] == digest, 'ready/native source identity mismatch')
        expected = [path + ': OK' for path in hashes]
        for path in [prefix + 'source-after.log', f'native/candidate-unit-v{version}/source-after.log',
                     f'native/candidate-unit-v{version}/full-native-source-after.log']:
            need(e.text(path).splitlines() == expected, 'native gate source identity changed')


    for version in [4,15]:
        prefix = f'native/candidate-build-v{version}/'
        declared = sums(e.text(prefix + 'source-before.sha256'))
        need(len(declared) == (7 if version == 4 else 11), 'baseline declared source inventory')
        folder = 'hash-matched-source/' if version == 4 else 'frozen-source/'
        for path,digest in declared.items():
            need(sha(e.read(prefix + folder + path)) == digest, 'historical baseline source bytes differ')
        need(e.text(prefix + 'source-after.log').splitlines() == [path + ': OK' for path in declared], 'baseline changed during native gate')
    need(e.read('native/candidate-build-v4/hash-matched-source/crates/ra-jit/src/main.rs') == e.read('baseline-head/toolchain/resident/crates/ra-jit/src/main.rs'), 'v4 baseline main archive mismatch')


def native(e):
    reports = {}; full_names = {}
    inventory_paths = e.obj('manifest.json')['sha256']
    for version in [16, 17, 18, 19]:
        rows = []
        for kind, wanted in [('build', 2), ('unit', 6)]:
            prefix = f'native/candidate-{kind}-v{version}/'
            summaries = sorted(p for p in inventory_paths if p.startswith(prefix + 'justrust-home/runs/') and p.endswith('/summary.json'))
            need(len(summaries) == wanted, 'native run inventory differs')
            seen = set()
            for path in summaries:
                summary = e.obj(path); folder = path.removesuffix('summary.json')
                meta = e.obj(folder + 'meta.json'); journal = e.journal(folder + 'output.jsonl')
                need(journal and all(r['s'] in ['e', 'o'] and math.isfinite(r['t']) for r in journal), 'bad raw native journal')
                text = '\n'.join(r['l'] for r in journal)
                need(summary['id'] == meta['id'] == Path(folder.rstrip('/')).name, 'native ID mismatch')
                args = summary['args']; need(args == meta['args'], 'native args mismatch')
                need(summary['exit'] == 0 and meta['place'] == 'local' and meta['route'] == 'JUSTRUST_REMOTE=off', 'native gate exit/route')
                need(re.fullmatch('[a-f0-9]{12}', summary['git']['head']) is not None, 'missing native git identity')
                need(math.isclose(meta['start'], summary['start'], abs_tol=0.1), 'native timestamps mismatch')
                command = summary['subcommand']
                filt = args[args.index('--') + 1] if '--' in args else None
                expected = [command, '--release', '--locked', '-p', 'ra-jit', '--bin', 'jr-engine']
                if command == 'check': expected += ['--tests']
                expected += ['-j', '6']
                if filt: expected += ['--', filt, '--nocapture']
                need(args == expected, 'native command/filter gate changed')
                if kind == 'build':
                    need(command in ['check', 'build'] and filt is None, 'bad build gate')
                    need(command not in seen, 'duplicate build gate'); seen.add(command)
                else:
                    need(command == 'test' and filt not in seen, 'duplicate/wrong test gate'); seen.add(filt)
                    count = FILTERS[filt] if filt else 351
                    names, counts = libtest(text, native=True)
                    need(counts == (count, 0, 0, 0, 351-count), 'native positive coverage mismatch')
                    tests = summary['tests']
                    need(tests['passed'] == count and tests['failed'] == 0 and tests['filtered_out'] == 351-count, 'native summary differs from raw')
                    if filt:
                        need(all(filt in name for name, _ in names), 'native filter admitted wrong names')
                    else:
                        full_names[version] = [name for name, _ in names]
                rows.append({'id': summary['id'], 'command': command, 'filter': filt,
                             'passed': summary['tests']['passed']})
            need(seen == ({'check', 'build'} if kind == 'build' else set(FILTERS) | {None}), 'native gate missing')
            for filename in (['exit', 'check-exit', 'compile-exit', 'orchestration-exit'] if kind == 'build'
                             else ['exit', 'full-native.exit'] + [f + '.exit' for f in FILTERS]):
                need(e.text(prefix + filename).strip() == '0', 'native exit receipt mismatch')
        reports[str(version)] = rows
    need(all(names == full_names[16] for names in full_names.values()) and len(full_names[17]) == 351, 'full351 names differ')
    need(sha(('\n'.join(full_names[17]) + '\n').encode()) == '46b08df4cf3782c0f225bef8467aa3e32441b9d48e11b0d78a77d3aae61bf241', 'full351 raw name identity changed')
    return {'runs': reports, 'full351_names': full_names[17],
            'full351_names_sha256': sha(('\n'.join(full_names[17]) + '\n').encode())}


def timing(e, label, old, new, ov, nv):
    values = []
    for name, version in [(old, ov), (new, nv)]:
        build_id = re.search(r'build (resident-v1-[a-f0-9]{64})', e.text(f'native/candidate-build-v{version}/engine-version.txt'))
        need(build_id is not None, 'missing executable build identity')
        values.append(session(e, name, ENGINE_HASHES[version], build_id[1], version))
    (ma, sa, ha), (mb, sb, hb) = values
    need(ha == hb and ha['execution'] == 'local native direct', 'unmatched host/execution')
    for key in ['source_before', 'toolchain_before', 'profiles', 'proc_macro_policy', 'store_scope']:
        need(ma[key] == mb[key], 'unmatched paired input: ' + key)
    need(ma['tools_before']['reference'] == mb['tools_before']['reference'], 'unmatched reference')
    before, after = sa['engine']['median_seconds'], sb['engine']['median_seconds']
    ra, rb = before / sa['reference']['median_seconds'], after / sb['reference']['median_seconds']
    raw, normalized = (after / before - 1) * 100, (rb / ra - 1) * 100
    if label == 'v18':
        need(0 < raw < 10, 'rejected-v18 conclusion contradicted')
    elif label in ['v16','v17','v17-cumulative']:
        need(-10 < raw < 0, 'historical below-floor conclusion contradicted')
    elif label == 'v19':
        need(raw < -10, 'first above-floor v19 observation contradicted')
    elif label == 'v19-reverse':
        need(-10 < raw < 0 and normalized > 0, 'reverse-order nonreplication conclusion contradicted')
    floor_text = ('exceeds' if abs(raw) > 10 else 'does not exceed')
    expected = ('Native resident scoped diagnostic: descriptive/uncontrolled, noise unknown.\n'
                f'Engine median: {before:.6f}s -> {after:.6f}s, delta {after-before:+.6f}s ({raw:+.2f}%).\n'
                f'Reference-normalized engine/reference ratio: {ra:.6f} -> {rb:.6f}, relative delta {normalized:+.2f}%.\n'
                f'Absolute median delta {floor_text} requested 10% floor. No significance or speedup verdict.\n'
                'Not a comprehensive workload or default-promotion gate.\n')
    need(e.text(f'provenance/suite-{label}-compare.log') == expected, 'raw comparison replay differs')
    for suffix in ['.exit', '-baseline.exit', '-candidate.exit']:
        need(e.text(f'provenance/suite-{label}{suffix}').strip() == '0', 'suite process gate failed')
    # Decimal math from exact JSON numeric lexemes, not rounded six-place logs.
    exact = []
    for name in [old, new]:
        rows = json.loads(e.text(f'sessions/{name}/records.json'), parse_float=Decimal)
        exact.append({stack: statistics.median([r['elapsed_seconds'] for r in rows if r['stack'] == stack and r['phase'] == 'edit']) for stack in ['engine', 'reference']})
    with localcontext() as context:
        context.prec = 50
        a, b = exact
        arithmetic = {'engine_before': str(a['engine']), 'engine_after': str(b['engine']),
                      'reference_before': str(a['reference']), 'reference_after': str(b['reference']),
                      'raw_percent': str((b['engine']/a['engine']-1)*100),
                      'normalized_percent': str(((b['engine']/b['reference'])/(a['engine']/a['reference'])-1)*100)}
    return {'pair': [old, new], 'engine_samples': [sa['engine']['samples_seconds'], sb['engine']['samples_seconds']],
            'raw_delta_percent': raw, 'normalized_delta_percent': normalized,
            'exact_decimal_replay': arithmetic, 'noise': 'unknown', 'promotion': False, 'absolute_floor_exceeded': abs(raw) > 10}


def traces(e, version):
    prefix = f'traces/v{version}/'; state = e.obj(prefix + 'state.json')
    need(state['scope'] == 'matched-instrumented-native-roots-not-speedup' and state['status'] == 'passed' and state['source_restored'] is True and state['identities_unchanged'] is True,
         'instrumented process/source/identity gate')
    need(sha(e.read(prefix + 'original.rs')) == state['source_before'], 'instrumented source identity')
    rows = state['rows']; need(len(rows) == 12, '12 actual trace rows required')
    expected_order = [('old', 'warmup'), ('new', 'warmup')]
    for rep in range(5): expected_order += [(stack, rep) for stack in (['old', 'new'] if rep % 2 == 0 else ['new', 'old'])]
    need([(r['stack'], r['replicate']) for r in rows] == expected_order, 'trace ordering/replicates mismatch')
    summary_rows = []
    for stack in ['old', 'new']:
        for rep in ['warmup'] + list(range(5)):
            row = next(r for r in rows if r['stack'] == stack and r['replicate'] == rep)
            identity = ENGINE_HASHES[version-1 if stack == 'old' else version]
            need(row['tool_sha256'] == identity and row['status'] == 0 and row['error'] is None and row['timed_out'] is False, 'trace runtime gate')
            need(row['noise']['contamination'] == 'unknown', 'trace noise guarantee invented')
            matched = {16:('rsb05','rsa11'),17:('rsb06','rsa12'),18:('rsb08','rsa14')}[version][stack == 'new']
            metadata = e.obj(f'sessions/{matched}/metadata.json')
            need(row['command'] == [metadata['tools_before']['engine']['path'], 'test', '--lib', '--', 'summary::tests::', '--test-threads=1', '--nocapture'], 'trace native command mismatch')
            need(math.isfinite(row['elapsed_seconds']) and row['elapsed_seconds'] > 0, 'invalid trace process duration')
            for key in ['CARGO_TARGET_DIR','CARGO_HOME','BLAZE_HOME','RA_JIT_HOME','BLAZE_BACKEND','BLAZE_OPT']:
                need(row['environment'][key] == metadata['environments']['engine'][key], 'trace private environment mismatch')
            raw = '\n'.join(e.text(prefix + f'{stack}-{rep}.{s}') for s in ['stdout', 'stderr'])
            names, counts = libtest(raw)
            need(names == [(name, 'ok') for name in NAMES] and counts == (4, 0, 0, 0, 144), 'trace raw runtime parity')
            if rep != 'warmup':
                need(re.findall(r'JR_RUNTIME_EXECUTED_[^\s]*', raw) == [f'JR_RUNTIME_EXECUTED_{rep+501}'], 'trace runtime marker stale')
            path = prefix + f'{stack}-{rep}.trace.json'
            need(sha(e.read(path)) == row['trace_sha256'], 'trace checksum mismatch')
            events = e.obj(path)['traceEvents']
            need(any(x.get('name') == 'client planning' for x in events), 'missing planning trace')
            selected = {}
            for event in events:
                if event.get('ph') == 'X' and event.get('name') in ('client planning', 'client metadata', 'client command', 'apply change', 'roots'):
                    selected.setdefault(event['name'], []).append(event.get('dur'))
            need(selected == row['selected_nested_elapsed_us'], 'trace selected intervals mismatch')
            intervals = {name: [{'thread':x.get('tid'), 'start_us':x['ts'], 'duration_us':x['dur']}
                               for x in events if x.get('ph') == 'X' and x.get('name') == name] for name in PHASES}
            need(all(math.isfinite(x['duration_us']) and x['duration_us'] >= 0 for xs in intervals.values() for x in xs), 'invalid trace duration')
            if rep != 'warmup':
                need(all(len(xs) == 1 for xs in intervals.values()), 'missing/duplicate measured trace phase')
                summary_rows.append({'stack':stack, 'replicate':rep, 'intervals':intervals})
    medians = {stack: {} for stack in ['old', 'new']}
    for stack in medians:
        for name in PHASES:
            samples = [r['intervals'][name][0]['duration_us'] for r in summary_rows if r['stack'] == stack and len(r['intervals'][name]) == 1]
            medians[stack][name] = {'single_interval_samples':len(samples), 'median_elapsed_us':statistics.median(samples) if len(samples) == 5 else None}
    phase = e.obj(prefix + 'phase-summary.json')
    need(phase['rows'] == summary_rows and phase['medians'] == medians, 'phase-summary not derived from actual traces')
    need(phase['scope'] == 'nested elapsed intervals, not CPU work, time saved, critical path or total speedup', 'trace phase overclaim')
    return {'actual_rows':12, 'medians':medians, 'scope':phase['scope']}


def cleanup(e):
    data = e.obj('provenance/resident-private-cleanup-before-v19-20261030.json')
    need(data['scope'] == 'native stop receipts, not independent process-exit attestation; no stores deleted', 'cleanup guarantee overclaim')
    cases = ['rsb02','rsa07','rsa08','rsb03','rsa09','rsb04','rsa10','rsb05','rsa11','rsb06','rsa12','rsb07','rsa13','rsb08','rsa14','rsa04','rsa05']
    need([(r['case'],r['stack']) for r in data['rows']] == [(case,stack) for case in cases for stack in ['reference','engine']], 'cleanup missing/duplicate namespace')
    captured = {name for _,old,new,_,_ in PAIRS for name in [old,new]}
    for row in data['rows']:
        need(row['status'] == 0 and re.fullmatch('[a-f0-9]{64}',row['tool_sha256']), 'cleanup stop failed/identity missing')
        prefix = f"/home/jeremy/{row['case']}/{row['stack']}/"
        need(set(row['namespace']) == {'BLAZE_HOME','RA_JIT_HOME'} and all(path.startswith(prefix) and '..' not in Path(path).parts for path in row['namespace'].values()), 'cleanup namespace escaped')
        need(row['command'][1:] == (['cargo','stop'] if row['stack']=='reference' else ['stop']), 'cleanup command changed')
        if row['case'] in captured:
            meta = e.obj(f"sessions/{row['case']}/metadata.json")
            need(row['tool_sha256'] == meta['tools_before'][row['stack']]['sha256'] and row['command'][0] == meta['tools_before'][row['stack']]['path'], 'cleanup executable binding mismatch')
            need(all(path == meta['environments'][row['stack']][key] for key,path in row['namespace'].items()), 'cleanup metadata namespace mismatch')
    return {'stop_receipts':34, 'scope':data['scope'], 'historical_pairs_pooled_with_final':False, 'memory_pressure_changed_before_final_pair':True}


def audit(e, verify_inventory=True):
    try:
        count = inventory(e) if verify_inventory else None
        provenance = e.obj('provenance.json')
        need(provenance['decision'] == 'REJECTED_NOT_REPLICATED' and provenance['promotion'] is False and provenance['noise'] == 'unknown' and provenance['floor_percent'] == 10, 'promotion/noise/floor overclaim')
        need(provenance['pairs'] == [{'label':l, 'baseline':a, 'candidate':b, 'baseline_version':av, 'candidate_version':bv} for l,a,b,av,bv in PAIRS], 'wrong matched pair lineage')
        sources(e)
        return {'audit':'PASS', 'decision':'REJECTED_NOT_REPLICATED', 'promotion':False,
                'artifacts':count, 'native':native(e), 'pre_v19_cleanup':cleanup(e),
                'separate_warm_observation':warm_checks.replay(e, ENGINE_HASHES),
                'timings':{label:timing(e,label,old,new,ov,nv) for label,old,new,ov,nv in PAIRS},
                'traces':{str(v):traces(e,v) for v in [16,17,18]},
                'scope':'unknown noise, scoped diagnostic only, not shipping or broad/default-promotion gate',
                'source':'v16/v17/v18/v19 full source lineage retained; evidence retention never restores or commits live compiler source'}
    except (KeyError, TypeError, IndexError, UnicodeError, json.JSONDecodeError, StopIteration) as error:
        raise InvalidEvidence(f'malformed evidence: {error}') from error


if __name__ == '__main__':
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('root', nargs='?', type=Path, default=Path(__file__).resolve().parent)
    args = parser.parse_args()
    try:
        print(json.dumps(audit(Evidence(args.root)), indent=2))
    except InvalidEvidence as error:
        raise SystemExit('INVALID EVIDENCE: ' + str(error))
