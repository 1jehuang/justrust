#!/usr/bin/env python3
"""Portable historical semantic replay of rejected builder-pool evidence, no native execution."""
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
CASES = ['builder-context-v53-' + x for x in ('off', 'on', 'on-repeat', 'off-repeat')]
MARKER = b'resident: builder-context-pool reused'
SOURCE_ANCHOR = '79c53ff6e87a22c84c9bc7bac6917c69b3f8d0fbc29b107f734cc00cc39a64df'
TC_ANCHOR = '18b7d072eafb76575f39cd49db3e2a9326d06724b82dae935bb1b8df87adb079'
SOURCE_SHA = {'original': 'ec08cb7a35fa6c4bcdfbf3514e7fceb367a84eb3958817ef9f803703100a7b14', 'v52': '23b2fe7b528e597ac0596dff4ad82f68bf4527b177799c56a1842a9291068fca', 'v53': 'd191f9954867c0eebc4a0d2048e347cfdb2f1325bb0d4b5aa64431c46ed94aa9'}
ENGINES = {'v52': '6f7d5ab1526668de39f2c2102738bc492866fa5b642b185b049ea8af389f9631', 'v53': '76e527c076e368fd6f2adfa0777c86502fcbee4d3603b12f342afbf1412c2b17'}
BUILD_IDS = {'v52': '1d28c3c4652de4a1dace6e79c8508f8383a5826a60b2cdf3b89fd7e44d232aba', 'v53': 'b5f8afa936d719f3cbe377273fe573eca6ef75ae0509f71d57302d4a198201c7'}
FIXTURES = {'builder_pool_exact_default_off_gate', 'builder_pool_native_ir_signatures_and_verification_match_fresh', 'builder_pool_discards_failed_and_unfinished_native_builders', 'builder_pool_nested_native_builds_keep_only_one_idle_context'}
DECISION = dict(decision='REJECTED', promotion=False, latency_claim=False, zero_overhead_claim=False, noise='unknown', strict_rows=28, debug_finalize_panic_executed=False, worker_snapshots_are_cumulative=True)


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


