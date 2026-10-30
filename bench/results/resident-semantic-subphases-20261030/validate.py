#!/usr/bin/env python3
"""Relocatable offline receipt replay. Never imports or executes workload runners."""
import argparse
from collections import Counter, defaultdict
import hashlib
import json
from pathlib import Path
import re
import statistics

EXCLUDED = {'SHA256SUMS', 'validation.json', 'mutation-controls.json', 'audit.json'}
LABELS = ['diagnostics.semantic.' + name for name in (
    'prepare_parse', 'syntax_walk', 'module_context', 'collect', 'render_and_filter')]
PREFIX = 'toolchain/resident/'
LIB = PREFIX + 'vendor/rust-analyzer/crates/ide-diagnostics/src/lib.rs'
TESTS = PREFIX + 'vendor/rust-analyzer/crates/ide-diagnostics/src/tests.rs'
MAIN = PREFIX + 'crates/ra-jit/src/main.rs'
ACCEPTED = {
    MAIN: '68a9b0625158dcc201da88c5c0777cf9a2eb193d6053089ef7c1b9bf375a51a0',
    LIB: '7af1b6eb7fc616d3230a1970673165da1d9a724af72666a4b3e8d721930176b0',
    TESTS: '75695280c0733b18166d35b8acf1b943dab2d218beb6bd46cead08fde545c770',
    'docs/resident-semantic-subphases.md': '419e738d0fab78976cc09415c80b57fb564c57704f6257aba07e4817edd47a1c',
}
NATIVE = {
    '20261030-133307142-3479390': ('resident_semantic_subphases', 5),
    '20261030-133436214-3485583': ('diagnostic_hash::tests', 2),
    '20261030-133436486-3485632': ('diagnostics_timing_tests', 3),
    '20261030-133436701-3485707': ('resident_graph_reuse', 1),
    '20261030-133436923-3485756': ('the_roots_of_a_listed_crate', 1),
    '20261030-133437181-3485842': ('telemetry', 3),
    '20261030-133629708-3652589': (None, 351),
}
API_NAMES = ['tests::resident_semantic_subphases_' + name for name in (
    'all_bodies_exact', 'no_bodies_exact', 'selective_bodies_exact',
    'disabled_diagnostic_exact', 'disabled_clockfree_api_exact')]
RUNTIME_NAMES = ['summary::tests::' + name for name in (
    'parses_libtest_results', 'shares_overlapping_time',
    'splits_passes_without_double_counting', 'test_failures_are_not_compile_errors')]


def require(value, message):
    if not value:
        raise ValueError(message)


def sha(path):
    return hashlib.sha256(path.read_bytes()).hexdigest()


def read(root, path):
    return json.loads((root / path).read_text())


def files(root):
    return sorted(p for p in root.rglob('*') if p.is_file()
                  and '__pycache__' not in p.parts and p.name not in EXCLUDED)


def sign(root):
    (root / 'SHA256SUMS').write_text(''.join(
        f'{sha(p)}  {p.relative_to(root).as_posix()}\n' for p in files(root)))


def manifest(root, path):
    result = {}
    for line in (root / path).read_text().splitlines():
        digest, name = line.split('  ', 1)
        name = name.removeprefix('./')
        require(not Path(name).is_absolute() and '..' not in Path(name).parts, 'unsafe manifest path')
        require(name not in result, 'duplicate manifest entry')
        result[name] = digest
    return result


