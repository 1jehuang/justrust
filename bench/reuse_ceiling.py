#!/usr/bin/env python3
"""Offline reuse evidence inventory, not a semantic or performance ceiling.

Reads bounded MMPD format-9 profiles and optionally two pinned stock graphs.
Numeric query IDs never cross invocations. Graph comparisons use (kind, key
fingerprint), not serialization indices. No elapsed spans are added.
"""
import argparse
from collections import Counter
import hashlib
import json

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


def summarize_records(records):
    kinds = Counter()
    providers = Counter()
    hits = Counter()
    counters = {}
    unresolved = 0
    loads = 0
    for event in records:
        kind, typ = event['kind'], event['record_type']
        kinds[kind] += 1
        unresolved += not event['label_resolved']
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
                          events, strings, args.allow_unmapped_event_labels)),
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
