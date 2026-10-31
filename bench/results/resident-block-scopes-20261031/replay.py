#!/usr/bin/env python3
"""Offline historical semantic replay, no compiler/engine/network or external paths."""
import argparse
import difflib
import hashlib
import io
import json
import math
import posixpath
from pathlib import Path
import re
import sys
import tarfile
sys.dont_write_bytecode = True
sys.path.insert(0, str(Path(__file__).resolve().parent / 'oracle'))
import resident_state_matrix as matrix

PHASES = ['cold-prepare', 'repeated-noop-1', 'manifest-feature-add', 'type-error', 'type-recovery', 'assert-failure', 'assert-recovery']
CASES = ['block-scopes-v47-baseline', 'block-scopes-v49-candidate', 'block-scopes-v49-candidate-repeat', 'block-scopes-v49-baseline-repeat']
HIR = Path('toolchain/resident/vendor/rust-analyzer/crates/hir-def/src')
RA = Path('toolchain/resident/vendor/rust-analyzer')
CHANGED = ['lib.rs', 'expr_store/body.rs', 'expr_store/lower.rs', 'expr_store/lower/format_args.rs', 'nameres/frozen.rs', 'expr_store/tests/body/block.rs']
FOCUSED = {'nested_macros_and_closures', 'codec_identity_without_body_providers', 'semantic_early_exit_and_signature_traps', 'moves_edits_reparent_and_removal', 'macro_written_and_macro_revision', 'cfg_and_skip_revision_dependencies', 'serving_decode_errors_fail_open', 'verification_decode_errors_remain_strict', 'format_args_visits_before_output', 'const_static_variant_and_inline_module', 'positive_lang_item_gates', 'local_owner_descriptor_is_finite'}

def digest(data):
    return hashlib.sha256(data).hexdigest()

def require(ok, reason):
    if not ok:
        raise ValueError(reason)

def read_json(p):
    return json.loads(p.read_text())

def seal(root):
    files = sorted(p for p in root.rglob('*') if p.is_file() and p.name != 'SHA256SUMS.bundle' and '__pycache__' not in p.parts)
    (root / 'SHA256SUMS.bundle').write_text(''.join(f'{digest(p.read_bytes())}  {p.relative_to(root).as_posix()}\n' for p in files))

def verify_seal(root):
    seen = set()
    for line in (root / 'SHA256SUMS.bundle').read_text().splitlines():
        sha, name = line.split('  ', 1)
        p = Path(name)
        require(not p.is_absolute() and '..' not in p.parts and name not in seen, 'unsafe/duplicate checksum path')
        seen.add(name)
        require(digest((root / p).read_bytes()) == sha, 'bundle checksum: ' + name)
    actual = {p.relative_to(root).as_posix() for p in root.rglob('*') if p.is_file() and p.name != 'SHA256SUMS.bundle' and '__pycache__' not in p.parts}
    require(actual == seen, 'bundle inventory')

def hashes(p):
    return {x.relative_to(p).as_posix(): digest(x.read_bytes()) for x in sorted(p.rglob('*')) if x.is_file()}

def tar_bytes(p):
    result, links = {}, {}
    with tarfile.open(fileobj=io.BytesIO(p.read_bytes()), mode='r:gz') as tar:
        for entry in tar:
            name = Path(entry.name)
            require((entry.isfile() or entry.issym()) and not name.is_absolute() and '..' not in name.parts and entry.name not in result and entry.name not in links, 'safe source archive')
            if entry.issym():
                target = posixpath.normpath(posixpath.join(posixpath.dirname(entry.name), entry.linkname))
                require(not Path(target).is_absolute() and '..' not in Path(target).parts, 'safe internal license symlink')
                links[entry.name] = target
            else:
                result[entry.name] = tar.extractfile(entry).read()
    for name, target in links.items():
        require(target in result, 'symlink target is an archived regular file')
        result[name] = result[target]
    return result

def manifest(p):
    result = {}
    for line in p.read_text().splitlines():
        sha, name = line.split('  ', 1)
        require(name not in result, 'duplicate manifest member')
        result[name] = sha
    return result

def native_result(run):
    text = '\n'.join(json.loads(line)['l'] for line in (run / 'output.jsonl').read_text().splitlines())
    # libtest annotates expected-panic tests between name and delimiter. Preserve the
    # name/status and remove only that exact syntax for the shared strict parser.
    normalized = re.sub(r'^(test [^\s]+) - should panic(?= \.\.\. )', r'\1', text, flags=re.M)
    return matrix.shared.runtime.test_results(normalized), read_json(run / 'meta.json')

