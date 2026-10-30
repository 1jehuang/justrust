#!/usr/bin/env python3
"""Bounded offline Query elapsed-interval inventory, never a provider cost model."""
import argparse
from collections import defaultdict
import hashlib
import json
import re

import query_profile
from reuse_ceiling import interval_union_ns

SAFE_LABEL = re.compile(r'[A-Za-z_][A-Za-z_0-9]*', re.ASCII)


def summarize_records(records):
    groups = defaultdict(list)
    threads = defaultdict(list)
    total = unresolved = unknown = 0
    for record in records:
        if not isinstance(record, dict):
            raise ValueError('invalid profile record')
        if type(record.get('label_resolved')) is not bool:
            raise ValueError('invalid label resolution state')
        unresolved += not record['label_resolved']
        if not isinstance(record.get('kind'), str):
            raise ValueError('invalid event kind')
        if record.get('record_type') not in ('interval', 'instant', 'integer'):
            raise ValueError('invalid record type')
        if record['kind'] != 'Query':
            continue
        if record['record_type'] != 'interval':
            raise ValueError('Query must be an interval')
        thread = record.get('thread')
        if type(thread) is not int or thread < 0:
            raise ValueError('invalid profiler thread')
        span = (record.get('start_ns'), record.get('end_ns'))
        interval_union_ns([span])
        label = record.get('label')
        if not record['label_resolved'] or not isinstance(label, str) or not SAFE_LABEL.fullmatch(label):
            label = None
            unknown += 1
        total += 1
        groups[label, thread].append(span)
        threads[thread].append(span)
    return dict(
        query_interval_records=total,
        unresolved_event_label_records=unresolved,
        unknown_query_name_interval_records=unknown,
        per_kind_per_thread=[dict(query_kind=kind, thread=thread,
                                 interval_records=len(spans),
                                 interval_union_ns=interval_union_ns(spans))
                             for (kind, thread), spans in sorted(groups.items(), key=lambda item: (item[0][0] is None, item[0][0] or '', item[0][1]))],
        all_query_per_thread=[dict(thread=thread, interval_records=len(spans),
                                   interval_union_ns=interval_union_ns(spans))
                              for thread, spans in sorted(threads.items())],
        capture_flags='unknown', capture_complete=None,
        cost_for_same_results_ns=None,
        limitations=[
            'Only observed Query intervals grouped by safe static label and profiler thread. Unknown names are null.',
            'All-query unions are computed independently per thread. Do not add across kinds or threads: intervals can nest or overlap.',
            'Elapsed intervals are not CPU, exclusive provider cost, saved time, or a global ceiling.',
            'Capture flags and completeness are unknown. No marker-to-provider joins or same-result cost inference. Arguments are never emitted.',
        ])


def analyze(raw, large_capture=False, allow_unmapped_event_labels=False):
    events, strings = query_profile.prepare(raw, allow_unmapped_event_labels, large_capture)
    result = summarize_records(query_profile.iter_prepared_records(events, strings, allow_unmapped_event_labels))
    result['large_capture'] = large_capture
    result['allow_unmapped_event_labels'] = allow_unmapped_event_labels
    result['label_coverage_complete'] = result['unresolved_event_label_records'] == 0
    result.update(schema_version=1, raw_capture_sha256=hashlib.sha256(raw).hexdigest())
    return result


def main(argv=None):
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--profile', required=True)
    parser.add_argument('--large-capture', action='store_true')
    parser.add_argument('--allow-unmapped-event-labels', action='store_true',
                        help='retain missing virtual labels as unknown; other decoding stays strict')
    args = parser.parse_args(argv)
    try:
        result = analyze(query_profile.read_input(args.profile), args.large_capture,
                         args.allow_unmapped_event_labels)
    except (ValueError, OSError):
        # Decoder exceptions can contain capture strings or paths. Never echo them.
        parser.exit(2, 'provider_cost: invalid or unsupported profile (details withheld)\n')
    print(json.dumps(result, sort_keys=True))
    return 0


if __name__ == '__main__':
    raise SystemExit(main())
