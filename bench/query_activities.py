#!/usr/bin/env python3
"""Bounded all-kind activity elapsed accounting for decoded measureme records.

Retains compact interval tuples per Rust thread, not constant-memory streaming.
No activity classification, CPU-work estimate, or execution-DAG span inference.
"""
import argparse
import hashlib
import os
from pathlib import Path

import query_profile as qprof

MAX_RECORDS = 8_000_000
MAX_THREADS = 16_384
MAX_GROUPS = 2_048
MAX_OUTPUT_BYTES = 1024 * 1024


def nonnegative(value):
    return type(value) is int and value >= 0


def summarize_events(iterable, max_records=8_000_000):
    """Consume every record and partition laminar per-thread elapsed intervals.

    Crossing intervals fail closed. Equal intervals form a stable raw-order
    nesting chain with arbitrary exclusive attribution. Noninterval records
    contribute counts, never durations. Observation windows include instant
    timestamps but not integer payloads. Thread gaps use interval endpoints only.
    Memory is O(interval records + bounded groups + bounded threads).
    """
    if type(max_records) is not int or not 0 <= max_records <= 8_000_000:
        raise ValueError('invalid record bound')
    limit = min(max_records, MAX_RECORDS)
    threads, groups, kinds = {}, {}, {}
    types = dict(interval=0, instant=0, integer=0)
    count = 0
    first = last = None
    for ordinal, record in enumerate(iterable):
        count += 1
        if count > limit:
            raise ValueError('activity record bound exceeded')
        if not isinstance(record, dict):
            raise ValueError('invalid decoded record')
        thread, kind, label = record.get('thread'), record.get('kind'), record.get('label')
        if not nonnegative(thread) or not isinstance(kind, str) or not kind:
            raise ValueError('invalid activity thread or kind')
        if label is not None:
            if not isinstance(label, str):
                raise ValueError('invalid activity label')
            try:
                size = len(label.encode('utf-8'))
            except UnicodeError as error:
                raise ValueError('invalid activity label encoding') from error
            if size > 4096:
                raise ValueError('activity label exceeds 4096 UTF-8 bytes')
        record_type = record.get('record_type')
        if not isinstance(record_type, str) or record_type not in types:
            raise ValueError('invalid activity record type')
        if thread not in threads:
            if len(threads) >= MAX_THREADS:
                raise ValueError('activity thread bound exceeded')
            threads[thread] = []
        key = (kind, label)
        if key not in groups:
            if len(groups) >= MAX_GROUPS:
                raise ValueError('activity group bound exceeded')
            groups[key] = dict(kind=kind, label=label, records=0,
                record_types=dict(interval=0, instant=0, integer=0),
                interval_count=0, inclusive_elapsed_ns=0,
                exclusive_elapsed_ns=0, largest_interval_ns=0)
        row = groups[key]
        row['records'] += 1
        row['record_types'][record_type] += 1
        types[record_type] += 1
        kinds[kind] = kinds.get(kind, 0) + 1
        if record_type == 'integer':
            if not nonnegative(record.get('value')):
                raise ValueError('invalid integer payload')
            continue
        start = record.get('start_ns')
        if not nonnegative(start):
            raise ValueError('invalid activity timestamp')
        end = record.get('end_ns') if record_type == 'interval' else start
        if not nonnegative(end) or end < start:
            raise ValueError('invalid activity interval timestamps')
        first = start if first is None else min(first, start)
        last = end if last is None else max(last, end)
        if record_type == 'interval':
            duration = end - start
            row['interval_count'] += 1
            row['inclusive_elapsed_ns'] += duration
            row['largest_interval_ns'] = max(row['largest_interval_ns'], duration)
            threads[thread].append((start, end, ordinal, key))
    thread_rows = []
    equal = covered_total = 0
    for thread, intervals in sorted(threads.items()):
        intervals.sort(key=lambda item: (item[0], -item[1], item[2]))
        stack = []
        covered = 0
        previous = None
        def finish():
            _, _, key, exclusive = stack.pop()
            if exclusive < 0:
                raise ValueError('negative exclusive activity elapsed')
            groups[key]['exclusive_elapsed_ns'] += exclusive
        for start, end, _, key in intervals:
            if previous == (start, end):
                equal += 1
            previous = (start, end)
            while stack and stack[-1][1] <= start:
                finish()
            duration = end - start
            if stack:
                if end > stack[-1][1]:
                    raise ValueError(f'crossing same-thread activity intervals: thread={thread}, '
                        f'parent=[{stack[-1][0]},{stack[-1][1]}], '
                        f'child=[{start},{end}], raw_ordinal={_}')
                stack[-1][3] -= duration
            else:
                covered += duration
            stack.append([start, end, key, duration])
        while stack:
            finish()
        lower = intervals[0][0] if intervals else None
        upper = max(item[1] for item in intervals) if intervals else None
        window = upper - lower if intervals else 0
        thread_rows.append(dict(thread=thread, interval_count=len(intervals),
            first_interval_ns=lower, last_interval_ns=upper,
            covered_elapsed_ns=covered, interval_window_ns=window,
            gap_elapsed_ns=window - covered))
        covered_total += covered
    rows = sorted(groups.values(), key=lambda row: (row['kind'], row['label'] is not None, row['label'] or ''))
    exclusive_total = sum(row['exclusive_elapsed_ns'] for row in rows)
    if exclusive_total != covered_total:
        raise ValueError('exclusive/covered activity invariant failed')
    return dict(records=count, record_types=types, event_kind_counts=dict(sorted(kinds.items())),
        rows=rows, threads=thread_rows, covered_elapsed_ns=covered_total,
        exclusive_elapsed_ns=exclusive_total, observation_start_ns=first,
        observation_end_ns=last, observation_window_ns=last - first if first is not None else 0,
        equal_interval_records=equal, cpu_work_ns=None, execution_span_ns=None,
        limitations=[
            'Elapsed intervals are not CPU work or a complete execution span. Rust thread IDs are not proven OS TIDs.',
            'Equal intervals use stable raw order with arbitrary exclusive attribution; equal_interval_records counts duplicates beyond the first per thread and endpoint pair.',
            'All recorded kinds are retained without classification. Query is not synonymous with frontend.',
            'Exclusive elapsed subtracts direct nested intervals of ALL kinds, unlike query-only attribution; waits, subprocesses and descheduling may remain.',
            'Observation window covers interval and instant timestamps only; thread gaps cover first/last interval endpoints and are not proven idle time.',
            'Memory is not constant: all interval tuples are retained per Rust thread.'])