def validate(root):
    verify_seal(root)
    frozen = root / 'frozen'
    for v in ('v46', 'v47', 'v49'):
        for kind in ('original', 'candidate'):
            declared = manifest(frozen / v / (kind + '.sha256'))
            require({k.removeprefix('./'): val for k, val in declared.items()} == hashes(frozen / v / kind), 'complete frozen ' + v + ' ' + kind)
    for p in (frozen / 'v46/original').rglob('*'):
        if p.is_file():
            require(p.read_bytes() == (frozen / 'v46/candidate' / p.relative_to(frozen / 'v46/original')).read_bytes(), 'v46 remains no-op Rust investigation')
    v47_frozen = (frozen / 'v47/candidate' / HIR / 'nameres/frozen.rs').read_bytes()
    v49_frozen = (frozen / 'v49/candidate' / HIR / 'nameres/frozen.rs').read_bytes()
    require(v47_frozen.split(b'#[cfg(test)]', 1)[0] == v49_frozen.split(b'#[cfg(test)]', 1)[0], 'v49 production codec unchanged')
    for name in CHANGED:
        p = HIR / name
        require((frozen / 'v49/rejected-live' / p).read_bytes() == (frozen / 'v49/candidate' / p).read_bytes(), 'preserved exact rejected source')
        if name != 'expr_store/tests/body/block.rs':
            require((frozen / 'v47/original' / p).read_bytes() == (frozen / 'v49/original' / p).read_bytes(), 'same accepted source')
        if name not in ('nameres/frozen.rs', 'expr_store/tests/body/block.rs'):
            require((frozen / 'v47/candidate' / p).read_bytes() == (frozen / 'v49/candidate' / p).read_bytes(), 'v49 production unchanged')
    block = HIR / 'expr_store/tests/body/block.rs'
    original = (frozen / 'v49/original' / block).read_bytes()
    repaired = (frozen / 'v49/candidate' / block).read_bytes()
    require(original.replace(b'Id(881)', b'Id(901)', 1) == repaired, 'single observed numeric snapshot repair')
    candidate_frozen = (frozen / 'v49/candidate' / HIR / 'nameres/frozen.rs').read_text()
    for exact in ('Body::block_scopes_', 'Body::of_', 'Body::relative_', 'Body::with_source_map_', '#![feature(lang_items)]', 'resident_materializing_provider_count(&control_events)'):
        require(exact in candidate_frozen, 'strict fixture source control ' + exact)
    snapshots = {}
    for version, fv in [('v48', 'v47'), ('v49', 'v49')]:
        native = root / 'native' / ('snapshot-' + version)
        before = read_json(native / 'source-copy-before.json')
        require(len(before['original']) == len(before['copy']) == len(before['selected']) == 2444, 'all2444 snapshot members')
        require(before['original'] == before['copy'], 'byte-identical whole source copy')
        require(before['original']['Cargo.lock'] == digest((root / 'provenance/original-vendor-Cargo.lock').read_bytes()), 'exact original vendor lock')
        require(set(before['original']) == set(before['selected']), 'private snapshot inventory')
        require([k for k in before['original'] if before['original'][k] != before['selected'][k]] == ['Cargo.lock'], 'only private lock differs')
        files = tar_bytes(root / 'snapshots' / ('rust-analyzer-' + version + '.tar.gz'))
        require({k: digest(v) for k, v in files.items()} == before['selected'], 'actual2444 archive matches selected native source')
        snapshots[version] = files
        for p in (frozen / fv / 'candidate' / RA).rglob('*'):
            if p.is_file():
                name = p.relative_to(frozen / fv / 'candidate' / RA).as_posix()
                require(files[name] == p.read_bytes(), 'native snapshot matches frozen candidate: ' + name)
        if version == 'v49':
            after = read_json(native / 'source-copy-after.json')
            require(after == dict(original=before['original'], selected=before['selected'], unchanged=True), 'full snapshot after join')
            for name in ('block-scopes-native.exit', 'full-hir-def-native.exit', 'exit', 'orchestration-exit'):
                require((native / name).read_text().strip() == '0', 'v49 snapshot gate ' + name)
            results = {}
            runs = sorted((native / 'runs').iterdir())
            require(len(runs) == 2, 'exactly focused and full native snapshot runs')
            for run in runs:
                parsed, meta = native_result(run)
                require(parsed is not None, 'native raw test summary')
                require('--locked' in meta['args'] and '--offline' in meta['args'] and 'hir-def' in meta['args'], 'actual snapshot command')
                results[parsed['passed']] = parsed
            require(set(results) == {12, 545}, 'v49 focused/full raw journal inventory')
            for count, ignored, filtered in [(12, 0, 534), (545, 1, 0)]:
                parsed = results[count]
                require((parsed['failed'], parsed['ignored'], parsed['filtered']) == (0, ignored, filtered), 'v49 raw native strict totals')
                observed = {n.split('resident_block_scopes_', 1)[1] for n, status in parsed['tests'] if 'resident_block_scopes_' in n and status == 'ok'}
                require(observed == FOCUSED, 'all12 exact native fixture identities')
        else:
            results = sorted((p['passed'], p['failed'], p['ignored']) for p, _ in [native_result(run) for run in (native / 'runs').iterdir()])
            require(results == [(8, 4, 0), (540, 5, 1)], 'retain actual v48 focused/full failures')
    require(snapshots['v48']['Cargo.lock'] == snapshots['v49']['Cargo.lock'] == (root / 'provenance/private-snapshot-Cargo.lock').read_bytes(), 'same established private lock')
    lock_delta = ''.join(difflib.unified_diff((root / 'provenance/original-vendor-Cargo.lock').read_text().splitlines(True), snapshots['v49']['Cargo.lock'].decode().splitlines(True), fromfile='original-vendor-Cargo.lock', tofile='private-snapshot-Cargo.lock'))
    require((root / 'provenance/private-lock.patch').read_text() == lock_delta, 'exact private lock delta')
    excluded = root / 'native/candidate-build-v47'
    require((excluded / 'check.exit').read_text().strip() == '101', 'retain excluded-package failure')
    excluded_text = (excluded / 'check.log').read_text()
    require(all(x in excluded_text for x in ('expect_test', 'test_fixture', 'test_utils', '103 previous errors')), 'retain actual excluded devdep diagnostics')
    build = root / 'native/candidate-build-v49'
    unit = root / 'native/candidate-unit-v49'
    for directory, names in [(build, ['check.exit', 'compile.exit', 'orchestration-exit']), (unit, ['codegen-native.exit', 'full-native.exit', 'exit'])]:
        for name in names:
            require((directory / name).read_text().strip() == '0', 'production native gate ' + name)
    source = manifest(build / 'all-source-before.sha256')
    require(len(source) == 3757, 'full3757 native source manifest')
    for p in (frozen / 'v49/candidate/toolchain/resident').rglob('*'):
        if p.is_file():
            rel = p.relative_to(frozen / 'v49/candidate/toolchain/resident').as_posix()
            require(source[rel] == digest(p.read_bytes()), 'production source equals frozen candidate')
    after_lines = (unit / 'all-source-after.log').read_text().splitlines()
    require(after_lines == [name + ': OK' for name in source], 'all3757 after checks')
    lock = (root / 'provenance/production-Cargo.lock').read_bytes()
    require(next(iter(manifest(build / 'lock-before.sha256').values())) == digest(lock), 'unchanged actual production lock')
    require((unit / 'lock-after.log').read_text().strip() == 'Cargo.lock: OK', 'production lock after check')
    counts = []
    for run in (unit / 'runs').iterdir():
        parsed, meta = native_result(run)
        require(parsed is not None and (parsed['failed'], parsed['ignored'], parsed['filtered']) == (0, 0, 0), 'full native production strict status')
        require(meta['args'][:3] == ['test', '--release', '--locked'], 'native production command')
        counts.append(parsed['passed'])
    require(sorted(counts) == [53, 360], 'actual production53/360 raw suites')
    candidate_engine = (build / 'engine-identity.sha256').read_text().split()[0]
    baseline_engine = (root / 'native/candidate-build-v43/engine-identity.sha256').read_text().split()[0]
    pinned = tar_bytes(root / 'snapshots/source63.tar.gz')
    source_identity = toolchain_identity = None
    all_rows = []
    timings = {}
    for case in CASES:
        directory = root / 'raw' / case
        state = read_json(directory / 'state.json')
        engine = candidate_engine if 'candidate' in case else baseline_engine
        require(state['status'] == 'passed' and state['unchanged'], 'state completion')
        require(state['startup_controls'] == dict(RA_JIT_DEMAND_STD='1', RA_JIT_CACHE_MISS_STATS='0', RA_JIT_STATS='1'), 'exact startup controls')
        require(state['source_before'] == state['source_after'], 'source restoration per state')
        require(state['tool_before'] == state['tool_after'] == engine, 'engine build/run identity')
        require(state['toolchain_before'] == state['toolchain_after'], 'toolchain before/after')
        if source_identity is None:
            source_identity, toolchain_identity = state['source_before'], state['toolchain_before']
        require(state['source_before'] == source_identity and len(source_identity) == 63, 'same pinned63 source')
        require(state['toolchain_before'] == toolchain_identity, 'same complete toolchain4020')
        require(len(toolchain_identity['files']) == 4020 and toolchain_identity['files']['lib/rustlib/multirust-channel-manifest.toml']['sha256'] == matrix.shared.TOOLCHAIN_MANIFEST, 'pinned toolchain manifest')
        require({name: digest(pinned[name]) for name in source_identity} == source_identity, 'actual63 source archive')
        require([row['phase'] for row in state['rows']] == PHASES, 'seven phases in order')
        originals = {name: (directory / f'original-{i}').read_bytes() for i, name in enumerate(matrix.FILES)}
        for name, data in originals.items():
            require(digest(data) == source_identity[name], 'exact input backup')
        contract = None
        timings[case] = []
        for i, row in enumerate(state['rows']):
            phase = row['phase']
            require(re.fullmatch('[0-9a-f]{32}', row['nonce']) is not None, 'nonce shape')
            require(row['accepted'] and not row['failures'] and row['input_unchanged'], 'recorded oracle completion')
            require(math.isfinite(row['elapsed_seconds']) and row['elapsed_seconds'] > 0, 'finite positive elapsed')
            require(row['noise']['contamination'] == 'unknown', 'retain unknown noise')
            require(row['command'][1:] == ['test', *matrix.arguments(phase)], 'command phase contract')
            wanted = dict(source_identity)
            for name, data in matrix.inputs(originals, phase, 0, row['nonce']).items():
                wanted[name] = digest(data)
            require(row['inputs'] == wanted, 'actual exact source input recipe')
            streams = []
            for suffix in ('stdout', 'stderr'):
                data = (directory / f'{i:02}-{phase}.{suffix}').read_bytes()
                require(len(data) <= matrix.MAX_RAW_BYTES, 'bounded raw parser')
                receipt = row['raw'][suffix]
                require(len(data) == receipt['bytes'] and digest(data) == receipt['sha256'] and Path(receipt['path']).name == f'{i:02}-{phase}.{suffix}', 'raw receipt')
                streams.append(data.decode(errors='replace'))
            text = '\n'.join(streams)
            parsed = matrix.shared.runtime.test_results(text)
            require(json.loads(json.dumps(parsed)) == row['parsed'], 'parsed exact raw replay')
            if i == 0:
                contract = matrix.contract(parsed, phase)
                require(contract is not None, 'exact cold runtime test contract')
            failures = matrix.oracle(dict(row, parsed=parsed), text, contract, phase, row['nonce'], 0)
            require(not failures, 'original actual semantic oracle: ' + repr(failures))
            require(len(list(directory.glob(f'{i:02}-{phase}-worker-*.log'))) == 1, 'actual cumulative worker journal')
            timings[case].append(row['elapsed_seconds'])
            all_rows.append(dict(case=case, **row))
    require(all_rows == read_json(root / 'rows28.json'), 'coherent index of all28 raw rows')
    journal = read_json(root / 'provenance/block-scopes-v49-validation.json')
    require(journal['decision'] == 'REJECTED' and journal['promotion'] is False and journal['noise'] == 'unknown' and journal['raw_rows'] == 28, 'retain rejection/noise/promotion boundary')
    comparisons = []
    for i, phase in enumerate(PHASES):
        seconds = {case: timings[case][i] for case in CASES}
        comparisons.append(dict(phase=phase, seconds=seconds, first_candidate_relative_percent=100 * (seconds[CASES[1]] / seconds[CASES[0]] - 1), reverse_candidate_relative_percent=100 * (seconds[CASES[2]] / seconds[CASES[3]] - 1)))
    require(comparisons == journal['comparisons'], 'exact timing percentage recomputation')
    require((root / 'provenance/block-scopes-v49-pairs.exit').read_text().strip() == '0', 'pair orchestration status')
    restoration = read_json(root / 'restoration.json')
    require(restoration['status'] == 'RESTORED' and set(restoration['files']) == {(HIR / name).as_posix() for name in CHANGED}, 'all6 restored source receipt')
    for name, receipt in restoration['files'].items():
        require(receipt == dict(sha256=digest((frozen / 'v49/original' / name).read_bytes()), cmp_original=True, cmp_head=True), 'exact original restoration')
    return dict(status='PASS', decision='REJECTED', promotion=False, noise='unknown', raw_rows=28, native=dict(focused=12, hir_def_passed=545, hir_def_ignored=1, codegen=53, resident=360), failures_retained=dict(excluded_errors=103, v48_focused=[8, 4], v48_full=[540, 5, 1]), source_restored=True, comparisons=comparisons)

if __name__ == '__main__':
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--root', type=Path, default=Path(__file__).resolve().parent)
    parser.add_argument('--seal', action='store_true')
    args = parser.parse_args()
    if args.seal:
        seal(args.root)
    print(json.dumps(validate(args.root), indent=2))
