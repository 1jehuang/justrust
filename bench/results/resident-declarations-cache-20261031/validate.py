#!/usr/bin/env python3
"""Portable frozen-evidence audit. Never runs an engine, compiler, Git, network or benchmark."""
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
COMMITMENTS = '18df8bd714388afcda0ac7d1bbf67facc5f974b17a9664d9bece8324886e8fec'
PHASES = ['cold-prepare', 'repeated-noop-1', 'manifest-feature-add', 'type-error', 'type-recovery', 'assert-failure', 'assert-recovery']
SESSIONS = ['v40-off', 'v40-on', 'v41-on', 'v41-off', 'v41-off-repeat', 'v41-on-repeat']
ENV_ALLOW = set('HOME CARGO_HOME CARGO_TARGET_DIR RUSTUP_HOME BLAZE_HOME BLAZE_CACHE BLAZE_BACKEND BLAZE_OPT RA_JIT_HOME RA_JIT_CACHE XDG_CACHE_HOME XDG_CONFIG_HOME XDG_DATA_HOME XDG_RUNTIME_DIR'.split())
ENV_ALLOW |= {f'CARGO_PROFILE_{profile}_{key}' for profile in ('DEV', 'TEST') for key in ('DEBUG', 'INCREMENTAL', 'OPT_LEVEL')}
FIXTURES = set('declarations_certificate_tracks_dependency_content_and_edges declarations_external_byte_reads_are_native_unsupported_inputs declarations_round_trip_remaps_fresh_database_and_preserves_native_walk declarations_portable_record_preserves_order_duplicates_and_body_queues declarations_reject_truncation_corruption_and_wrong_certificate declarations_certificate_tracks_source_cfg_path_inventory_and_environment declarations_replay_macro_environment_diagnostics_only_after_complete_decode declarations_do_not_promote_local_fixture_paths_to_immutable_libraries declarations_observed_env_witness_allows_nonce_changes_but_rejects_changed_reads'.split())


def require(ok, message):
    if not ok:
        raise ValueError(message)


def sha(data):
    return hashlib.sha256(data).hexdigest()


def load(path):
    return json.loads(path.read_text())


def canonical(value):
    return json.dumps(value, sort_keys=True, separators=(',', ':')).encode()


def files(root):
    found = {}
    for path in sorted(root.rglob('*')):
        require(not path.is_symlink(), 'symlink in evidence')
        if path.is_file() and path not in (root / 'SHA256SUMS', root / 'validation.json') and '__pycache__' not in path.parts:
            found[str(path.relative_to(root))] = sha(path.read_bytes())
    return found


def seal(root):
    (root / 'SHA256SUMS').write_text(''.join(f'{digest}  {name}\n' for name, digest in sorted(files(root).items())))


def inventory(root):
    listed = {}
    for line in (root / 'SHA256SUMS').read_text().splitlines():
        require(re.fullmatch(r'[0-9a-f]{64}  .+', line), 'bad inventory entry')
        digest, name = line.split('  ', 1)
        require(not Path(name).is_absolute() and '..' not in Path(name).parts and name not in listed, 'unsafe inventory path')
        listed[name] = digest
    require(listed == files(root), 'transport hash inventory mismatch')


def helpers(root, commitments):
    for name, digest in commitments['helpers'].items():
        require(sha((root / name).read_bytes()) == digest, 'frozen helper commitment')
    for name in ('blaze_edit_compare', 'blaze_runtime_compare', 'resident_engine_compare', 'resident_workload_compare', 'resident_state_matrix'):
        path = root / 'provenance' / (name + '.py')
        spec = importlib.util.spec_from_file_location(name, path)
        module = importlib.util.module_from_spec(spec)
        sys.modules[name] = module
        spec.loader.exec_module(module)
    return sys.modules['resident_engine_compare'], sys.modules['resident_state_matrix']


