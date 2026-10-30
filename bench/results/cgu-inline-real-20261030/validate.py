#!/usr/bin/env python3
"""Portable offline replay. Never invokes rustc, cargo, justrust or captured runners."""
import argparse
from collections import defaultdict
import gzip
import hashlib
import json
from pathlib import Path
import re
import shlex
import sys

ROOT = Path(__file__).resolve().parent
sys.path.insert(0, str(ROOT / 'decoder'))
import query_profile as q

STACKS = ('owned-off', 'owned-cgu-traversal')
PHASES = ('prepare', 'edit', 'restore')
LABELS = ('cgu_partitioning_place_items', 'cgu_partitioning', 'partition_and_assert_distinct_symbols', 'jr_cgu_inline_shared_traversal', 'jr_cgu_inline_shared_revisit', 'macro_expand_crate', 'codegen_crate')
TRAVERSAL = 'jr_cgu_inline_shared_traversal'
REVISIT = 'jr_cgu_inline_shared_revisit'
DRIVER = '29ea709935d751ce13e77995173659006c1e15683e0f0645616d12cd5a3a1665'
TESTS = ['summary::tests::' + n for n in ('parses_libtest_results', 'shares_overlapping_time', 'splits_passes_without_double_counting', 'test_failures_are_not_compile_errors')]
FLAGS = '-Zjr-empty-lint-level-fast-path=no -Zjr-cache-small-query-values=no -Zjr-cache-associated-items=no'


def digest(raw):
    return hashlib.sha256(raw).hexdigest()


def union_ns(intervals):
    total = 0
    end = -1
    for start, stop in sorted(intervals):
        assert 0 <= start <= stop
        total += max(0, stop - max(start, end))
        end = max(end, stop)
    return total


def parse_runtime(output):
    names = re.findall(r'^test ([^\s]+) \.\.\. (?:(?:[^\n]*\n)*?)(ok|FAILED|ignored)(?: <[0-9]+(?:\.[0-9]+)?s>)?[ \t]*$', output, re.M)
    totals = re.findall(r'test result: (ok|FAILED)\. (\d+) passed; (\d+) failed; (\d+) ignored; (\d+) measured; (\d+) filtered out', output)
    assert len(totals) == 1 and len(names) == len(dict(names)) == 4
    state, passed, failed, ignored, measured, filtered = totals[0]
    assert [int(passed), int(failed), int(ignored)] == [sum(s == x for _, s in names) for x in ('ok', 'FAILED', 'ignored')]
    assert state == 'ok' and measured == '0'
    return dict(tests=sorted([list(p) for p in names]), passed=int(passed), failed=int(failed), ignored=int(ignored), filtered=int(filtered))


def safe_files(root):
    result = {}
    for path in root.rglob('*'):
        assert not path.is_symlink(), str(path)
        if path.is_file() and '__pycache__' not in path.parts and path.name != 'sha256.json':
            result[str(path.relative_to(root))] = path
    return result


def inventory(root):
    paths = safe_files(root)
    expected = json.loads((root / 'sha256.json').read_text())
    assert set(paths) == set(expected), 'inventory changed'
    for name, path in paths.items():
        assert digest(path.read_bytes()) == expected[name], 'inventory hash: ' + name


def load_small(root):
    return {name: path.read_bytes() for name, path in safe_files(root).items() if not name.endswith('.mm_profdata.gz')}


def load_json(blobs, name):
    return json.loads(blobs[name])


def decode_profile(raw):
    events, strings = q.prepare(raw)
    metadata = q.metadata_from_strings(strings)
    intervals = defaultdict(list)
    total = unresolved = 0
    for event in q.iter_prepared_records(events, strings):
        total += 1
        unresolved += not event['label_resolved']
        if event['label_resolved'] and event['record_type'] == 'interval' and event['kind'] == 'GenericActivity' and event['label'] in LABELS:
            intervals[event['label'], event['thread']].append([event['start_ns'], event['end_ns']])
    return dict(metadata=metadata, records=total, unresolved_records=unresolved,
                intervals=[dict(label=label, thread=thread, records=len(values), union_elapsed_ns=union_ns(values), intervals_ns=values)
                           for (label, thread), values in sorted(intervals.items())])


def decode_all(root, blobs):
    state = load_json(blobs, 'state.json')
    profiles = load_json(blobs, 'profiles.json')
    assert len(profiles) == 6
    decoded = {}
    for row, profile in zip(state['rows'], profiles):
        key = row['stack'] + '-' + row['phase']
        assert profile['key'] == key
        path = Path(profile['path'])
        assert not path.is_absolute() and '..' not in path.parts
        compressed = (root / path).read_bytes()
        assert digest(compressed) == profile['gzip_sha256'] and len(compressed) == profile['gzip_bytes']
        with gzip.open(root / path, 'rb') as stream:
            raw = stream.read(q.MAX_FILE_BYTES + 1)
        assert len(raw) <= q.MAX_FILE_BYTES
        assert len(raw) == profile['raw_bytes']
        assert digest(raw) == profile['raw_sha256'] == row['profiles'][0]['sha256']
        decoded[key] = decode_profile(raw)
    return decoded


