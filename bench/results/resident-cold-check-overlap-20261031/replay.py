#!/usr/bin/env python3
"""Portable historical semantic replay of rejected cold-check-overlap evidence, no native execution."""
import argparse
import contextlib
import hashlib
import io
import json
import math
from pathlib import Path
import re
import sys
import tarfile

sys.dont_write_bytecode = True
sys.path.insert(0, str(Path(__file__).resolve().parent / 'oracle'))
import resident_state_matrix as matrix

PHASES = ['cold-prepare', 'repeated-noop-1', 'manifest-feature-add', 'type-error', 'type-recovery', 'assert-failure', 'assert-recovery']
CASES = ['cold-check-overlap-v54-' + x for x in ('off', 'on', 'on-repeat', 'off-repeat')]
MARKER = b'cold-check-overlap: asynchronous diagnostics started'
SOURCE_ANCHOR = '79c53ff6e87a22c84c9bc7bac6917c69b3f8d0fbc29b107f734cc00cc39a64df'
TC_ANCHOR = '18b7d072eafb76575f39cd49db3e2a9326d06724b82dae935bb1b8df87adb079'
SOURCE_SHA = {'original':'3ef8143a8f6294335168f9a987d3266ce347f06209e1e3c3f209b72d9c6447fd','v54':'7ccde95fa158299e440b738c85ebe0b61605260bebb89c3f3740e4dda54f845d','v54b':'7c43bf86ba2bf3e1f05f2218f29efd20155b258e9a566312d313581b6142709f'}
ENGINE = 'fa7e283ecc47832e9754f50d6329baa5dbd26f9c60427bfe332e4f44b7684a91'
ORIGINAL_ENGINE = '1197df17843a18b1cf5af78b22dc805d15bfe45056eefe1a04113599e681e3e0'
DECISION = dict(decision='REJECTED', promotion=False, latency_claim=False, zero_overhead_claim=False, noise='unknown', strict_rows=28, fixture_rows=30, worker_snapshots_are_cumulative=True, injected_panic_unset_procmacro_tests=False, alternate_lint_borrow_controls=False, long_lived_include_recovery=False)


def digest(data):
    return hashlib.sha256(data).hexdigest()


def canonical(value):
    return digest(json.dumps(value, sort_keys=True, separators=(',', ':')).encode())


def require(ok, reason):
    if not ok:
        raise ValueError(reason)


def read_json(path):
    return json.loads(path.read_text())


def inventory(root):
    require(not any(p.is_symlink() for p in root.rglob('*')), 'no bundle links')
    return sorted(p for p in root.rglob('*') if p.is_file() and p.name != 'SHA256SUMS.bundle' and '__pycache__' not in p.parts)


def seal(root):
    (root / 'SHA256SUMS.bundle').write_text(''.join(f'{digest(p.read_bytes())}  {p.relative_to(root).as_posix()}\n' for p in inventory(root)))


def manifest(path):
    result = {}
    for line in path.read_text().splitlines():
        sha, name = line.split('  ', 1)
        require(re.fullmatch('[0-9a-f]{64}', sha) is not None and name not in result and not Path(name).is_absolute() and '..' not in Path(name).parts, 'safe unique manifest paths')
        result[name] = sha
    return result


def verify_seal(root):
    listed = manifest(root / 'SHA256SUMS.bundle')
    require(set(listed) == {p.relative_to(root).as_posix() for p in inventory(root)}, 'complete bundle inventory')
    for name, sha in listed.items():
        require(digest((root / name).read_bytes()) == sha, 'transport checksum ' + name)


def native_result(run):
    text = '\n'.join(json.loads(line)['l'] for line in (run / 'output.jsonl').read_text().splitlines())
    # Original libtest names/statuses stay intact, only this literal annotation is removed.
    text = re.sub(r'^(test [^\s]+) - should panic(?= \.\.\. )', r'\1', text, flags=re.M)
    return matrix.shared.runtime.test_results(text)


