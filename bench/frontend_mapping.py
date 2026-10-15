#!/usr/bin/env python3
"""Summarize validated native frontend evidence, without inventing work or span.

The public justrust graph validator is the authority for native receipt schemas.
This tool snapshots bounded original bytes, validates that immutable snapshot,
then computes descriptive timing summaries within each receipt's own epoch.
"""
import argparse
import collections
import hashlib
import json
import os
from pathlib import Path
import stat
import subprocess
import tempfile

MAX_BYTES = 64 * 1024 * 1024
LARGE_MAX_BYTES = 512 * 1024 * 1024
MAX_FILES = 20_000
STAGES = ('registered', 'notified', 'condvar_returned', 'resumed')


def intervals_summary(intervals):
    """Elapsed intervals only. Neither their sum nor union measures CPU work."""
    endpoints = []
    total = 0
    for start, end in intervals:
        if not isinstance(start, int) or not isinstance(end, int) or start < 0 or end < start:
            raise ValueError('invalid elapsed interval')
        total += end - start
        if end > start:
            endpoints += [(start, 1), (end, -1)]
    active = peak = union = 0
    previous = None
    for at, change in sorted(endpoints):
        if previous is not None and active:
            union += at - previous
        active += change
        peak = max(peak, active)
        previous = at
    return dict(count=len(intervals), sum_elapsed_ns=total, union_elapsed_ns=union,
                max_elapsed_ns=max((b - a for a, b in intervals), default=0),
                peak_overlapping_intervals=peak)


def summarize(query, assessment):
    """Only call after strict native validation. Never merge different clock epochs."""
    if query['capture_id'] != assessment['capture_id']:
        raise ValueError('assessment/capture mismatch')
    # graph reports lifecycle violations as successful assessments with issues,
    # not necessarily a failing exit status. Without per-wait validity authority
    # we cannot safely salvage intervals from an issue-bearing capture.
    if assessment.get('issues') != []:
        raise ValueError('native query assessment has issues or missing issue authority')
    waits = collections.defaultdict(dict)
    for event in query['wait_events']:
        if event['kind'] in waits[event['wait_id']]:
            raise ValueError('duplicate wait lifecycle stage')
        waits[event['wait_id']][event['kind']] = event
    phases = {name: [] for name in ('producer_notification', 'post_notification',
                                   'post_condvar', 'total')}
    reasons = collections.Counter()
    complete = 0
    for events in waits.values():
        if any(stage not in events for stage in STAGES):
            continue
        a, b, c, d = [events[stage]['at_ns'] for stage in STAGES]
        if not (a <= b <= c <= d):
            raise ValueError('nonmonotonic wait lifecycle')
        complete += 1
        reasons[events['notified']['reason']] += 1
        phases['producer_notification'].append((a, b))
        phases['post_notification'].append((b, d))
        phases['post_condvar'].append((c, d))
        phases['total'].append((a, d))
    return dict(
        capture_id=query['capture_id'],
        scope='validated-native-frontend-evidence-not-an-execution-span',
        admissions_complete=assessment['structural_complete'],
        waits_complete=assessment.get('wait_structural_complete', False),
        expected_waits=query['expected_waits'], complete_observed_waits=complete,
        notification_reasons=dict(sorted(reasons.items())),
        query_clock_wait_intervals={name: intervals_summary(values)
                                    for name, values in phases.items()},
        read_coverage=assessment.get('query_reads'),
        task_binding_coverage=assessment.get('task_bindings'),
        observed_read_records=len(query['query_reads']['events']),
        full_pipeline_complete=False, cpu_work_ns=None, execution_span_ns=None,
        theoretical_parallelism=None,
        limitations=[
            'Wait intervals overlap and include waiting/descheduling, not CPU work.',
            'Post-condvar elapsed can include resource reacquisition but is not attributed resource cost.',
            'Independent useful work may execute during a wait, so wait length is not critical-path cost.',
            'Query and pool timestamps use separate epochs and are not merged.',
            'Resolved publishing-job edges do not imply fresh provider execution.',
            'Unbound/lost reads, nonquery work and incomplete task/resource coverage prevent a frontend span.',
            'Semantic constraints and current implementation ordering are not interchangeable.',
        ])


def _read_regular(path, remaining, directory_fd=None):
    fd = os.open(path, os.O_RDONLY | os.O_NOFOLLOW | os.O_NONBLOCK,
                 dir_fd=directory_fd)
    try:
        info = os.fstat(fd)
        if not stat.S_ISREG(info.st_mode) or info.st_size > remaining:
            raise ValueError('receipt is not a bounded regular file')
        chunks = []
        count = 0
        while True:
            chunk = os.read(fd, min(1024 * 1024, remaining - count + 1))
            if not chunk:
                break
            count += len(chunk)
            if count > remaining:
                raise ValueError('capture exceeds byte budget')
            chunks.append(chunk)
        return b''.join(chunks)
    finally:
        os.close(fd)


