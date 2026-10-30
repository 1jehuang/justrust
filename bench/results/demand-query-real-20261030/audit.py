#!/usr/bin/env python3
"""Audit retained dp43 bytes without native tools or original scratch paths.

Run from any directory with Python 3 and this repository's bench modules.
Checks are receipt consistency, not independent attestation of historical runs.
"""
from collections import defaultdict
import gzip
import hashlib
import json
from pathlib import Path
import re
import shlex
import statistics
import sys

ROOT = Path(__file__).resolve().parent
sys.path.insert(0, str(ROOT.parents[1]))
import query_profile as q
from reuse_ceiling import interval_union_ns
from blaze_runtime_compare import test_results

LABELS = ('incr_comp_query_cache_promotion', 'jr_demand_query_cache_promotion_skipped',
          'incr_comp_persist_result_cache', 'incr_comp_serialize_result_cache',
          'encode_query_values', 'jr_incremental_green_recompute')
TESTS = sorted('summary::tests::' + name for name in (
    'parses_libtest_results', 'shares_overlapping_time',
    'splits_passes_without_double_counting', 'test_failures_are_not_compile_errors'))


def require(ok, message):
    if not ok:
        raise ValueError(message)


def digest(data):
    return hashlib.sha256(data).hexdigest()


def load(name):
    return json.loads((ROOT / name).read_text())