def validate_native(root):
    path = 'toolchain/resident/crates/ra-jit/src/main.rs'
    for version in ('v54', 'v54b'):
        directory = root / 'frozen' / version
        for name, sha in manifest(directory / 'SHA256SUMS').items():
            require(digest((directory / name).read_bytes()) == sha, 'frozen inner checksum ' + version + name)
        require(digest((directory / 'original' / path).read_bytes()) == SOURCE_SHA['original'], 'original anchor')
        require(digest((directory / 'candidate' / path).read_bytes()) == SOURCE_SHA[version], 'candidate anchor')
    frozen = root / 'frozen/v54b'
    require((frozen / 'live-before-restoration/main.rs').read_bytes() == (frozen / 'candidate' / path).read_bytes(), 'live candidate preserved')
    require((frozen / 'live-before-restoration/resident-cold-check-overlap.md').read_bytes() == (frozen / 'candidate/docs/resident-cold-check-overlap.md').read_bytes(), 'live doc preserved')
    require((frozen / 'preliminary-v54' / path).read_bytes() == (root / 'frozen/v54/candidate' / path).read_bytes(), 'preliminary lineage')
    build, unit = root / 'native/candidate-build-v54', root / 'native/candidate-unit-v54'
    for base, names in [(build, ['check.exit','compile.exit','orchestration-exit']), (unit, ['codegen-native.exit','full-native.exit','exit'])]:
        for name in names:
            require((base / name).read_text().strip() == '0', 'actual native exit ' + name)
    source = manifest(build / 'all-source-before.sha256')
    require(len(source) == 3757 and source['crates/ra-jit/src/main.rs'] == SOURCE_SHA['v54b'], 'whole3757 source to candidate join')
    require(canonical(source) == '0c7f0f7796efdc4302359fea17ebd9ff9319754cd03f685fa4444c9f507841ec', 'exact whole3757 source anchor')
    require((unit / 'all-source-after.log').read_text().splitlines() == [name + ': OK' for name in source], 'whole3757 immutable source')
    require(list(manifest(build / 'lock-before.sha256').values()) == [digest((root / 'provenance/production-Cargo.lock').read_bytes())], 'lock before join')
    require((unit / 'lock-after.log').read_text().strip() == 'Cargo.lock: OK', 'lock after')
    require((build / 'engine-identity.sha256').read_text().split()[0] == ENGINE, 'native binary anchor')
    require((build / 'engine-version.txt').read_text().strip() == 'jr-engine experimental 0.20261027.0 (Blaze-derived origin 5d4f78f7577cf7170993b74f735099dd03f5c6ca; build resident-v1-a78f239e8e6118c3b98a8b417182df5a16e532cefeee442decda648dbc0a5b6e)', 'native origin/build anchor')
    commands = []
    for run in sorted((build / 'runs').iterdir()):
        meta, summary = read_json(run / 'meta.json'), read_json(run / 'summary.json')
        require(meta['id'] == summary['id'] == run.name and summary['exit'] == 0 and meta['args'] == summary['args'], 'build journal join')
        commands.append(meta['args'])
    require(commands == [['check','--release','--locked','-p','ra-jit','--tests','-p','hir-ty-codegen','-j','6'], ['build','--release','--locked','-p','ra-jit','--bin','jr-engine','-j','6']], 'exact check/build commands')
    totals = []
    for run in (unit / 'runs').iterdir():
        meta, summary = read_json(run / 'meta.json'), read_json(run / 'summary.json')
        parsed = native_result(run)
        require(meta['id'] == summary['id'] == run.name and summary['exit'] == 0 and meta['args'] == summary['args'], 'test journal join')
        require(parsed is not None and (parsed['failed'],parsed['ignored'],parsed['filtered']) == (0,0,0), 'complete native totals')
        count = parsed['passed']
        wanted = ['test','--release','--locked','-p','hir-ty-codegen','-j','6'] if count == 53 else ['test','--release','--locked','-p','ra-jit','--bin','jr-engine','-j','6']
        require(count in (53,360) and meta['args'] == wanted, 'exact native test controls')
        require(canonical(parsed) == {53:'3cea71e7d802a608b600ec3655d3e268679dc6156745fdbc0739c34e2cd8f73d',360:'7b8002405c0b8140efe15cd3f3ddc8d619fbd47422fb143e41caf9b57e56bd75'}[count], 'exact native test identities/statuses')
        totals.append(count)
    require(sorted(totals) == [53,360], 'actual53/360 totals')