def validate(root):
    verify_seal(root)
    require(read_json(root / 'decision.json') == DECISION, 'rejection/no claim/debug limitation boundary')
    frozen = root / 'frozen'
    require(digest((frozen / 'v52/original.rs').read_bytes()) == SOURCE_SHA['original'], 'original source anchor')
    sources = {}
    for version in ('v52', 'v53'):
        directory = frozen / version
        for name, sha in manifest(directory / 'SHA256SUMS').items():
            require(digest((directory / name).read_bytes()) == sha, 'frozen inner checksum ' + version + name)
        require(digest((directory / 'candidate.rs').read_bytes()) == SOURCE_SHA[version], 'frozen candidate source anchor')
        build = root / 'native' / ('candidate-build-' + version)
        unit = root / 'native' / ('candidate-unit-' + version)
        for base, names in [(build, ['check.exit', 'compile.exit', 'orchestration-exit']), (unit, ['codegen-native.exit', 'full-native.exit', 'exit'])]:
            for name in names:
                require((base / name).read_text().strip() == '0', 'native gate exit ' + version + name)
        source = manifest(build / 'all-source-before.sha256')
        sources[version] = source
        require(len(source) == 3757 and source['crates/hir-ty-codegen/mod.rs'] == SOURCE_SHA[version], 'whole3757 native source/frozen join')
        require((unit / 'all-source-after.log').read_text().splitlines() == [name + ': OK' for name in source], 'whole3757 immutable source check')
        require(list(manifest(build / 'lock-before.sha256').values()) == [digest((root / 'provenance/production-Cargo.lock').read_bytes())], 'production lock join')
        require((unit / 'lock-after.log').read_text().strip() == 'Cargo.lock: OK', 'native lock after check')
        require((build / 'engine-identity.sha256').read_text().split()[0] == ENGINES[version], 'exact native binary digest')
        require((build / 'engine-version.txt').read_text().strip() == 'jr-engine experimental 0.20261027.0 (Blaze-derived origin 5d4f78f7577cf7170993b74f735099dd03f5c6ca; build resident-v1-' + BUILD_IDS[version] + ')', 'native derived origin/build identity')
        build_commands = []
        for run in sorted((build / 'runs').iterdir()):
            meta, summary = read_json(run / 'meta.json'), read_json(run / 'summary.json')
            require(meta['id'] == summary['id'] == run.name and summary['exit'] == 0 and summary['args'] == meta['args'], 'native build journal join')
            build_commands.append(meta['args'])
        require(build_commands == [['check', '--release', '--locked', '-p', 'ra-jit', '--tests', '-p', 'hir-ty-codegen', '-j', '6'], ['build', '--release', '--locked', '-p', 'ra-jit', '--bin', 'jr-engine', '-j', '6']], 'actual check/build controls')
        totals = []
        for run in (unit / 'runs').iterdir():
            meta, summary = read_json(run / 'meta.json'), read_json(run / 'summary.json')
            parsed = native_result(run)
            require(meta['id'] == summary['id'] == run.name and summary['exit'] == 0 and summary['args'] == meta['args'], 'native test journal join')
            require(parsed is not None and (parsed['failed'], parsed['ignored'], parsed['filtered']) == (0, 0, 0), 'native complete libtest totals')
            count = parsed['passed']
            wanted = ['test', '--release', '--locked', '-p', 'hir-ty-codegen', '-j', '6'] if count == 57 else ['test', '--release', '--locked', '-p', 'ra-jit', '--bin', 'jr-engine', '-j', '6']
            require(meta['args'] == wanted and count in (57, 360), 'actual release native test controls')
            if count == 57:
                names = {name for name, status in parsed['tests'] if status == 'ok'}
                require({'tests::' + name for name in FIXTURES} <= names and 'tests::builder_pool_discards_panicking_finalize' not in names, 'actual pool fixtures and debug-only absence')
            totals.append(count)
        require(sorted(totals) == [57, 360], 'actual57 codegen360 resident totals')
    require(sources['v52'].keys() == sources['v53'].keys() and {n for n in sources['v52'] if sources['v52'][n] != sources['v53'][n]} == {'crates/hir-ty-codegen/mod.rs'}, 'only native builder source changed v52-v53')
    require((frozen / 'v53/original.rs').read_bytes() == (frozen / 'v52/candidate.rs').read_bytes() and (frozen / 'v53/original-doc.md').read_bytes() == (frozen / 'v52/doc.md').read_bytes(), 'exact v52 to v53 originals')
    require((frozen / 'v53/live-before-restoration.rs').read_bytes() == (frozen / 'v53/candidate.rs').read_bytes() and (frozen / 'v53/live-before-restoration-doc.md').read_bytes() == (frozen / 'v53/doc.md').read_bytes(), 'exact live rejected source/doc preserved')
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
        require(state['startup_controls'] == dict(RA_JIT_DEMAND_STD='1', RA_JIT_CACHE_MISS_STATS='0', RA_JIT_STATS='1', RA_JIT_BUILDER_POOL='1' if enabled else '0'), 'exact runtime startup controls')
        require(state['source_before'] == state['source_after'] == source_identity, 'pinned source restored')
        require(state['tool_before'] == state['tool_after'] == ENGINES['v53'], 'same native binary before/after runtime')
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
            require(sum(MARKER in line for line in lines) == count and count == int(enabled), 'fixed successful reuse marker ON1 OFF0')
            marker_counts[case].append(count)
            timing[case].append(row['elapsed_seconds'])
            index_rows.append(dict(case=case, **row))
    require(index_rows == read_json(root / 'rows28.json'), 'exact all28 row index')
    rows = []
    for i, phase in enumerate(PHASES):
        times = [timing[c][i] for c in CASES]
        rows.append(dict(phase=phase, seconds=dict(zip(('off', 'on', 'on-repeat', 'off-repeat'), times)), first_percent=100*(times[1]/times[0]-1), reverse_percent=100*(times[2]/times[3]-1)))
    analysis = read_json(root / 'provenance/builder-context-v53-validation.json')
    require(analysis['status'] == 'passed' and analysis['strict_rows'] == 28 and analysis['noise'] == 'unknown' and analysis['rows'] == rows and analysis['marker_counts'] == {k.removeprefix('builder-context-v53-'): v for k, v in marker_counts.items()}, 'exact original runtime analysis')
    source = (root / 'provenance/validate-builder-context-v53.py').read_text()
    output = io.StringIO()
    with contextlib.redirect_stdout(output):
        exec(compile(source, 'original-validator', 'exec'), {'__file__': str(root / 'raw/validate-builder-context-v53.py'), '__name__': '__main__'})
    require(json.loads(output.getvalue()) == analysis, 'unchanged original validator replay')
    require((root / 'provenance/builder-context-v53-pairs.exit').read_text().strip() == '0', 'actual pair launcher exit')
    restored = read_json(root / 'restoration.json')
    require(restored['status'] == 'RESTORED' and restored['files'] == {'toolchain/resident/crates/hir-ty-codegen/mod.rs': dict(sha256=SOURCE_SHA['original'], cmp_original=True, cmp_head=True)} and restored['observed'] == '2026-10-31T06:11:49Z' and restored['released'] == '2026-10-31T06:12:01Z', 'exact scoped source restoration receipt')
    return dict(status='PASS', strict_rows=28, native={'v52': [57, 360], 'v53': [57, 360]}, original_semantic_oracles='PASS', original_analysis='PASS', markers=marker_counts, decision='REJECTED', noise='unknown', debug_finalize_panic_executed=False, latency_claim=False, rows=rows)


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