def apply_unified(original, patch):
    lines, out, cursor = original.splitlines(keepends=True), [], 0
    parts, at = patch.splitlines(keepends=True), 0
    while at < len(parts):
        match = re.match(r'@@ -(\d+)(?:,(\d+))? \+(\d+)(?:,(\d+))? @@', parts[at])
        if not match:
            at += 1
            continue
        start = max(0, int(match[1]) - 1)
        require(start >= cursor, 'overlapping source patch')
        out.extend(lines[cursor:start])
        cursor, old, new = start, 0, 0
        at += 1
        while at < len(parts) and parts[at][:1] in (' ', '+', '-') and not parts[at].startswith(('--- ', '+++ ')):
            mark, text = parts[at][0], parts[at][1:]
            if mark in (' ', '-'):
                require(cursor < len(lines) and lines[cursor] == text, 'source patch context')
                cursor += 1
                old += 1
            if mark in (' ', '+'):
                out.append(text)
                new += 1
            at += 1
        require(old == int(match[2] or 1) and new == int(match[4] or 1), 'source patch hunk size')
    out.extend(lines[cursor:])
    return ''.join(out)


def source_audit(root, c):
    for name, digest in c['frozen_sources'].items():
        require(sha((root / name).read_bytes()) == digest, 'frozen native source commitment')
    base = root / 'frozen'
    patch = (base / 'v39/candidate.patch').read_text()
    sections = re.split(r'(?=^diff --git )', patch, flags=re.M)
    for name in ('main', 'frozen'):
        selected = next(p for p in sections if p.startswith('diff --git a/toolchain/resident/crates/ra-jit/src/' + name + '.rs '))
        actual = apply_unified((base / f'v39/{name}.head.rs').read_text(), selected)
        require(actual == (base / f'v39/{name}.candidate.rs').read_text(), 'v39 original-to-candidate patch replay')
    sections = re.split(r'(?=^diff --git )', (base / 'v41/incremental-v40-v41.patch').read_text(), flags=re.M)
    require(apply_unified((base / 'v41/frozen.v40-original.rs').read_text(), sections[1]) == (base / 'v41/frozen.candidate.rs').read_text(), 'v40-to-v41 patch replay')
    require((base / 'v40/frozen.rs').read_bytes() == (base / 'v41/frozen.v40-original.rs').read_bytes(), 'v40 repair snapshot join')
    repaired = (base / 'v40/frozen.rs').read_text()
    require('#[salsa::tracked(returns(copy))]' in repaired and repaired.count('map(hir_def::DefWithBodyId::from)') == 2, 'native compiler repair retained')
    candidate = (base / 'v41/frozen.candidate.rs').read_text()
    require('fn declarations_env_digest' in candidate and 'declaration observed environment changed' in candidate and 'Dec::new_checked' in candidate, 'actual checked observed-read implementation retained')
    require((base / 'v39/stable.head.rs').read_bytes() == (base / 'v39/stable.candidate.rs').read_bytes(), 'wrong-owner stable source was changed')
    for version in (39, 40, 41):
        manifest = (root / f'gates/candidate-build-v{version}/all-source-before.sha256').read_text().splitlines()
        table = dict((line.split('  ', 1)[1], line.split('  ', 1)[0]) for line in manifest)
        require(len(table) == 3757, '3757-source identity coverage')
        native = base / ('v39/frozen.candidate.rs' if version == 39 else 'v40/frozen.rs' if version == 40 else 'v41/frozen.candidate.rs')
        require(table['crates/ra-jit/src/frozen.rs'] == sha(native.read_bytes()), 'native source-to-build identity join')
        require(table['crates/ra-jit/src/main.rs'] == sha((base / 'v39/main.candidate.rs').read_bytes()), 'main source-to-build identity join')
        require(table['vendor/rust-analyzer/crates/hir-def/src/nameres/frozen.rs'] == sha((base / 'hir-def-frozen.rs').read_bytes()), 'portable codec identity join')
        if version != 39:
            after = (root / f'gates/candidate-unit-v{version}/all-source-after.log').read_text().splitlines()
            require(len(after) == 3757 and all(line.endswith(': OK') for line in after), 'native source restoration/after identity receipt')
            require((root / f'gates/candidate-unit-v{version}/lock-after.log').read_text().strip() == 'Cargo.lock: OK', 'lock identity receipt')


