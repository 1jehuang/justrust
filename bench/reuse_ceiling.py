#!/usr/bin/env python3
"""Offline reuse evidence inventory, not a semantic or performance ceiling.

Reads bounded MMPD format-9 profiles and optionally two pinned stock graphs.
Numeric query IDs never cross invocations. Graph comparisons use (kind, key
fingerprint), not serialization indices. No elapsed spans are added.
"""
import argparse
from collections import Counter, defaultdict
import hashlib
import json
import re

import query_graph
import query_profile


LIMITATIONS = [
    'Capture filters, dropped instrumentation and unrecorded requests are unknown. Absence is not zero execution or zero reuse.',
    'QueryCacheHit instant records and QueryCacheHitCount integer values describe overlapping in-memory hits and must not be added.',
    'IncrementalLoadResult intervals record disk-result loading, not all cross-session reuse or provider avoidance.',
    'Repeated Query records for an invocation-local ID are candidates for investigation only, not proven redundant provider executions.',
    'Matching graph fingerprints do not establish semantic dispensability, execution, cache eligibility, or sound reuse.',
    'No nested wall spans are summed. Counts are not time saved, CPU work, execution span, or a theoretical maximum.',
]


NATIVE_MARKERS = ('jr_incremental_green_query', 'jr_incremental_green_recompute')
OPTIONAL_BRANCH_MARKERS = ('jr_empty_lint_level_fast_path',)
REEXECUTION_MARKERS = tuple(
    f'jr_incremental_{family}_{outcome}'
    for family in ('reexecuted', 'eval_always')
    for outcome in ('same_value', 'different_value', 'no_hash')
)


def interval_union_ns(intervals):
    """Union within one profiler thread and epoch, never CPU or saved time."""
    for start, stop in intervals:
        if any(type(v) is not int or v < 0 for v in (start, stop)) or stop < start:
            raise ValueError('invalid native marker interval')
    end = None
    total = 0
    for start, stop in sorted(intervals):
        total += max(0, stop - max(start, end if end is not None else start))
        end = max(stop, end if end is not None else stop)
    return total


def native_summary(markers):
    result = {}
    for marker in NATIVE_MARKERS:
        values = markers[marker]
        threads = defaultdict(list)
        names = Counter()
        unknown_names = 0
        for event, name in values:
            threads[event['thread']].append((event['start_ns'], event['end_ns']))
            if name is None:
                unknown_names += 1
            else:
                names[name] += 1
        result[marker] = dict(
            evidence='observed' if values else 'unknown-marker-absent',
            interval_records=len(values) if values else None,
            query_name_interval_records=dict(sorted(names.items())),
            records_without_query_name=unknown_names if values else None,
            per_thread=[dict(thread=thread, interval_records=len(spans),
                             union_elapsed_ns=interval_union_ns(spans))
                        for thread, spans in sorted(threads.items())],
        )
    return dict(markers=result, capture_complete=None, marker_filter_enabled=None,
                semantically_avoidable_provider_calls=None, time_saved_ns=None,
                scope='Native proven-green route and provider-recompute branch only. Counts are invocations, not distinct query keys. Per-thread elapsed unions are not summed across threads or markers, CPU work, or removable time.')


def optional_branch_summary(markers):
    result = {}
    for marker in OPTIONAL_BRANCH_MARKERS:
        values = markers[marker]
        threads = defaultdict(list)
        for event in values:
            threads[event['thread']].append((event.get('start_ns'), event.get('end_ns')))
        result[marker] = dict(
            evidence='observed' if values else 'unknown-marker-absent',
            interval_records=len(values) if values else None,
            per_thread=[dict(thread=thread, interval_records=len(spans),
                             union_elapsed_ns=interval_union_ns(spans))
                        for thread, spans in sorted(threads.items())],
        )
    return dict(markers=result, capture_complete=None, marker_filter_enabled=None,
                cpu_work_ns=None, time_saved_ns=None, critical_path_ns=None,
                scope='Optional native branch inventory only. Counts are branch entries, not query keys or avoided work. Per-thread elapsed unions are not summed across threads or markers and imply no CPU work, saved time, or critical path.')