def validate_structure(blobs, decoded):
    state = load_json(blobs, 'state.json')
    assert state['status'] == 'completed'
    assert state['scope'] == 'real-cgu-traversal-mechanism-not-speedup'
    assert state['source_before'] == state['source_after'] and len(state['source_before']) == 63
    assert state['identities_before'] == state['identities_after']
    identities = state['identities_before']
    assert identities['rustc_driver_0']['sha256'] == DRIVER
    for name, sha in state['source_before'].items():
        assert digest(blobs['source/' + name]) == sha, 'source bytes: ' + name
    original = blobs['source.original']
    assert original == blobs['source/src/summary.rs']
    anchor = b'fn shares_overlapping_time() {'
    assert original.count(anchor) == 1
    edited = original.replace(anchor, anchor + b'\nlet observed = std::hint::black_box(47u64); assert_eq!(observed, 47u64); println!("JR_RUNTIME_EXECUTED_47");', 1)
    assert blobs['source.edited'] == edited
    assert blobs['runner/profile47.exit'] == b'0\n'
    assert blobs['runner/profile47.stderr'] == b''
    assert [(r['stack'], r['phase']) for r in state['rows']] == [(s, p) for s in STACKS for p in PHASES]
    assert set(decoded) == {s + '-' + p for s in STACKS for p in PHASES}
    expected = dict(tests=[[n, 'ok'] for n in TESTS], passed=4, failed=0, ignored=0, filtered=144)
    rows = []
    ids = set()
    for row in state['rows']:
        stack, phase = row['stack'], row['phase']
        key = stack + '-' + phase
        env = row['environment']
        enabled = stack == STACKS[1]
        assert row['status'] == 0 and not row['error'] and not row['timed_out'] and row['elapsed_seconds'] > 0
        assert row['noise']['contamination'] == 'unknown'
        assert row['source_sha256'] == digest(edited if phase == 'edit' else original)
        assert row['command'] == [identities['justrust']['path'], 'test', '--lib', '--', 'summary::tests::', '--test-threads=1', '--nocapture']
        assert env['RUSTC'] == identities['rustc']['path']
        assert env['CARGO'] == env['JUSTRUST_REAL_CARGO'] == identities['cargo']['path']
        isolation = '/home/jeremy/.jcode/scratch/dp47/' + stack
        for field, suffix in (('HOME', '/home'), ('CARGO_TARGET_DIR', '/target'), ('JUSTRUST_HOME', '/home/.justrust'), ('XDG_RUNTIME_DIR', '/runtime'), ('XDG_CACHE_HOME', '/cache'), ('XDG_CONFIG_HOME', '/config'), ('XDG_DATA_HOME', '/data')):
            assert env[field] == isolation + suffix
        expected_flags = FLAGS + (' -Zjr-cgu-shared-inline-traversal=yes' if enabled else '') + ' -Zself-profile=/home/jeremy/.jcode/scratch/dp47/profiles/' + stack + ' -Zself-profile-events=generic-activity,function-args'
        assert env['RUSTFLAGS'] == expected_flags
        for field in ('JUSTRUST_REMOTE', 'JUSTRUST_PREBUILT'):
            assert env[field] == 'off'
        for field in ('JUSTRUST_BUILDD', 'JUSTRUST_BUILDD_PREBUILD', 'JUSTRUST_CARGO_FORK', 'JUSTRUST_DEPCACHE', 'JUSTRUST_LOCALCACHE', 'JUSTRUST_LOCALCACHE_TARGET_BOUND', 'JUSTRUST_CACHE_PROC_MACROS', 'JUSTRUST_PEER_CACHE', 'JUSTRUST_PREBUILT_ALWAYS', 'JUSTRUST_REMOTE_CACHE', 'JUSTRUST_TOOLCHAIN_AUTO_INSTALL'):
            assert env[field] == '0'
        assert env['JUSTRUST_TOOLCHAIN'] == 'system' and env['JUSTRUST_TOOLCHAIN_NIGHTLY'] == '1'
        assert 'JUSTRUST_HOTPATCH' not in env and 'JUSTRUST_CARGO_DAEMON' not in env
        for profile in ('DEV', 'TEST'):
            for field, value in (('DEBUG', '0'), ('INCREMENTAL', 'true'), ('OPT_LEVEL', '0')):
                assert env['CARGO_PROFILE_' + profile + '_' + field] == value
        receipt = row['raw_output']
        assert receipt['artifact'] == key + '.raw-output.jsonl' and not receipt['hotpatch']['present']
        assert receipt['run_id'] not in ids
        ids.add(receipt['run_id'])
        raw = blobs['logs/' + receipt['artifact']]
        assert digest(raw) == receipt['sha256']
        assert blobs['journals/' + key + '/output.jsonl'] == raw
        entries = [json.loads(line) for line in raw.splitlines()]
        assert entries and all(e['s'] in ('o', 'e') and isinstance(e['l'], str) for e in entries)
        output = '\n'.join(e['l'] for e in entries) + '\n'
        assert parse_runtime(output) == row['parsed'] == expected
        assert re.findall(r'JR_RUNTIME_EXECUTED_\d+', output) == (['JR_RUNTIME_EXECUTED_47'] if phase == 'edit' else [])
        meta = load_json(blobs, 'journals/' + key + '/meta.json')
        summary = load_json(blobs, 'journals/' + key + '/summary.json')
        assert meta['id'] == summary['id'] == receipt['run_id']
        assert meta['args'] == summary['args'] == row['command'][1:]
        assert meta['real_rustc'] == env['RUSTC'] and meta['real_cargo'] == env['CARGO']
        assert meta['env']['RUSTFLAGS'] == expected_flags and summary['exit'] == 0
        assert summary['tests']['passed'] == 4 and summary['tests']['failed'] == 0
        observation = decoded[key]
        recorded = row['profiles'][0]
        assert observation['metadata'] == recorded['metadata']
        assert observation['records'] == recorded['records'] and observation['unresolved_records'] == recorded['unresolved_records'] == 0
        compact = [{k: v for k, v in interval.items() if k != 'intervals_ns'} for interval in observation['intervals']]
        assert compact == recorded['intervals'], 'per-thread interval counts/unions differ: ' + key
        for interval in observation['intervals']:
            assert interval['records'] == len(interval['intervals_ns'])
            assert interval['union_elapsed_ns'] == union_ns(interval['intervals_ns'])
        cmd = shlex.split(observation['metadata']['cmd'])
        assert cmd[0] == env['RUSTC'] and cmd[cmd.index('--crate-name') + 1] == 'justrust'
        assert '--test' in cmd and '-C' in cmd and 'incremental=' + isolation + '/target/debug/incremental' in cmd
        assert all(flag in cmd for flag in shlex.split(expected_flags))
        shared_flags = [flag for flag in cmd if flag.startswith('-Zjr-cgu-shared-inline-traversal')]
        assert shared_flags == (['-Zjr-cgu-shared-inline-traversal=yes'] if enabled else [])
        by_label = defaultdict(list)
        counts = defaultdict(int)
        for interval in observation['intervals']:
            by_label[interval['label']].append(interval)
            counts[interval['label']] += interval['records']
        assert counts[TRAVERSAL] == ({'prepare': 14040, 'edit': 14041, 'restore': 14040}[phase] if enabled else 0)
        assert counts[REVISIT] == int(enabled)
        assert len(by_label['cgu_partitioning_place_items']) == 1
        place = by_label['cgu_partitioning_place_items'][0]
        assert place['records'] == 1
        start, end = place['intervals_ns'][0]
        for label in (TRAVERSAL, REVISIT):
            for group in by_label[label]:
                assert group['thread'] == place['thread']
                assert all(start <= a <= b <= end for a, b in group['intervals_ns'])
        if enabled:
            revisit = by_label[REVISIT][0]['intervals_ns'][0]
            assert any(b <= revisit[0] for group in by_label[TRAVERSAL] for _, b in group['intervals_ns'])
        export_paths = [name for name in blobs if name.startswith('exports/xps14-cgu-inline-real-' + stack + '-20261030/') and name.endswith('.jsonl')]
        exports = [json.loads(line) for name in export_paths for line in blobs[name].splitlines()]
        assert len(exports) == 3
        exported = [entry for entry in exports if entry['id'] == receipt['run_id']]
        assert len(exported) == 1 and exported[0]['exit'] == 0 and exported[0]['repo'] == 'private'
        assert exported[0]['machine'] == 'xps14-cgu-inline-real-' + stack + '-20261030'
        rows.append(dict(key=key, run_id=receipt['run_id'], place_elapsed_ns=place['union_elapsed_ns'], traversal_events=counts[TRAVERSAL], revisit_events=counts[REVISIT], subprocess_elapsed_seconds=row['elapsed_seconds']))
    return dict(verdict='passed', scope='offline-real-workload-mechanism-not-speedup', source_files=63, rows=rows, unresolved_records=0)


def validate(root=ROOT, check_inventory=True, write_observations=False):
    if check_inventory:
        inventory(root)
    blobs = load_small(root)
    decoded = decode_all(root, blobs)
    result = validate_structure(blobs, decoded)
    if write_observations:
        (root / 'observations.json').write_text(json.dumps(decoded, indent=2) + '\n')
    else:
        assert decoded == load_json(blobs, 'observations.json'), 'exact raw interval replay differs'
    return result


if __name__ == '__main__':
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--write-observations', action='store_true', help='initial offline derivation before inventory freeze')
    args = parser.parse_args()
    print(json.dumps(validate(check_inventory=not args.write_observations, write_observations=args.write_observations), indent=2))