def write_output(path, result):
    raw = qprof.record_json(result) + b'\n'
    if len(raw) > MAX_OUTPUT_BYTES:
        raise ValueError('activity output exceeds 1MiB bound')
    with os.fdopen(qprof.open_regular(path, os.O_WRONLY | os.O_CREAT | os.O_EXCL), 'wb') as handle:
        handle.write(raw)


def main(argv=None):
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--profile', type=Path, required=True)
    parser.add_argument('--output', type=Path, required=True)
    parser.add_argument('--large-capture', action='store_true')
    parser.add_argument('--allow-unmapped-event-labels', action='store_true')
    args = parser.parse_args(argv)
    try:
        data = qprof.read_input(args.profile)
        events, strings = qprof.prepare(data,
            allow_unmapped_event_labels=args.allow_unmapped_event_labels,
            large_capture=args.large_capture)
        metadata = qprof.metadata_from_strings(strings)
        result = summarize_events(qprof.iter_prepared_records(events, strings,
            allow_unmapped_event_labels=args.allow_unmapped_event_labels))
        result['profile_sha256'] = hashlib.sha256(data).hexdigest()
        result['metadata'] = dict(process_id=metadata['process_id'], start_time=metadata['start_time'],
            command_sha256=hashlib.sha256(metadata['cmd'].encode('utf-8')).hexdigest())
        write_output(args.output, result)
    except (OSError, ValueError) as error:
        message = str(error) if isinstance(error, ValueError) else 'exclusive output unavailable'
        parser.exit(2, 'query activities: ' + message + '\n')


if __name__ == '__main__':
    main()