def reexecution_summary(markers):
    result = {}
    for marker in REEXECUTION_MARKERS:
        values = markers[marker]
        names = Counter()
        unknown_names = 0
        for event, name in values:
            # Check the event shape without assigning meaning to classification
            # duration. This marker is emitted after provider execution.
            interval_union_ns([(event.get('start_ns'), event.get('end_ns'))])
            if name is None:
                unknown_names += 1
            else:
                names[name] += 1
        result[marker] = dict(
            evidence='observed' if values else 'unknown-marker-absent',
            interval_records=len(values) if values else None,
            query_name_interval_records=dict(sorted(names.items())),
            records_without_query_name=unknown_names if values else None,
        )
    return dict(markers=result, capture_complete=None, marker_filter_enabled=None,
                changed_input_queries=None, avoidable_provider_calls=None,
                provider_elapsed_ns=None, cpu_work_ns=None, time_saved_ns=None,
                scope='Retrospective result-hash classifications for re-executed existing keys only. Eval-always is separate. New keys are excluded; no_hash is unknown value equality. Same value does not prove changed inputs, avoidable execution, or prospective reuse. Marker durations are not provider durations.')


def summarize_records(records, resolve_event=None):
    kinds = Counter()
    providers = Counter()
    hits = Counter()
    counters = {}
    unresolved = 0
    loads = 0
    markers = defaultdict(list)
    optional_markers = defaultdict(list)
    reexecution_markers = defaultdict(list)
    for event in records:
        kind, typ = event['kind'], event['record_type']
        kinds[kind] += 1
        unresolved += not event['label_resolved']
        if event.get('label') in NATIVE_MARKERS + OPTIONAL_BRANCH_MARKERS + REEXECUTION_MARKERS:
            if kind != 'GenericActivity' or typ != 'interval':
                raise ValueError('invalid native marker event type')
            if type(event.get('thread')) is not int or event['thread'] < 0:
                raise ValueError('invalid native marker thread')
            if event.get('label') in OPTIONAL_BRANCH_MARKERS:
                optional_markers[event['label']].append(event)
                continue
            # The normal decoder intentionally discards function arguments.
            # Resolve only these known marker strings, never arbitrary labels.
            parts = resolve_event(event['event_id']).split('\x1e') if resolve_event else []
            name = (parts[1] if len(parts) == 2 and parts[0] == event['label']
                    and re.fullmatch(r'[A-Za-z_][A-Za-z_0-9]*', parts[1]) else None)
            destination = reexecution_markers if event['label'] in REEXECUTION_MARKERS else markers
            destination[event['label']].append((event, name))
        expected = {'Query': 'interval', 'QueryCacheHit': 'instant',
                    'QueryCacheHitCount': 'integer', 'IncrementalLoadResult': 'interval'}
        if kind in expected and typ != expected[kind]:
            raise ValueError('unexpected reuse event record type')
        if kind not in expected:
            continue
        identity = event['event_id']
        if not 0 <= identity <= 100_000_000:
            raise ValueError('reuse event lacks invocation-local virtual ID')
        if kind == 'Query':
            providers[identity] += 1
        elif kind == 'QueryCacheHit':
            hits[identity] += 1
        elif kind == 'QueryCacheHitCount':
            if identity in counters:
                raise ValueError('duplicate query cache-hit counter identity')
            counters[identity] = event['value']
        else:
            loads += 1
    common = hits.keys() & counters.keys()
    return dict(
        records=sum(kinds.values()), event_kind_records=dict(sorted(kinds.items())),
        unresolved_label_records=unresolved,
        observed_provider_interval_records=sum(providers.values()),
        distinct_provider_ids=len(providers),
        repeated_provider_ids=sum(n > 1 for n in providers.values()),
        hypothetical_repeat_provider_record_candidates=sum(n - 1 for n in providers.values()),
        in_memory_hit_instant_records=sum(hits.values()),
        distinct_in_memory_hit_ids=len(hits),
        hit_ids_without_observed_provider=len(hits.keys() - providers.keys()),
        in_memory_hit_counter_records=len(counters),
        in_memory_hit_counter_value_sum=sum(counters.values()),
        counter_ids_without_observed_provider=len(counters.keys() - providers.keys()),
        hit_ids_in_both_channels=len(common),
        hit_ids_with_channel_count_disagreement=sum(hits[i] != counters[i] for i in common),
        incremental_load_interval_records=loads,
        coverage=dict(capture_complete=None, request_count=None, enabled_filters=None,
                      cross_session_reused_query_count=None, semantic_avoidable_query_count=None),
        cpu_work_ns=None, time_saved_ns=None, execution_span_ns=None,
        theoretical_maximum_reuse=None,
        native_green_reuse=native_summary(markers),
        optional_branch_inventory=optional_branch_summary(optional_markers),
        native_reexecution_results=reexecution_summary(reexecution_markers),
    )


