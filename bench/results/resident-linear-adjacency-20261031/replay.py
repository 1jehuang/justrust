#!/usr/bin/env python3
"""Portable offline semantic replay of actual native adjacency evidence, never native execution."""
import argparse
import collections
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
CASES = ['linear-adjacency-v50-' + x for x in ('baseline', 'candidate', 'candidate-repeat', 'baseline-repeat')] + ['linear-adjacency-v51-diagnostic']
FEATURE = 'hir-ty-codegen/native-adjacency-diagnostic'
PATTERN = re.compile(r'native-adjacency-dual-v51 timings_invalid=true selector_calls=(\d+) yielded=(\d+) visits=(\d+) raw_lengths=(\d+) max_raw_length=(\d+) fanout=\[([0-9, ]+)\] exhausted=(\d+) aborted=(\d+) candidate_inspections=(\d+) original_inspections=(\d+)')

def digest(data):
    return hashlib.sha256(data).hexdigest()

def require(ok, reason):
    if not ok:
        raise ValueError(reason)

def read_json(path):
    return json.loads(path.read_text())

def seal(root):
    files = sorted(p for p in root.rglob('*') if p.is_file() and p.name != 'SHA256SUMS.bundle' and '__pycache__' not in p.parts)
    (root / 'SHA256SUMS.bundle').write_text(''.join(f'{digest(p.read_bytes())}  {p.relative_to(root).as_posix()}\n' for p in files))

def verify_seal(root):
    seen = set()
    for line in (root / 'SHA256SUMS.bundle').read_text().splitlines():
        sha, name = line.split('  ', 1)
        require(not Path(name).is_absolute() and '..' not in Path(name).parts and name not in seen, 'safe checksum path')
        seen.add(name)
        require(digest((root / name).read_bytes()) == sha, 'transport checksum ' + name)
    require(seen == {p.relative_to(root).as_posix() for p in root.rglob('*') if p.is_file() and p.name != 'SHA256SUMS.bundle' and '__pycache__' not in p.parts}, 'complete bundle inventory')

def manifest(path):
    result = {}
    for line in path.read_text().splitlines():
        sha, name = line.split('  ', 1)
        require(name not in result, 'duplicate manifest name')
        result[name] = sha
    return result

def native_result(run):
    text = '\n'.join(json.loads(line)['l'] for line in (run / 'output.jsonl').read_text().splitlines())
    text = re.sub(r'^(test [^\s]+) - should panic(?= \.\.\. )', r'\1', text, flags=re.M)
    return matrix.shared.runtime.test_results(text), read_json(run / 'meta.json')

def snapshots(root):
    directory = root / 'raw' / CASES[-1]
    state = read_json(directory / 'state.json')
    previous = collections.Counter()
    rows = []
    for index, row in enumerate(state['rows']):
        current = collections.Counter()
        for log in sorted(directory.glob(f'{index:02}-{row["phase"]}-worker-*.log')):
            for line in log.read_text().splitlines():
                if 'native-adjacency-dual-v51' in line:
                    match = PATTERN.search(line)
                    require(match is not None, 'well-formed dual selector marker')
                    current[match.group(0)] += 1
        metrics = collections.Counter()
        hist = None
        for text, count in current.items():
            match = PATTERN.fullmatch(text)
            calls, yielded, visits, lengths, maximum = map(int, match.groups()[:5])
            fanout = list(map(int, match.group(6).split(',')))
            exhausted, aborted, candidate, original = map(int, match.groups()[6:])
            require(len(fanout) == 8 and visits > 0 and sum(fanout) == visits, 'raw fanout histogram')
            require(exhausted + aborted == visits and calls == yielded + exhausted, 'selector conservation')
            require(candidate <= lengths and candidate <= original, 'actual dual selector scan bounds')
            hist = [count * x + y for x, y in zip(fanout, hist or [0] * 8)]
            for key, value in dict(selector_calls=calls, yielded=yielded, visits=visits, raw_lengths=lengths, exhausted=exhausted, aborted=aborted, candidate_inspections=candidate, original_inspections=original).items():
                metrics[key] += count * value
            metrics['max_raw_length'] = max(metrics['max_raw_length'], maximum)
        rows.append(dict(captured_after=row['phase'], report_count=sum(current.values()), snapshot_metrics=dict(metrics), worker_snapshot_reset_or_replaced=bool(previous-current), repeats_previous_visible_report_counts=current == previous, fanout=hist, original_over_candidate=metrics['original_inspections']/metrics['candidate_inspections'] if metrics['candidate_inspections'] else None))
        previous = current
    require(any(x['worker_snapshot_reset_or_replaced'] for x in rows), 'preserve actual failed delta assumption')
    require(rows[4]['snapshot_metrics'] == rows[5]['snapshot_metrics'] == rows[6]['snapshot_metrics'], 'assertion snapshots repeat recovery counts, not fresh work')
    expected = read_json(root / 'provenance/linear-adjacency-v51-snapshot-validation.json')
    require(expected['status'] == 'passed' and expected['native_rows'] == 7 and expected['reports_in_final_snapshot'] == sum(previous.values()), 'snapshot validation totals')
    require(rows == expected['rows'], 'exact corrected snapshot replay')
    require('do not sum snapshots or infer fresh phase work' in expected['warning'] and 'not fresh phase attribution' in expected['scope'], 'snapshot attribution boundary')
    require((root / 'provenance/linear-adjacency-v51-validation.json').read_bytes() == b'', 'retain failed validator empty output')
    require("assert not (previous-current)" in (root / 'provenance/validate-linear-adjacency-v51.py').read_text(), 'retain failed original delta validator')
    return rows