def replay_patch(root):
    """Apply retained unified hunks in memory, with exact old context verification."""
    patch = (root / 'source-ready/four-paths.patch').read_text().splitlines(keepends=True)
    current = None
    chunks = {}
    index = 0
    while index < len(patch):
        line = patch[index]
        if line.startswith('diff --git '):
            current = line.rstrip().split(' b/', 1)[1]
            chunks[current] = []
        if line.startswith('@@ '):
            match = re.match(r'@@ -(\d+)(?:,(\d+))? \+(\d+)(?:,(\d+))? @@', line)
            require(match is not None and current is not None, 'invalid patch hunk')
            old_start, old_count, _, new_count = match.groups()
            old, new = [], []
            index += 1
            while index < len(patch) and patch[index][:1] in (' ', '+', '-'):
                part = patch[index]
                if part[0] in ' -':
                    old.append(part[1:])
                if part[0] in ' +':
                    new.append(part[1:])
                index += 1
            require(len(old) == int(old_count or 1) and len(new) == int(new_count or 1), 'patch hunk lengths')
            chunks[current].append((int(old_start), old, new))
            continue
        index += 1
    require(set(chunks) == set(ACCEPTED), 'patch not exactly four assigned paths')
    for name, hunks in chunks.items():
        original = root / 'source-ready/originals' / name
        lines = original.read_text().splitlines(keepends=True) if original.exists() else []
        offset = 0
        for start, old, new in hunks:
            pos = max(0, start - 1) + offset
            require(lines[pos:pos + len(old)] == old, f'patch original context: {name}')
            lines[pos:pos + len(old)] = new
            offset += len(new) - len(old)
        require(''.join(lines) == (root / 'source-ready/frozen' / name).read_text(), f'patch replay mismatch: {name}')


def source_checks(root):
    for name, digest in ACCEPTED.items():
        require(sha(root / 'source-ready/frozen' / name) == digest, f'accepted frozen source anchor: {name}')
    require(sha(root / 'source-ready/four-paths.patch') ==
            'e97c7c63f5c7819c60d324d2236804c909109ee8ba7d32c84003c26d7ce59955', 'four-file patch anchor')
    replay_patch(root)
    for kind in ('frozen', 'originals'):
        for name, digest in manifest(root, f'source-ready/{kind}.sha256').items():
            require(sha(root / f'source-ready/{kind}' / name) == digest, 'handoff manifest mismatch')
    build_manifest = manifest(root, 'build/v25/source-before.sha256')
    for name, digest in build_manifest.items():
        require(sha(root / 'build/v25/frozen-source' / name) == digest, 'build frozen source mismatch')
    for name in (MAIN, LIB, TESTS):
        require(build_manifest[name.removeprefix(PREFIX)] == ACCEPTED[name], 'build/owned source disagreement')
    for name in ('check-exit', 'compile-exit', 'exit', 'orchestration-exit'):
        require((root / 'build/v25' / name).read_text().strip() == '0', 'original resident build failed')
    before = read(root, 'native/v25-snapshot/source-copy-before.json')
    after = read(root, 'native/v25-snapshot/source-copy-after.json')
    require(len(before['original']) == len(before['copy']) == len(before['selected']) == 2444, 'not exact2444 snapshot')
    require(before['original'] == before['copy'], 'initial copy differs from original')
    require(set(before['original']) == set(before['selected']), 'private snapshot lost files')
    require([k for k in before['original'] if before['original'][k] != before['selected'][k]] == ['Cargo.lock'], 'private delta not lock-only')
    require(after['original'] == before['original'] and after['copy'] == before['selected'], 'sources/lock changed during native gates')
    require(after['original_unchanged'] and after['private_resolved_lock_unchanged'], 'snapshot unchanged flags false')
    for name in (LIB, TESTS):
        key = name.split('vendor/rust-analyzer/', 1)[1]
        require(before['selected'][key] == ACCEPTED[name], 'native snapshot not accepted owned source')
    for label, key in (('original', 'original'), ('private', 'selected')):
        require(sha(root / f'snapshot/Cargo.lock.{label}') == before[key]['Cargo.lock'], 'retained lock mismatch')
    for receipt in ('build/v25/source-after.log', 'native/v25-snapshot/source-before.log',
                    'native/v25-snapshot/source-after.log', 'native/v25-snapshot/full-native-source-after.log'):
        lines = (root / receipt).read_text().splitlines()
        require(lines == [f'{name}: OK' for name in build_manifest], 'selected build source integrity receipt mismatch')
    return dict(files=2444, private_delta='Cargo.lock', four_path_patch_replayed=True)