def native_audit(root, c):
    counts = []
    for gate in c['gates']:
        run = root / 'journals' / gate['run']
        meta, summary = load(run / 'meta.json'), load(run / 'summary.json')
        require(meta['id'] == summary['id'] == gate['run'], 'native run identity')
        require(summary['exit'] == gate['exit'] and summary['tests'] == gate['tests'], 'native verdict history')
        directory = root / 'gates' / gate['group']
        receipt = 'codegen-native.exit' if gate['tests']['passed'] == 48 else 'full-native.exit' if gate['tests']['passed'] in (368,369) else 'check.exit' if summary['subcommand'] == 'check' else 'compile.exit'
        require(int((directory / receipt).read_text()) == gate['exit'], 'direct native exit receipt')
        if gate['group'].startswith('candidate-build-v') and gate['exit'] == 0:
            version = gate['group'][-2:]
            require((directory / 'engine-identity.sha256').read_text().split()[0] == c['native'][version]['engine_sha'], 'native executable identity receipt')
            require((directory / 'engine-version.txt').read_text().strip() == c['native'][version]['version'], 'native version attribution receipt')
        require(set(meta.get('env', {})) <= {'JCODE_SESSION_ID', 'CARGO_TARGET_DIR', 'JCODE_SCRATCH_DIR'}, 'native environment allowlist')
        raw = [json.loads(line) for line in (run / 'output.jsonl').read_text().splitlines()]
        text = '\n'.join(line['l'] for line in raw)
        slim = load(root / 'slim' / (gate['run'] + '.json'))
        require(all(slim[k] == summary[k] for k in ('id', 'args', 'subcommand', 'start', 'wall', 'exit', 'tests', 'route')), 'slim export differs from raw native summary')
        passed = re.findall(r'^test (\S+) \.\.\. ok(?: <[^>]+>)?$', text, re.M)
        expected = gate['tests']['passed']
        require(len(passed) == len(set(passed)) == expected, 'full native test names/count mismatch')
        if gate['exit'] == 101:
            require('due to 4 previous errors' in text and 'error[E0283]' in text and text.count('error[E0277]') >= 3, 'v39 four real compiler errors absent')
        else:
            require(summary['tests']['failed'] == 0, 'native test failure')
        if expected in (368, 369):
            fixture_names = {name.rsplit('::', 1)[-1] for name in passed if name.startswith('frozen::declaration_tests::')}
            wanted = FIXTURES if expected == 369 else FIXTURES - {'declarations_observed_env_witness_allows_nonce_changes_but_rejects_changed_reads'}
            require(fixture_names == wanted, 'actual native read/portable fixture coverage')
        counts.append(dict(run=gate['run'], exit=gate['exit'], tests=expected))
    return counts