def validate_fixtures(root):
    directory = root / 'fixtures'
    state = read_json(directory / 'state.json')
    cases = ['valid','unused_error','reached_error','denied_lint','borrow_error','include_valid','include_error','include_module','macro_error','syntax_error']
    require(state['status'] == 'collected' and len(state['rows']) == 30 and state['source_unchanged_except_new_lock'] and state['identities_unchanged'], 'fixture collection completed')
    require(state['tools'] == state['tools_after'] == {'original': ORIGINAL_ENGINE, 'off': ENGINE, 'on': ENGINE}, 'fixture actual binary joins')
    require(state['toolchain'] == state['toolchain_after'] and canonical(state['toolchain']['files']) == TC_ANCHOR and len(state['toolchain']['files']) == 4020, 'fixture whole4020 toolchain')
    before = {p.relative_to(directory / 'source').as_posix(): digest(p.read_bytes()) for p in (directory / 'source').rglob('*') if p.is_file()}
    require(before == state['source_after'], 'fixture final source bytes')
    frozen = root / 'frozen/v54/fixtures'
    inputs = {p.relative_to(frozen).as_posix(): digest(p.read_bytes()) for p in frozen.rglob('*') if p.is_file()}
    require(inputs == state['source_before'] and all(before.get(k) == v for k,v in inputs.items()) and set(before)-set(inputs) <= {'Cargo.lock'}, 'fixture source input identity and new-lock-only change')
    require([(r['case'],r['mode']) for r in state['rows']] == [(c,m) for c in cases for m in ('original','off','on')], 'exact30 fixture order')
    require(canonical({p.name:digest(p.read_bytes()) for p in directory.glob('*-stop.std*')}) == '9ef4c5ca9dc2045c852110e49d3e03366d4073e2078bb5c1bf762957d6545599', 'exact all30 post-observation stop raw receipts')
    results, diagnostics = [], {}
    for case in cases:
        observed = []
        for row in [r for r in state['rows'] if r['case'] == case]:
            mode = row['mode']
            require(row['complete'] and not row['timed_out'] and not row.get('error') and not row.get('raw_error'), 'fixture complete raw observation')
            require(row['startup_controls'] == dict(RA_JIT_DEMAND_STD='1',RA_JIT_STATS='1',RA_JIT_COLD_CHECK_OVERLAP='1' if mode == 'on' else '0'), 'fixture startup controls')
            require(row['command'][1:] == ['run','--bin',case] and Path(row['command'][0]).name == 'jr-engine', 'fixture actual run command')
            stop = row['post_observation_stop']
            require(stop['status'] == 0 and not stop['timed_out'] and not stop.get('error') and stop['command'][1:] == ['stop'], 'actual post-observation stop')
            streams = []
            for receipt, prefix in ((row['raw'],case+'-'+mode+'-raw'), (None,case+'-'+mode+'-stop')):
                pair = []
                for suffix in ('stdout','stderr'):
                    data = (directory / (prefix+'.'+suffix)).read_bytes()
                    if receipt is not None:
                        require(len(data) == receipt[suffix]['bytes'] and digest(data) == receipt[suffix]['sha256'] and Path(receipt[suffix]['path']).name == prefix+'.'+suffix, 'fixture raw byte receipt')
                    pair.append(data)
                streams.append(pair)
            out, err = streams[0]
            diagnostic = err[err.index(b'error'):err.index(b'blaze time:')] if b'error' in err else b''
            require(row['status'] == (0 if case in ('valid','include_valid') else 101), 'fixture exact status')
            require(bool(diagnostic) == (case not in ('valid','include_valid')), 'fixture error/success diagnostic presence')
            if not diagnostic:
                require(b'Running `' in err, 'fixture native executable ran')
            require(len(row['worker_logs']) == 1, 'fixture one worker')
            lines = (directory / row['worker_logs'][0]).read_bytes().splitlines()
            count = sum(line == MARKER for line in lines)
            require(sum(MARKER in line for line in lines) == count == int(mode == 'on'), 'fixture actual fixed startup marker')
            observed.append((row['status'],out,diagnostic))
        require(observed[0] == observed[1] == observed[2], 'complete original/OFF/ON diagnostics ' + case)
        diagnostics[case] = digest(observed[0][2])
        results.append(dict(case=case,status=observed[0][0],complete_diagnostics_identical=True,activation_controls_passed=True))
    require(canonical(diagnostics) == '44d4b9c21b409d7a0fb0089577eae7d9d9e9317032e0c9ee1b1c02117dc6be35', 'complete original diagnostic anchor')
    analysis = read_json(root / 'provenance/cold-check-fixtures-v54-validation.json')
    require(analysis['status'] == 'passed' and analysis['rows'] == 30 and analysis['cases'] == results and analysis['limitations'] == 'No injected panic/unset/procmacro failure proof, no long-lived include recovery or lint/borrow alternate policy coverage in this fixture batch.', 'exact original fixture analysis/limits')
    # Original validator is preserved verbatim. Only its historical data-root binding
    # is relocated in memory, without loosening its diagnostic comparison.
    source = (root / 'provenance/validate-cold-check-fixtures-v54.py').read_text()
    binding = "ROOT=Path('/home/jeremy/.jcode/scratch/blaze-native-engine-20261030')"
    require(source.count(binding) == 1, 'original fixture validator root binding')
    source = source.replace(binding, 'ROOT=Path(__archive_root__)').replace("p=ROOT/'cold-check-overlap-v54-fixture-observations'", "p=ROOT/'fixtures'")
    output = io.StringIO()
    with contextlib.redirect_stdout(output):
        exec(compile(source, 'original-fixture-validator', 'exec'), {'__archive_root__':str(root),'__name__':'__main__'})
    require(json.loads(output.getvalue()) == analysis, 'original fixture exact semantic replay')
    return results