def native_checks(root):
    for run, command in (('20261030-133105097-3471090', 'check'), ('20261030-133115051-3471864', 'build')):
        summary = read(root, f'journals/build-v25/{run}/summary.json')
        meta = read(root, f'journals/build-v25/{run}/meta.json')
        require(summary['id'] == meta['id'] == run and summary['args'] == meta['args'], 'build journal identity/args mismatch')
        require(summary['exit'] == 0 and summary['args'][0] == command and 'ra-jit' in summary['args'], 'original native build/check journal failure')
    directory = root / 'journals/v25-snapshot'
    require({p.name for p in directory.iterdir()} == set(NATIVE), 'native run set differs')
    rows = []
    for run, (filter_name, count) in NATIVE.items():
        base = directory / run
        summary = json.loads((base / 'summary.json').read_text())
        meta = json.loads((base / 'meta.json').read_text())
        require(summary['id'] == meta['id'] == run and summary['args'] == meta['args'], 'native identity/args mismatch')
        require(summary['exit'] == 0 and summary['tests']['passed'] == count and summary['tests']['failed'] == 0, 'native test counts/exit')
        require(filter_name is None or filter_name in summary['args'], 'native filter missing')
        if count == 5:
            require('ide-diagnostics' in summary['args'] and '--offline' in summary['args'] and '--locked' in summary['args'], 'fiveAPI not owned native API gate')
            require('--manifest-path' in summary['args'] and summary['args'][summary['args'].index('--manifest-path') + 1].endswith('candidate-unit-v25-snapshot/vendor/rust-analyzer/Cargo.toml'), 'fiveAPI not private snapshot')
        lines = [json.loads(line)['l'] for line in (base / 'output.jsonl').read_text().splitlines()]
        raw = '\n'.join(lines)
        require(re.search(rf'test result: ok\. {count} passed; 0 failed;', raw), 'raw native journal not positive')
        if count == 5:
            for name in API_NAMES:
                require(f'test {name} ... ok' in raw, 'actual API test absent from journal')
        log_name = filter_name or 'full-native'
        require((root / f'native/v25-snapshot/{log_name}.exit').read_text().strip() == '0', 'native raw exit mismatch')
        require(f'{count} passed, 0 failed,' in (root / f'native/v25-snapshot/{log_name}.log').read_text(), 'native condensed log count mismatch')
        rows.append(dict(run_id=run, filter=filter_name, passed=count))
    export = read(root, 'offline-export.json')
    for item in export:
        data = read(root, item['source'])
        require(item['summary_sha256'] == sha(root / item['source']) and item['id'] == data['id'] and item['tests'] == data.get('tests') and item['exit'] == data['exit'] and item['args'] == data['args'], 'offline export not exact summary projection')
    require({x['source'] for x in export} == {str(p.relative_to(root)) for p in (root / 'journals').glob('*/*/summary.json')}, 'offline export coverage differs from retained journals')
    require(set(NATIVE).issubset({x['id'] for x in export}), 'native export coverage missing')
    require((root / 'provenance/bench-regression-v25.exit').read_text().strip() == '0', '700 harness regression exit failure')
    require('Ran 700 tests' in (root / 'provenance/bench-regression-v25.log').read_text()
            and (root / 'provenance/bench-regression-v25.log').read_text().rstrip().endswith('OK'), '700 harness regression receipt')
    return rows


def runtime_ok(text):
    require('test result: ok. 4 passed; 0 failed;' in text, 'runtime four-test gate')
    for name in RUNTIME_NAMES:
        require(f'test {name} ...' in text, 'runtime expected test absent')