def compare_graphs(before, after):
    """Descriptive fingerprint comparison only, independent of profile IDs."""
    if (before['compiler_commit'], before['command_hash_le']) != (
            after['compiler_commit'], after['command_hash_le']):
        raise ValueError('graph compiler or command hash mismatch')

    def index(graph):
        result = {}
        for node in graph['nodes'].values():
            key = (node['kind'], node['key_fingerprint'])
            if key in result:
                raise ValueError('ambiguous duplicate graph query key')
            result[key] = node['value_fingerprint']
        return result

    old, new = index(before), index(after)
    shared = old.keys() & new.keys()
    return dict(before_sha256=before['sha256'], after_sha256=after['sha256'],
                identical_graph_bytes=before['sha256'] == after['sha256'],
                before_nodes=len(old), after_nodes=len(new), shared_query_keys=len(shared),
                before_only_keys=len(old.keys() - new.keys()),
                after_only_keys=len(new.keys() - old.keys()),
                shared_keys_matching_value_fingerprint=sum(old[k] == new[k] for k in shared),
                shared_keys_changed_value_fingerprint=sum(old[k] != new[k] for k in shared),
                observed_cross_session_reuse=None, executed_matching_result_queries=None,
                hypothetical_avoidable_recomputation=None,
                provenance='Caller-selected snapshots. Ordering, crate identity and session continuity are not independently established. No profile join.')


def main(argv=None):
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--profile', required=True)
    parser.add_argument('--before-graph')
    parser.add_argument('--after-graph')
    parser.add_argument('--large-capture', action='store_true')
    parser.add_argument('--allow-unmapped-event-labels', action='store_true')
    args = parser.parse_args(argv)
    if bool(args.before_graph) != bool(args.after_graph):
        parser.error('both graph snapshots are required')
    try:
        raw = query_profile.read_input(args.profile)
        events, strings = query_profile.prepare(raw, args.allow_unmapped_event_labels, args.large_capture)
        result = dict(schema=1, profile_sha256=hashlib.sha256(raw).hexdigest(),
                      profile_bytes=len(raw),
                      profile=summarize_records(query_profile.iter_prepared_records(
                          events, strings, args.allow_unmapped_event_labels), strings.resolve),
                      graph_comparison=None, limitations=LIMITATIONS)
        if args.before_graph:
            result['graph_comparison'] = compare_graphs(*[
                query_graph.decode(query_graph.read(path), args.large_capture)
                for path in (args.before_graph, args.after_graph)])
        print(json.dumps(result, sort_keys=True, allow_nan=False))
    except (ValueError, OSError, UnicodeError):
        parser.exit(2, 'reuse ceiling: invalid, unsupported or unavailable evidence\n')


if __name__ == '__main__':
    main()