def analyze(directory, justrust, large_capture=False):
    directory = Path(directory)
    if directory.is_symlink() or not directory.is_dir():
        raise ValueError('capture must be a nonsymlink directory')
    # Snapshot before validation, not afterward: mutations cannot substitute bytes
    # after the strict public consumer has endorsed a different receipt.
    originals = []
    used = 0
    byte_budget = LARGE_MAX_BYTES if large_capture else MAX_BYTES
    directory_fd = os.open(directory, os.O_RDONLY | os.O_NOFOLLOW | os.O_DIRECTORY)
    try:
        with os.scandir(directory_fd) as entries:
            for scanned, entry in enumerate(entries, 1):
                if scanned > MAX_FILES:
                    raise ValueError('capture exceeds directory entry budget')
                if not entry.name.endswith('.json'):
                    continue
                content = _read_regular(entry.name, byte_budget - used, directory_fd)
                used += len(content)
                originals.append((entry.name, content))
    finally:
        os.close(directory_fd)
    if not originals:
        raise ValueError('empty capture is unknown, not successful evidence')
    # Capture IDs alone cannot prove that the native directory router consumed
    # a source: owner/envelope filenames are deliberately skipped by graph.
    # Route every query source under a neutral snapshot name, retaining its exact
    # bytes. Leave auxiliary documents under their original native routing names.
    query_sources = {}
    source_receipts = {}
    snapshot_names = {name for name, _ in originals}
    for name, content in originals:
        receipt = json.loads(content)
        if receipt.get('producer') != 'rustc-query-admission':
            continue
        capture_id = receipt['capture_id']
        if not isinstance(capture_id, str):
            raise ValueError('query capture identity must be a string')
        if capture_id in query_sources:
            raise ValueError('duplicate query source capture identity')
        if receipt.get('schema') != 4:
            raise ValueError('frontend report requires native schema4 evidence')
        query_sources[capture_id] = name
        source_receipts[name] = receipt
    neutral_names = {}
    next_name = 0
    for name in source_receipts:
        while True:
            candidate = f'frontend-query-{next_name}.json'
            next_name += 1
            if candidate not in snapshot_names:
                break
        snapshot_names.add(candidate)
        neutral_names[name] = candidate
    with tempfile.TemporaryDirectory(prefix='jr-frontend-snapshot-',
                                     dir=os.environ.get('JCODE_SCRATCH_DIR')) as temporary:
        snapshot = Path(temporary)
        for name, content in originals:
            destination = snapshot / neutral_names.get(name, name)
            with destination.open('xb') as output:
                output.write(content)
            destination.chmod(0o400)
        command = [str(justrust), 'graph', str(snapshot), '--json']
        if large_capture:
            command.append('--large-capture')
        result = subprocess.run(command, capture_output=True, text=True, timeout=300)
        if result.returncode:
            raise ValueError('native graph validation failed:\n' + result.stdout + '\n' + result.stderr)
        assessments = json.loads(result.stdout)
        indexed = {}
        for assessment in assessments:
            if assessment['producer'] == 'rustc-query-admission':
                key = assessment['capture_id']
                if key in indexed:
                    raise ValueError('duplicate query capture authority')
                indexed[key] = assessment
        if set(indexed) != set(query_sources):
            raise ValueError('query source/validated authority mismatch')
        summaries = []
        inputs = []
        for name, content in sorted(originals):
            inputs.append(dict(name=name, sha256=hashlib.sha256(content).hexdigest(),
                               bytes=len(content)))
            if name not in source_receipts:
                continue
            receipt = source_receipts[name]
            summaries.append(summarize(receipt, indexed[receipt['capture_id']]))
        if not summaries:
            raise ValueError('no native frontend receipt')
        return dict(schema=1, scope='native-frontend-capture-diagnostics',
                    large_capture=large_capture,
                    inputs=inputs, captures=summaries,
                    full_pipeline_complete=False, cpu_work_ns=None,
                    execution_span_ns=None, theoretical_parallelism=None)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--capture', type=Path, required=True)
    parser.add_argument('--justrust', default='justrust')
    parser.add_argument('--large-capture', action='store_true',
                        help='Explicitly allow up to 512 MiB; forwards the public validator opt-in')
    parser.add_argument('--output', type=Path, required=True)
    args = parser.parse_args()
    try:
        report = analyze(args.capture, args.justrust, args.large_capture)
        encoded = (json.dumps(report, sort_keys=True, indent=2) + '\n').encode()
        fd = os.open(args.output, os.O_WRONLY | os.O_CREAT | os.O_EXCL, 0o600)
        with os.fdopen(fd, 'wb') as output:
            output.write(encoded)
    except (ValueError, OSError, KeyError, subprocess.SubprocessError) as error:
        parser.exit(2, 'frontend mapping: ' + str(error) + '\n')


if __name__ == '__main__':
    main()