def runtime_audit(root, c, shared, matrix):
    result = {}
    pinned = {str(path.relative_to(root / 'input/source63')): sha(path.read_bytes()) for path in (root / 'input/source63').rglob('*') if path.is_file()}
    require(pinned == c['source_before'], 'pinned source63 identity')
    for name in SESSIONS:
        base = root / 'runtime' / name
        state = load(base / 'state.json')
        require(state['status'] == 'passed' and state['unchanged'], 'runtime final verdict/restoration')
        require(state['source_before'] == state['source_after'] == c['source_before'], 'runtime source identity')
        require(state['toolchain_before'] == state['toolchain_after'], 'runtime toolchain restoration')
        require(sha(canonical(state['toolchain_before'])) == c['toolchain_sha256'], 'runtime toolchain commitment')
        require(len(state['toolchain_before']['files']) == 4020, '4020 toolchain file identities')
        version = name[1:3]
        require(state['tool_before'] == state['tool_after'] == c['native'][version]['engine_sha'], 'runtime tool-to-native binary join')
        require(state['startup_controls'] == dict(RA_JIT_DEMAND_STD='1', RA_JIT_DECLARATIONS_CACHE='1' if '-on' in name else '0', RA_JIT_STATS='1'), 'explicit startup opt-in control')
        require([row['phase'] for row in state['rows']] == PHASES, 'complete ordered seven-state runtime coverage')
        originals = {file: (base / f'original-{i}').read_bytes() for i, file in enumerate(matrix.FILES)}
        require(all(sha(data) == c['source_before'][file] for file, data in originals.items()), 'durable source originals')
        rows, expected, seen_hits = [], None, []
        for i, row in enumerate(state['rows']):
            phase = row['phase']
            require(row['accepted'] and not row['failures'], 'runtime acceptance claim')
            require(math.isfinite(row['elapsed_seconds']) and row['elapsed_seconds'] > 0, 'runtime elapsed validity')
            require(row['noise']['contamination'] == 'unknown', 'uncontrolled-noise claim changed')
            require(set(row['environment']) <= ENV_ALLOW, 'runtime environment allowlist')
            wanted = matrix.inputs(originals, phase, 0, row['nonce'])
            source = dict(c['source_before'])
            source.update({file: sha(data) for file, data in wanted.items()})
            require(row['inputs'] == source, 'exact phase source input recipe')
            text = ''
            for stream in ('stdout', 'stderr'):
                path = base / f'{i:02}-{phase}.{stream}'
                data = path.read_bytes()
                receipt = row['raw'][stream]
                require(receipt['bytes'] == len(data) and receipt['sha256'] == sha(data), 'nested raw runtime receipt')
                text += data.decode(errors='replace') + '\n'
            parsed = shared.runtime.test_results(text)
            require(canonical(parsed) == canonical(row['parsed']), 'raw libtest parser join')
            if i == 0:
                expected = matrix.contract(parsed, phase)
                require(expected is not None, 'exact four-test native cold contract')
            require(not matrix.oracle(dict(row, parsed=parsed), text, expected, phase, row['nonce'], 0), 'strict actual runtime state oracle')
            log = (base / f'{i:02}-{phase}-worker-0.log').read_text()
            hits = re.findall(r'declarations-cache: native_hit=1 native_hits=(\d+)', log)
            hit_values = list(map(int, hits))
            require(hit_values == list(range(1, len(hit_values) + 1)), 'native hit counter sequence/cumulative history')
            previous = seen_hits[-1] if seen_hits else 0
            require(len(hit_values) >= previous or (name == 'v40-on' and i == 2), 'worker cumulative hit history shrank')
            seen_hits.append(len(hit_values))
            rows.append(dict(phase=phase, seconds=row['elapsed_seconds'], native_hits_cumulative=len(hit_values), native_hits_new=max(0, len(hit_values)-previous)))
        wanted_hits = [21]*7 if name.startswith('v41-on') else [21,21,0,0,0,0,0] if name == 'v40-on' else [0]*7
        require(seen_hits == wanted_hits, 'expected actual hits/cumulative not new-per-state')
        result[name] = rows
    return result


def validate(root=HERE):
    root = Path(root).resolve()
    inventory(root)
    require(sha((root / 'commitments.json').read_bytes()) == COMMITMENTS, 'independent historical commitments')
    c = load(root / 'commitments.json')
    require(c['decision'] == 'REJECT/no promotion', 'rejection decision')
    source_audit(root, c)
    native = native_audit(root, c)
    shared, matrix = helpers(root, c)
    runtime = runtime_audit(root, c, shared, matrix)
    pairs = {}
    for suffix in ('', '-repeat'):
        on, off = runtime['v41-on'+suffix], runtime['v41-off'+suffix]
        pairs['reverse' if suffix else 'first'] = {a['phase']: dict(off_seconds=b['seconds'], on_seconds=a['seconds'], percent_change=100*(a['seconds']/b['seconds']-1)) for a,b in zip(on,off)}
    require(pairs['first']['assert-recovery']['percent_change'] > 20 and pairs['reverse']['assert-recovery']['percent_change'] > 9, 'important recovery regressions must not be promoted')
    return dict(decision='REJECT/no promotion', native_gates=native, runtime_rows=42, v41_rows=28, runtime=runtime, paired= pairs, noise='unknown/uncontrolled', comprehensive_benchmark=False, new_native_calls=0, scope='Offline retention and semantic replay only. 21 hits are cumulative daemon-history markers, all first observed at cold state, not 21 new hits at each transition.')


if __name__ == '__main__':
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--seal', action='store_true', help='Regenerate resignable transport inventory, not historical proof')
    args = parser.parse_args()
    if args.seal:
        seal(HERE)
    report = validate()
    (HERE / 'validation.json').write_text(json.dumps(report, indent=2, sort_keys=True) + '\n')
    print(json.dumps(dict(decision=report['decision'], native_gates=len(report['native_gates']), runtime_rows=report['runtime_rows'], native_calls=0)))