def audit():
    for name, expected in load('retained-sha256.json').items():
        require(digest((ROOT / name).read_bytes()) == expected, f'changed retained bytes: {name}')
    state = load('state.json')
    require(state['status'] == 'completed', 'incomplete run')
    require(state['source_before'] == state['source_after'] and len(state['source_before']) == 63,
            '63-source restoration receipt mismatch')
    require(state['identities_before'] == state['identities_after'], 'changed tool identity receipt')
    identities = state['identities_before']
    require(identities['rustc_driver_0']['sha256'] ==
            'ed21691a9db267ffb71cc3b8975a8cd89768397a7a7f82c9c839083485424efd', 'wrong frozen driver')
    original = (ROOT / 'summary.rs.before').read_bytes()
    edited = (ROOT / 'summary.rs.edited').read_bytes()
    anchor = b'fn shares_overlapping_time() {'
    body = b'let observed = std::hint::black_box(43u64); assert_eq!(observed, 43u64); println!("JR_RUNTIME_EXECUTED_43");'
    require(original.count(anchor) == 1 and edited == original.replace(anchor, anchor + b'\n' + body, 1),
            'not the unique executable edit')
    require(edited.count(b'JR_RUNTIME_EXECUTED_43') == 1, 'nonunique source marker')
    require(digest(original) == state['source_before']['src/summary.rs'], 'wrong original source')
    rows = state['rows']
    require([(r['stack'], r['phase']) for r in rows] ==
            [(s, p) for s in ('owned-off', 'owned-demand') for p in ('prepare', 'edit', 'restore')],
            'missing/duplicate/reordered rows')
    results = []
    for row in rows:
        name = row['stack'] + '/' + row['phase']
        require(row['status'] == 0 and not row['timed_out'] and row['error'] is None, name + ': failed')
        require(row['source_sha256'] == digest(edited if row['phase'] == 'edit' else original), name + ': source')
        receipt = row['raw_output']
        raw = (ROOT / receipt['artifact']).read_bytes()
        require(digest(raw) == receipt['sha256'], name + ': journal digest')
        journal = [json.loads(line) for line in raw.decode().splitlines()]
        require(all(e['s'] in ('o', 'e') and isinstance(e['l'], str) for e in journal), name + ': journal schema')
        text = '\n'.join(e['l'] for e in journal) + '\n'
        parsed = json.loads(json.dumps(test_results(text)))
        require(parsed == row['parsed'] and parsed is not None, name + ': parsed test mismatch')
        require(parsed['tests'] == [[n, 'ok'] for n in TESTS] and
                [parsed[k] for k in ('passed', 'failed', 'ignored', 'filtered')] == [4, 0, 0, 144], name + ': four tests')
        require(re.findall(r'JR_RUNTIME_EXECUTED_\d+', text) ==
                (['JR_RUNTIME_EXECUTED_43'] if row['phase'] == 'edit' else []), name + ': runtime edit marker')
        require(len(row['profiles']) == 1, name + ': profile count')
        profile = row['profiles'][0]
        data = gzip.decompress((ROOT / (profile['path'] + '.gz')).read_bytes())
        require(digest(data) == profile['sha256'], name + ': raw profile digest')
        events, strings = q.prepare(data, allow_unmapped_event_labels=True)
        metadata = q.metadata_from_strings(strings)
        require(metadata == profile['metadata'], name + ': metadata')
        tokens = shlex.split(metadata['cmd'])
        require(tokens[0] == identities['rustc']['path'] and '--test' in tokens and
                tokens[tokens.index('--crate-name') + 1] == 'justrust', name + ': compiler/crate')
        enabled = row['stack'] == 'owned-demand'
        for flags in (tokens, shlex.split(row['environment']['RUSTFLAGS'])):
            require([t for t in flags if t.startswith('-Zjr-demand-query-cache')] ==
                    (['-Zjr-demand-query-cache=yes'] if enabled else []), name + ': demand flag')
            for flag in ('jr-empty-lint-level-fast-path', 'jr-cache-small-query-values', 'jr-cache-associated-items'):
                require([t for t in flags if t.startswith('-Z' + flag)] == ['-Z' + flag + '=no'], name + ': other experimental flags')
        intervals = defaultdict(list)
        total = unresolved = 0
        for event in q.iter_prepared_records(events, strings, allow_unmapped_event_labels=True):
            total += 1
            if not event['label_resolved']:
                unresolved += 1
            elif event['record_type'] == 'interval' and event['kind'] == 'GenericActivity' and event['label'] in LABELS:
                intervals[(event['label'], event['thread'])].append((event['start_ns'], event['end_ns']))
        observed = [dict(label=k[0], thread=k[1], records=len(v), union_elapsed_ns=interval_union_ns(v))
                    for k, v in sorted(intervals.items())]
        require((total, unresolved, observed) == (profile['records'], profile['unresolved_records'], profile['intervals']),
                name + ': exact event counts/interval unions')
        counts = {label: sum(len(v) for (lab, _), v in intervals.items() if lab == label) for label in LABELS}
        require(counts[LABELS[0]] == int(not enabled) and counts[LABELS[1]] == int(enabled), name + ': branch marker')
        require(all(counts[label] == 1 for label in LABELS[2:5]), name + ': persistence/encoding markers')
        results.append(dict(stack=row['stack'], phase=row['phase'], run_id=receipt['run_id'],
                            records=total, unresolved_records=unresolved, intervals=observed))
    # Preserve the independent br42 negative total-wait observation, not dp43 timing.
    negative = load('br42-summary.json')
    records = load('br42-records.json')
    for stack in ('owned-off', 'owned-demand'):
        times = [r['elapsed_seconds'] for r in records if r.get('stack') == stack and r.get('phase') == 'edit']
        require(len(times) == 5 and statistics.median(times) == negative['medians_seconds'][stack], 'br42 median mismatch: ' + stack)
    require(negative['medians_seconds']['owned-demand'] > negative['medians_seconds']['owned-off'], 'lost negative result')
    exports = [json.loads(line) for p in (ROOT / 'exports').rglob('*.jsonl') for line in p.read_text().splitlines()]
    require(sorted(e['id'] for e in exports) == sorted(r['raw_output']['run_id'] for r in rows), 'dedicated exports mismatch')
    return dict(verdict='passed', scope='mechanism-not-total-speedup', rows=results,
                negative_total_medians_seconds={s: negative['medians_seconds'][s] for s in ('owned-off', 'owned-demand')})


if __name__ == '__main__':
    print(json.dumps(audit(), indent=2))