def suite_checks(root):
    reports = []
    for name in ('rsb16', 'rsa22'):
        base = root / 'fixed-five' / name
        suite = json.loads((base / 'suite.json').read_text())
        rows = json.loads((base / 'records.json').read_text())
        meta = suite['metadata']
        expected_engine = (root / f'build/{"v23" if name == "rsb16" else "v25"}/engine-identity.sha256').read_text().split()[0]
        require(meta['tools_before']['engine']['sha256'] == expected_engine and meta['tools_after']['engine'] == meta['tools_before']['engine'], 'suite engine identity changed or wrong candidate')
        require(meta['status'] == 'complete' and meta['source_restored'] and meta['unchanged'], 'suite acceptance/restoration missing')
        require(meta['source_before'] == meta['source_after'] and meta['toolchain_before'] == meta['toolchain_after'], 'suite input mutation')
        for artifact, digest in suite['artifacts_sha256'].items():
            require('/' not in artifact and sha(base / artifact) == digest, 'suite artifact digest mismatch')
        expected = {(stack, phase, rep) for stack in ('reference', 'engine')
                    for phase, reps in [('version', range(1)), ('prepare', range(1)), ('type-error', range(1)),
                                        ('type-recovery', range(1)), ('assert-failure', range(1)),
                                        ('assert-recovery', range(1)), ('edit', range(5))] for rep in reps}
        require(len(rows) == 22 and {(r['stack'], r['phase'], r['replicate']) for r in rows} == expected, 'suite not exact22 rows')
        for row in rows:
            stem = f"{row['stack']}-{row['phase']}-{row['replicate']}"
            text = '\n'.join((base / f'{stem}.{suffix}').read_text() for suffix in ('stdout', 'stderr'))
            phase = row['phase']
            require(not row['timed_out'] and row['error'] is None, 'runtime timed out/error')
            require('diagnostics transport:' not in text and not any(label in text for label in LABELS), 'disabled suite emitted telemetry')
            if phase in ('type-error', 'assert-failure'):
                require(row['status'] != 0, 'expected negative gate succeeded')
                if phase == 'type-error':
                    require('E0308' in text and 'JR_RUNTIME_TYPE_ERROR' in text, 'type-error diagnostic absent')
                else:
                    require('JR_RUNTIME_FAILURE' in text and '3 passed; 1 failed;' in text, 'assert failure not actual runtime failure')
            else:
                require(row['status'] == 0, 'runtime positive gate failed')
                if phase != 'version':
                    runtime_ok(text)
                if phase == 'edit':
                    require(text.count(f"JR_RUNTIME_EXECUTED_{row['replicate'] + 1}") == 1, 'runtime edit marker absent/duplicated')
        for stack in ('reference', 'engine'):
            times = [r['elapsed_seconds'] for r in rows if r['stack'] == stack and r['phase'] == 'edit']
            require(suite['measurements'][stack]['samples_seconds'] == times, 'suite timing samples disagree')
            require(suite['measurements'][stack]['median_seconds'] == statistics.median(times), 'suite timing median disagrees')
        reports.append(dict(namespace=name, rows=22, disabled_telemetry=True))
    return reports