def validate(root):
    verify_seal(root)
    require(read_json(root / 'decision.json') == DECISION, 'rejection and missing-coverage boundaries')
    validate_native(root)
    fixture_results = validate_fixtures(root)
    pinned = {}
    with tarfile.open(fileobj=io.BytesIO((root / 'snapshots/source63.tar.gz').read_bytes()), mode='r:gz') as archive:
        for entry in archive:
            require(entry.isfile() and not Path(entry.name).is_absolute() and '..' not in Path(entry.name).parts and entry.name not in pinned, 'safe workload archive')
            pinned[entry.name] = archive.extractfile(entry).read()
    source_identity = {name: digest(data) for name, data in pinned.items()}
    require(len(pinned) == 63 and canonical(source_identity) == SOURCE_ANCHOR, 'pinned63 workload anchor')
    timing, marker_counts, index_rows = {}, {}, []
    for case in CASES:
        directory = root / 'raw' / case
        state = read_json(directory / 'state.json')
        enabled = case in (CASES[1], CASES[2])
        require(state['status'] == 'passed' and state['unchanged'], 'actual state completion')
        require(state['startup_controls'] == dict(RA_JIT_DEMAND_STD='1', RA_JIT_CACHE_MISS_STATS='0', RA_JIT_STATS='1', RA_JIT_COLD_CHECK_OVERLAP='1' if enabled else '0'), 'exact runtime startup controls')
        require(state['source_before'] == state['source_after'] == source_identity, 'pinned source restored')
        require(state['tool_before'] == state['tool_after'] == ENGINE, 'same native binary before/after runtime')
        require(state['toolchain_before'] == state['toolchain_after'], 'runtime toolchain unchanged')
        tc = state['toolchain_before']['files']
        require(len(tc) == 4020 and canonical(tc) == TC_ANCHOR and tc['lib/rustlib/multirust-channel-manifest.toml']['sha256'] == matrix.shared.TOOLCHAIN_MANIFEST, 'exact whole4020 pinned toolchain anchor')
        require([row['phase'] for row in state['rows']] == PHASES, 'exact seven phase order')
        originals = {name: (directory / f'original-{i}').read_bytes() for i, name in enumerate(matrix.FILES)}
        require(all(data == pinned[name] for name, data in originals.items()), 'actual input backups match pinned source')
        expected = None
        timing[case], marker_counts[case] = [], []
        for i, row in enumerate(state['rows']):
            phase, nonce = row['phase'], row['nonce']
            require(re.fullmatch('[0-9a-f]{32}', nonce) is not None, 'nonce shape')
            require(row['accepted'] and not row['failures'] and row['input_unchanged'], 'recorded oracle status')
            require(math.isfinite(row['elapsed_seconds']) and row['elapsed_seconds'] > 0 and row['noise']['contamination'] == 'unknown', 'finite elapsed/unknown noise boundary')
            require(row['command'][1:] == ['test', *matrix.arguments(phase)] and Path(row['command'][0]).name == 'jr-engine', 'actual native command')
            inputs = dict(source_identity)
            inputs.update({name: digest(data) for name, data in matrix.inputs(originals, phase, 0, nonce).items()})
            require(row['inputs'] == inputs, 'actual phase input recipe')
            streams = []
            for suffix in ('stdout', 'stderr'):
                name = f'{i:02}-{phase}.{suffix}'
                data, receipt = (directory / name).read_bytes(), row['raw'][suffix]
                require(len(data) <= matrix.MAX_RAW_BYTES and len(data) == receipt['bytes'] and digest(data) == receipt['sha256'] and Path(receipt['path']).name == name, 'actual raw byte receipt')
                streams.append(data.decode(errors='replace'))
            text = '\n'.join(streams)
            parsed = matrix.shared.runtime.test_results(text)
            require(json.loads(json.dumps(parsed)) == row['parsed'], 'exact raw test parser replay')
            if i == 0:
                expected = matrix.contract(parsed, phase)
                require(expected is not None, 'cold exact test contract')
            failures = matrix.oracle(dict(row, parsed=parsed), text, expected, phase, nonce, 0)
            require(not failures, 'original semantic oracle: ' + repr(failures))
            logs = sorted(directory.glob(f'{i:02}-{phase}-worker-*.log'))
            require(len(logs) == 1, 'one actual worker snapshot per phase')
            lines = logs[0].read_bytes().splitlines()
            count = sum(line == MARKER for line in lines)
            require(sum(MARKER in line for line in lines) == count and count == int(enabled and i < 4), 'exact cumulative asynchronous startup marker snapshots')
            marker_counts[case].append(count)
            timing[case].append(row['elapsed_seconds'])
            index_rows.append(dict(case=case, **row))
    require(index_rows == read_json(root / 'rows28.json'), 'exact all28 row index')
    rows = []
    for i, phase in enumerate(PHASES):
        times = [timing[c][i] for c in CASES]
        rows.append(dict(phase=phase, seconds=dict(zip(('off', 'on', 'on-repeat', 'off-repeat'), times)), first_percent=100*(times[1]/times[0]-1), reverse_percent=100*(times[2]/times[3]-1)))
    analysis = read_json(root / 'provenance/cold-check-overlap-v54-validation.json')
    require(analysis['status'] == 'passed' and analysis['strict_rows'] == 28 and analysis['noise'] == 'unknown' and analysis['rows'] == rows and analysis['marker_counts'] == {k.removeprefix('cold-check-overlap-v54-'): v for k, v in marker_counts.items()}, 'exact original runtime analysis')
    source = (root / 'provenance/validate-cold-check-overlap-v54.py').read_text()
    output = io.StringIO()
    with contextlib.redirect_stdout(output):
        exec(compile(source, 'original-validator', 'exec'), {'__file__': str(root / 'raw/validate-cold-check-overlap-v54.py'), '__name__': '__main__'})
    require(json.loads(output.getvalue()) == analysis, 'unchanged original validator replay')
    require((root / 'provenance/cold-check-overlap-v54-pairs.exit').read_text().strip() == '0', 'actual pair launcher exit')
    restored = read_json(root / 'restoration.json')
    require(restored['status'] == 'RESTORED' and restored['files'] == {'toolchain/resident/crates/ra-jit/src/main.rs': dict(sha256=SOURCE_SHA['original'],cmp_original=True,cmp_head=True)} and restored['observed'] == '2026-10-31T06:56:09Z' and restored['released'] == '2026-10-31T06:56:18Z', 'exact main restoration')
    return dict(status='PASS',strict_rows=28,fixture_rows=30,native=[53,360],original_semantic_oracles='PASS',original_analysis='PASS',markers=marker_counts,decision='REJECTED',noise='unknown',latency_claim=False,limitations=DECISION,rows=rows)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--seal', action='store_true', help='update transport inventory, not provenance authentication')
    args = parser.parse_args()
    root = Path(__file__).resolve().parent
    if args.seal:
        seal(root)
    print(json.dumps(validate(root), indent=2))


if __name__ == '__main__':
    main()