def analysis_replay(root):
    # Preserve the original code unchanged. A local synthetic __file__ only binds
    # its ROOT to this relocated bundle's raw directory, never historical paths.
    result = {}
    for name in ('validate-linear-adjacency-v51.py', 'validate-linear-adjacency-v51-snapshots.py'):
        source = (root / 'provenance' / name).read_text()
        output = io.StringIO()
        failure = None
        try:
            with contextlib.redirect_stdout(output):
                exec(compile(source, name, 'exec'), {'__file__': str(root / 'raw' / name), '__name__': '__main__'})
        except AssertionError as error:
            failure = str(error)
        if name == 'validate-linear-adjacency-v51.py':
            require(failure == 'worker report snapshots reset or lost, cannot attribute phase deltas' and output.getvalue() == '', 'original failed delta analyzer reproduced')
            result['failed_delta'] = failure
        else:
            require(failure is None and json.loads(output.getvalue()) == read_json(root / 'provenance/linear-adjacency-v51-snapshot-validation.json'), 'original corrected snapshot analyzer reproduced')
            result['corrected_snapshot'] = 'PASS'
    return result

def validate(root):
    verify_seal(root)
    decision = read_json(root / 'decision.json')
    require(decision == dict(decision='REJECTED', promotion=False, noise='unknown', latency_claim=False, fresh_phase_attribution=False, diagnostic_timings_valid=False, snapshot_counts_overlap_or_reset=True, strict_timing_rows=28, diagnostic_rows=7), 'rejection and no latency/fresh attribution boundary')
    for version in ('v50', 'v51'):
        directory = root / 'frozen' / version
        for name, sha in manifest(directory / 'SHA256SUMS').items():
            require(digest((directory / name).read_bytes()) == sha, 'frozen manifest ' + version)
    frozen = root / 'frozen'
    require((frozen / 'v51/original/cached.rs').read_bytes() == (frozen / 'v50/candidate.rs').read_bytes(), 'v51 based on exact v50')
    for name in ('cached.rs', 'Cargo.toml', 'resident-linear-adjacency.md'):
        require((frozen / 'v51/live-before-restoration' / name).read_bytes() == (frozen / 'v51/candidate' / name).read_bytes(), 'preserved exact rejected v51 ' + name)
    source_map = {'v43': {}, 'v50': {'crates/hir-ty-codegen/cached.rs': frozen / 'v50/candidate.rs'}, 'v51': {'crates/hir-ty-codegen/cached.rs': frozen / 'v51/candidate/cached.rs', 'crates/hir-ty-codegen/Cargo.toml': frozen / 'v51/candidate/Cargo.toml'}}
    engines = {}
    lock = digest((root / 'provenance/production-Cargo.lock').read_bytes())
    native = {}
    sources = {}
    for version, count in [('v43', 53), ('v50', 54), ('v51', 55)]:
        build = root / 'native' / ('candidate-build-' + version)
        unit = root / 'native' / ('candidate-unit-' + version)
        for directory, files in [(build, ['check.exit', 'compile.exit', 'orchestration-exit']), (unit, ['codegen-native.exit', 'full-native.exit', 'exit'])]:
            for name in files:
                require((directory / name).read_text().strip() == '0', 'native gate exit ' + version + name)
        source = manifest(build / 'all-source-before.sha256')
        sources[version] = source
        require(len(source) == 3757, 'whole3757 native source manifest')
        for name, path in source_map[version].items():
            require(source[name] == digest(path.read_bytes()), 'native exact frozen source ' + version + name)
        require((unit / 'all-source-after.log').read_text().splitlines() == [name + ': OK' for name in source], 'whole3757 source after checks')
        require(list(manifest(build / 'lock-before.sha256').values()) == [lock], 'native original lock')
        require((unit / 'lock-after.log').read_text().strip() == 'Cargo.lock: OK', 'native lock after check')
        for run in (build / 'runs').iterdir():
            meta = read_json(run / 'meta.json')
            require((FEATURE in meta['args']) == (version == 'v51'), 'explicit native diagnostic build feature')
        counts = []
        for run in (unit / 'runs').iterdir():
            parsed, meta = native_result(run)
            require(parsed is not None and (parsed['failed'], parsed['ignored'], parsed['filtered']) == (0, 0, 0), 'native raw full totals')
            require(meta['args'][:3] == ['test', '--release', '--locked'] and (FEATURE in meta['args']) == (version == 'v51'), 'explicit native unit controls')
            if parsed['passed'] == count and version != 'v43':
                names = {name for name, status in parsed['tests'] if status == 'ok'}
                require('cached::tests::native_linear_adjacency_matches_original_dfs' in names, 'exact original differential fixture passed')
                if version == 'v51':
                    require('cached::tests::native_adjacency_diagnostic_counts_actual_shadow_inspections' in names, 'exact shadow inspection fixture passed')
            counts.append(parsed['passed'])
        require(sorted(counts) == [count, 360], 'actual native codegen/resident totals ' + version)
        native[version] = sorted(counts)
        engines[version] = (build / 'engine-identity.sha256').read_text().split()[0]
        require('resident-v1-' in (build / 'engine-version.txt').read_text(), 'versioned native build identity')
    require(len(set(engines.values())) == 3, 'distinct actual original/default/diagnostic binaries')
    for before, after, changed in [('v43', 'v50', {'crates/hir-ty-codegen/cached.rs'}), ('v50', 'v51', {'crates/hir-ty-codegen/cached.rs', 'crates/hir-ty-codegen/Cargo.toml'})]:
        require(sources[before].keys() == sources[after].keys() and {name for name in sources[before] if sources[before][name] != sources[after][name]} == changed, 'exact whole-source delta ' + after)
    require(sources['v43']['crates/hir-ty-codegen/cached.rs'] == digest((frozen / 'v50/original.rs').read_bytes()), 'original accepted native source join')
    pinned = {}
    with tarfile.open(fileobj=io.BytesIO((root / 'snapshots/source63.tar.gz').read_bytes()), mode='r:gz') as tar:
        for entry in tar:
            require(entry.isfile() and not Path(entry.name).is_absolute() and '..' not in Path(entry.name).parts and entry.name not in pinned, 'safe pinned source archive')
            pinned[entry.name] = tar.extractfile(entry).read()
    source_identity = toolchain_identity = None
    all_rows = []
    timing = {}
    for case in CASES:
        directory = root / 'raw' / case
        state = read_json(directory / 'state.json')
        version = 'v51' if case == CASES[-1] else 'v50' if 'candidate' in case else 'v43'
        require(state['status'] == 'passed' and state['unchanged'], 'actual state completion')
        require(state['startup_controls'] == dict(RA_JIT_DEMAND_STD='1', RA_JIT_CACHE_MISS_STATS='0', RA_JIT_STATS='1'), 'runtime startup controls')
        require(state['source_before'] == state['source_after'], 'runtime source restoration')
        require(state['tool_before'] == state['tool_after'] == engines[version], 'native build/runtime engine join')
        require(state['toolchain_before'] == state['toolchain_after'], 'runtime toolchain unchanged')
        if source_identity is None:
            source_identity, toolchain_identity = state['source_before'], state['toolchain_before']
        require(state['source_before'] == source_identity and len(source_identity) == 63, 'same pinned63 source')
        require(state['toolchain_before'] == toolchain_identity, 'same whole toolchain identity')
        require(len(toolchain_identity['files']) == 4020 and toolchain_identity['files']['lib/rustlib/multirust-channel-manifest.toml']['sha256'] == matrix.shared.TOOLCHAIN_MANIFEST, 'pinned actual toolchain manifest')
        require({name: digest(pinned[name]) for name in source_identity} == source_identity, 'exact archived workload source')
        require([row['phase'] for row in state['rows']] == PHASES, 'exact phase order')
        originals = {name: (directory / f'original-{i}').read_bytes() for i, name in enumerate(matrix.FILES)}
        require(all(digest(data) == source_identity[name] for name, data in originals.items()), 'actual input backups')
        contract = None
        timing[case] = []
        for i, row in enumerate(state['rows']):
            phase = row['phase']
            require(re.fullmatch('[0-9a-f]{32}', row['nonce']) is not None, 'nonce shape')
            require(row['accepted'] and not row['failures'] and row['input_unchanged'], 'recorded oracle status')
            require(math.isfinite(row['elapsed_seconds']) and row['elapsed_seconds'] > 0, 'finite elapsed')
            require(row['noise']['contamination'] == 'unknown', 'unknown noise retained')
            require(row['command'][1:] == ['test', *matrix.arguments(phase)], 'actual command contract')
            wanted = dict(source_identity)
            for name, data in matrix.inputs(originals, phase, 0, row['nonce']).items():
                wanted[name] = digest(data)
            require(row['inputs'] == wanted, 'actual source edit recipe')
            streams = []
            for suffix in ('stdout', 'stderr'):
                name = f'{i:02}-{phase}.{suffix}'
                data = (directory / name).read_bytes()
                receipt = row['raw'][suffix]
                require(len(data) <= matrix.MAX_RAW_BYTES and len(data) == receipt['bytes'] and digest(data) == receipt['sha256'] and Path(receipt['path']).name == name, 'actual raw byte receipt')
                streams.append(data.decode(errors='replace'))
            text = '\n'.join(streams)
            parsed = matrix.shared.runtime.test_results(text)
            require(json.loads(json.dumps(parsed)) == row['parsed'], 'exact parsed raw replay')
            if i == 0:
                contract = matrix.contract(parsed, phase)
                require(contract is not None, 'cold exact test contract')
            failures = matrix.oracle(dict(row, parsed=parsed), text, contract, phase, row['nonce'], 0)
            require(not failures, 'original actual semantic oracle: ' + repr(failures))
            require(len(list(directory.glob(f'{i:02}-{phase}-worker-*.log'))) == 1, 'retained worker snapshot')
            timing[case].append(row['elapsed_seconds'])
            all_rows.append(dict(case=case, **row))
    require(all_rows == read_json(root / 'rows35.json'), 'coherent35 raw index')
    expected_timing = read_json(root / 'provenance/linear-adjacency-v50-validation.json')
    comparisons = []
    for i, phase in enumerate(PHASES):
        values = [timing[case][i] for case in CASES[:4]]
        comparisons.append(dict(phase=phase, seconds=dict(zip(('baseline', 'candidate', 'candidate-repeat', 'baseline-repeat'), values)), first_percent=100*(values[1]/values[0]-1), reverse_percent=100*(values[2]/values[3]-1)))
    require(expected_timing['status'] == 'passed' and expected_timing['strict_rows'] == 28 and comparisons == expected_timing['rows'], 'actual timing recomputation')
    require((root / 'provenance/linear-adjacency-v50-pairs.exit').read_text().strip() == '0', 'completed pair launcher')
    snapshot_rows = snapshots(root)
    analysis = analysis_replay(root)
    failure_receipt = read_json(root / 'provenance/initial-analysis-failure.json')
    require(failure_receipt['status'] == 'ORIGINAL_FAILURE_RETAINED' and failure_receipt['original_output_bytes'] == 0 and failure_receipt['failure'] == analysis['failed_delta'] and failure_receipt['diagnostic_timings_valid'] is False, 'original failure provenance retained')
    restoration = read_json(root / 'restoration.json')
    names = {'cached.rs': frozen / 'v50/original.rs', 'Cargo.toml': frozen / 'v51/original/Cargo.toml'}
    require(restoration['status'] == 'RESTORED' and set(restoration['files']) == {'toolchain/resident/crates/hir-ty-codegen/' + name for name in names}, 'only two restored production paths')
    for name, original in names.items():
        require(restoration['files']['toolchain/resident/crates/hir-ty-codegen/' + name] == dict(sha256=digest(original.read_bytes()), cmp_original=True, cmp_head=True), 'exact source restoration receipt')
    cleanup = read_json(root / 'cleanup/receipt.json')
    require(cleanup['stores_deleted'] is False and len(cleanup['rows']) == 8, 'post-measurement cleanup preserves stores')
    for row in cleanup['rows']:
        require(row['post_measurement_only'] and matrix.shared.runtime.base.succeeded(row), 'completed private stop receipt')
        require(row['command'][1:] == ['stop'], 'cleanup only stop command')
        for suffix in ('stdout', 'stderr'):
            require((root / 'cleanup' / (row['case'] + '.' + suffix)).is_file(), 'cleanup raw output retained')
    return dict(status='PASS', decision='REJECTED', promotion=False, latency_claim=False, noise='unknown', raw_rows=35, strict_timing_rows=28, diagnostic_rows=7, native=native, source_restored=True, diagnostic_timings_valid=False, fresh_phase_attribution=False, analysis_replay=analysis, snapshot_rows=snapshot_rows, comparisons=comparisons)

if __name__ == '__main__':
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--root', type=Path, default=Path(__file__).resolve().parent)
    parser.add_argument('--seal', action='store_true')
    args = parser.parse_args()
    if args.seal:
        seal(args.root)
    print(json.dumps(validate(args.root), indent=2))