def profile_checks(root):
    state = read(root, 'profile/state.json')
    require(state['status'] == 'passed' and state['unchanged'] and len(state['rows']) == 6, 'positive profile gate missing')
    require(state['source_before'] == state['source_after'] and state['toolchain_before'] == state['toolchain_after'], 'profile input changed')
    require(state['configs']['new']['sha256'] == state['tools_after']['new'] ==
            (root / 'build/v25/engine-identity.sha256').read_text().split()[0], 'profile binary changed or not accepted candidate')
    reports = []
    for index, row in enumerate(state['rows']):
        require(row['replicate'] == index and row['status'] == 0 and not row['timed_out'] and row['error'] is None, 'profile row failed')
        prefix = root / 'profile' / f'{index}-new'
        text = '\n'.join(prefix.with_suffix('.' + s).read_text() for s in ('stdout', 'stderr'))
        runtime_ok(text)
        require(row['marker'] == f'JR_RUNTIME_EXECUTED_{index + 2501}' and text.count(row['marker']) == 1, 'profile runtime marker')
        trace = root / 'profile' / f'{index}-new.trace.json'
        worker = root / 'profile' / f'{index}-new.worker-delta.log'
        require(sha(trace) == row['trace_sha256'] and sha(worker) == row['worker_delta_sha256'], 'profile raw digest mismatch')
        events = [e for e in json.loads(trace.read_text())['traceEvents'] if e.get('ph') == 'X' and e.get('name', '').startswith('diagnostics.')]
        require(events == row['events'], 'profile raw trace/state disagreement')
        callbacks = [[int(v) for v in m] for m in re.findall(r'asked_callback_calls=(\d+) asked_callback_cumulative_ns=(\d+)', worker.read_text())]
        transport = [[int(v) for v in m] for m in re.findall(r'diagnostics transport: retained=(\d+) stale=(\d+) dropped=(\d+) cap=4000', worker.read_text())]
        file_rows = [(int(f), int(c)) for f, c in re.findall(r'check timing: FileId\((\d+)\): asked_callback_calls=(\d+)', worker.read_text())]
        require(callbacks == row['callbacks'] and transport == row['transport'], 'profile callback/transport state disagreement')
        require(file_rows == ([(540, 373), (33, 231)] if index == 0 else [(540, 373)]), 'profile actual file callback rows')
        require(transport == ([[17, 249, 0]] if index == 0 else [[9, 0, 0]]), 'profile drops/stale/retained counts')
        groups = defaultdict(list)
        for event in events:
            require(event['dur'] >= 0 and isinstance(event['ts'], int), 'invalid actual interval')
            groups[event['pid'], event['tid']].append(event)
        outers = [e for e in events if e['name'] == 'diagnostics.semantic_inclusive_asked']
        require(len(outers) == (2 if index == 0 else 1), 'profile per-thread semantic group count')
        walls = [e for e in events if e['name'] == 'diagnostics.broken_in.wall']
        require(len(walls) == 1, 'outer broken_in wall missing')
        wall = walls[0]
        for outer in outers:
            require(wall['pid'] == outer['pid'] and wall['ts'] <= outer['ts'] and
                    outer['ts'] + outer['dur'] <= wall['ts'] + wall['dur'] + 1, 'semantic interval outside actual broken_in wall')
        thread_rows = []
        for outer in outers:
            own = groups[outer['pid'], outer['tid']]
            counts = Counter(e['name'] for e in own)
            require(all(counts[label] == 1 for label in LABELS), 'same-thread five actual phases missing/duplicated')
            require(counts['diagnostics.skeleton_key_clean'] == counts['diagnostics.filter_cache_update'] == 1, 'same-thread outer labels missing')
            phases = [next(e for e in own if e['name'] == label) for label in LABELS]
            for phase in phases:
                require(outer['ts'] <= phase['ts'] and phase['ts'] + phase['dur'] <= outer['ts'] + outer['dur'] + 1, 'phase outside actual same-thread semantic outer')
            for left, right in zip(phases, phases[1:]):
                # Integer microsecond transport truncates starts and durations separately.
                require(0 <= right['ts'] - left['ts'] - left['dur'] <= 1, 'actual phases not contiguous in label order')
            require(phases[3]['dur'] > 0, 'collect interval not positive')
            thread_rows.append(dict(pid=outer['pid'], tid=outer['tid'], phases_us={p['name']:p['dur'] for p in phases}))
        require(len(events) == transport[0][0], 'transport not equal retained actual event count')
        reports.append(dict(row=index, per_thread=thread_rows, file_callback_rows=file_rows,
                            file_to_thread_mapping='not recorded, no inferred bijection', dropped=0))
    return reports


def validate(root):
    root = root.resolve()
    require(not any(p.is_symlink() for p in root.rglob('*')), 'bundle contains symlink')
    expected = manifest(root, 'SHA256SUMS')
    actual = {str(p.relative_to(root)): sha(p) for p in files(root)}
    require(expected == actual, 'bundle hash inventory mismatch')
    collection = read(root, 'collection.json')
    require(collection['source_commit'] == '90164925c', 'wrong accepted source commit')
    for item in collection['files']:
        require(sha(root / item['artifact']) == item['sha256'], 'copied receipt hash mismatch')
    return dict(verdict='passed', source=source_checks(root), native=native_checks(root),
                disabled_suites=suite_checks(root), profile=profile_checks(root),
                scope='Actual correctness and inclusive telemetry receipts only, no speedup verdict')


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('root', nargs='?', type=Path, default=Path(__file__).resolve().parent)
    parser.add_argument('--sign', action='store_true', help='regenerate inventory, does not validate evidence')
    args = parser.parse_args()
    if args.sign:
        sign(args.root)
    else:
        print(json.dumps(validate(args.root), indent=2))


if __name__ == '__main__':
    main()
