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
CACHE_CALL_SCOPE = 'observed-cache-result-availability-not-execution-dag'
CACHE_CALL_COUNTS = (
    'observed_requests', 'observed_cache_hits', 'orphan_cache_hits',
    'resolved_requests', 'unresolved_requests', 'distinct_publishing_job_relations',
    'lost_events',
)
CACHE_CALL_REASONS = (
    'feed_request', 'interrupted_request', 'missing_request_terminal',
    'ambiguous_cache_hits', 'no_cache_hit', 'incompatible_cache_hit_route',
    'missing_context_or_consumer', 'capture_incomplete', 'no_prior_producer',
    'producer_published_too_late', 'job_endpoint_unavailable',
    'admission_authority_incomplete',
)
DISPATCH_RESULT_SCOPE = 'observed-dispatch-result-availability-not-execution-dag'
DISPATCH_RESULT_COUNTS = (
    'observed_requests', 'observed_dispatch_resolutions', 'resolved_requests',
    'unresolved_requests', 'distinct_publishing_job_relations', 'lost_events',
)
DISPATCH_RESULT_REASONS = (
    'feed_request', 'interrupted_request', 'missing_request_terminal',
    'cache_hit_request', 'no_dispatch_resolution', 'null_dep_node_index',
    'missing_context_or_consumer', 'capture_incomplete', 'no_prior_producer',
    'producer_published_too_late', 'job_endpoint_unavailable',
    'admission_authority_incomplete',
)


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


def summarize(query, assessment, timing=None):
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
    summary = dict(
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
    if timing is not None:
        summary['clock_alignment'] = timing['clock_alignment']
        if 'observed_cpu' in timing:
            summary['observed_cpu'] = timing['observed_cpu']
        summary['limitations'][3] = (
            'Native clock alignment does not establish CPU coverage or an execution span; '
            'query and pool intervals are not merged.')
    return summary


def _timing_summary(query, assessment, pool_authorities):
    """Copy public assessments, never raw timing or inferred clock/CPU claims."""
    if query['schema'] not in (5, 6):
        return None
    alignment = assessment.get('clock_alignment')
    if query['schema'] == 6:
        if 'execution_clock' not in query:
            raise ValueError('schema6 requires mandatory nullable execution clock')
    if (not isinstance(alignment, dict)
            or type(alignment.get('aligned')) is not bool
            or not isinstance(alignment.get('issues'), list)
            or any(not isinstance(issue, str) for issue in alignment['issues'])
            or (alignment['aligned'] and alignment['issues'])):
        raise ValueError('schema5/6 requires native clock alignment authority')
    if query['schema'] == 6 and query['execution_clock'] is None and alignment['aligned']:
        raise ValueError('schema6 null clock cannot have aligned native authority')
    timing = dict(clock_alignment=alignment)
    reference = query['task_bindings']['pool']
    authority = pool_authorities.get(reference['capture_id']) if reference else None
    if authority is None:
        if alignment['aligned']:
            raise ValueError('aligned query has no matching CPU pool authority')
        return timing
    source, cpu = authority
    if (source['query_capture_id'] != query['capture_id']
            or type(reference['observer']) is not int
            or reference['observer'] != source['observer']):
        raise ValueError('query/CPU pool qualified identity mismatch')
    # The pool's checked measurement sums and partial/failed coverage stay as
    # reported. In particular, do not upgrade query_clock_aligned or CPU coverage.
    timing['observed_cpu'] = cpu
    return timing


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


def _run_graph(snapshot, justrust, large_capture, cache_call_provenance, graph_output,
               dispatch_result_provenance=False):
    command = [str(justrust), 'graph', str(snapshot), '--json']
    if large_capture:
        command.append('--large-capture')
    if cache_call_provenance:
        command.append('--cache-call-provenance')
    if dispatch_result_provenance:
        command.append('--dispatch-result-provenance')
    if cache_call_provenance or dispatch_result_provenance:
        # Reserve before invoking the validator, so collisions never overwrite an
        # existing artifact or perform an unnecessary potentially expensive run.
        fd = os.open(graph_output, os.O_WRONLY | os.O_CREAT | os.O_EXCL, 0o600)
        with os.fdopen(fd, 'wb') as output:
            os.fchmod(output.fileno(), 0o600)
            result = subprocess.run(command, capture_output=True, text=False, timeout=300)
            if not isinstance(result.stdout, bytes):
                raise ValueError('exact graph artifact requires binary native stdout')
            output.write(result.stdout)
        artifact = dict(name=Path(graph_output).name, bytes=len(result.stdout),
                        sha256=hashlib.sha256(result.stdout).hexdigest(),
                        format='justrust-graph-json-stdout')
    else:
        result = subprocess.run(command, capture_output=True, text=True, timeout=300)
        artifact = None
    if result.returncode:
        def diagnostic(value):
            return value.decode('utf-8', errors='replace') if isinstance(value, bytes) else value
        raise ValueError('native graph validation failed:\n' + diagnostic(result.stdout)
                         + '\n' + diagnostic(result.stderr))
    return json.loads(result.stdout), artifact


def _cache_call_summary(assessment):
    """Native availability relations only, never execution or semantic edges."""
    calls = assessment.get('cache_call_provenance')
    if (not isinstance(calls, dict) or calls.get('scope') != CACHE_CALL_SCOPE
            or type(calls.get('records_complete')) is not bool
            or not isinstance(calls.get('relations'), list)
            or not isinstance(calls.get('unresolved_reasons'), dict)):
        raise ValueError('missing or malformed native cache-call provenance authority')
    reasons = calls['unresolved_reasons']
    for container, fields in ((calls, CACHE_CALL_COUNTS), (reasons, CACHE_CALL_REASONS)):
        if any(type(container.get(key)) is not int or container[key] < 0 for key in fields):
            raise ValueError('invalid native cache-call provenance counters')
    if (calls['resolved_requests'] != len(calls['relations'])
            or calls['resolved_requests'] + calls['unresolved_requests'] != calls['observed_requests']
            or sum(reasons[key] for key in CACHE_CALL_REASONS) != calls['unresolved_requests']):
        raise ValueError('native cache-call provenance partition mismatch')
    # Full native rows are retained in the exact public graph artifact. Do not
    # reserialize them or turn publishing-job relations into graph edges.
    return dict(scope=calls['scope'], records_complete=calls['records_complete'],
                **{key: calls[key] for key in CACHE_CALL_COUNTS},
                unresolved_reasons={key: reasons[key] for key in CACHE_CALL_REASONS},
                stored_relation_rows=len(calls['relations']))


def _dispatch_result_summary(assessment):
    """Copy strict public terminal-result facts, never inferred execution edges."""
    result = assessment.get('dispatch_result_provenance')
    if (not isinstance(result, dict) or result.get('scope') != DISPATCH_RESULT_SCOPE
            or type(result.get('records_complete')) is not bool
            or not isinstance(result.get('relations'), list)
            or not isinstance(result.get('unresolved_reasons'), dict)):
        raise ValueError('missing or malformed native dispatch-result provenance authority')
    reasons = result['unresolved_reasons']
    for container, fields in ((result, DISPATCH_RESULT_COUNTS),
                              (reasons, DISPATCH_RESULT_REASONS)):
        if any(type(container.get(key)) is not int or container[key] < 0 for key in fields):
            raise ValueError('invalid native dispatch-result provenance counters')
    if (result['resolved_requests'] != len(result['relations'])
            or result['resolved_requests'] + result['unresolved_requests'] != result['observed_requests']
            or sum(reasons[key] for key in DISPATCH_RESULT_REASONS) != result['unresolved_requests']):
        raise ValueError('native dispatch-result provenance partition mismatch')
    return dict(scope=result['scope'], records_complete=result['records_complete'],
                **{key: result[key] for key in DISPATCH_RESULT_COUNTS},
                unresolved_reasons={key: reasons[key] for key in DISPATCH_RESULT_REASONS},
                stored_relation_rows=len(result['relations']))


def analyze(directory, justrust, large_capture=False, cache_call_provenance=False,
            graph_output=None, dispatch_result_provenance=False):
    provenance = cache_call_provenance or dispatch_result_provenance
    if provenance != (graph_output is not None):
        raise ValueError('--cache-call-provenance or --dispatch-result-provenance '
                         'requires companion --graph-output and vice versa')
    directory = Path(directory)
    if directory.is_symlink() or not directory.is_dir():
        raise ValueError('capture must be a nonsymlink directory')
    if provenance:
        capture_root = directory.resolve()
        artifact_path = Path(graph_output).resolve()
        if artifact_path == capture_root or capture_root in artifact_path.parents:
            raise ValueError('graph output must be outside the original capture directory')
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
    # Route query and timing-pool sources under neutral snapshot names, retaining
    # exact bytes. Leave auxiliary documents under their native routing names.
    query_sources = {}
    source_receipts = {}
    pool_sources = {}
    neutral_sources = []
    snapshot_names = {name for name, _ in originals}
    for name, content in originals:
        receipt = json.loads(content)
        if receipt.get('producer') == 'rustc-thread-pool-join' and receipt.get('schema') == 3:
            capture_id = receipt['capture_id']
            if not isinstance(capture_id, str) or capture_id in pool_sources:
                raise ValueError('invalid or duplicate CPU pool source capture identity')
            pool_sources[capture_id] = receipt
            neutral_sources.append(name)
        if receipt.get('producer') != 'rustc-query-admission':
            continue
        capture_id = receipt['capture_id']
        if not isinstance(capture_id, str):
            raise ValueError('query capture identity must be a string')
        if capture_id in query_sources:
            raise ValueError('duplicate query source capture identity')
        allowed_schemas = (4, 5, 6) if dispatch_result_provenance else (4, 5)
        if type(receipt.get('schema')) is not int or receipt['schema'] not in allowed_schemas:
            raise ValueError('frontend report requires native schema4/5 evidence '
                             'or explicit --dispatch-result-provenance for schema6')
        query_sources[capture_id] = name
        source_receipts[name] = receipt
        neutral_sources.append(name)
    neutral_names = {}
    next_name = 0
    for name in neutral_sources:
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
        assessments, artifact = _run_graph(snapshot, justrust, large_capture,
                                          cache_call_provenance, graph_output,
                                          dispatch_result_provenance)
        indexed = {}
        pool_indexed = {}
        for assessment in assessments:
            if assessment['producer'] == 'rustc-query-admission':
                key = assessment['capture_id']
                if key in indexed:
                    raise ValueError('duplicate query capture authority')
                indexed[key] = assessment
            elif assessment['producer'] == 'rustc-thread-pool-join':
                key = assessment['capture_id']
                if key in pool_indexed:
                    raise ValueError('duplicate CPU pool capture authority')
                pool_indexed[key] = assessment
        if set(indexed) != set(query_sources):
            raise ValueError('query source/validated authority mismatch')
        pool_authorities = {}
        timing_ids = {key for key, value in pool_indexed.items()
                      if value.get('schema') == 3 or value.get('observed_cpu') is not None}
        if timing_ids != set(pool_sources):
            raise ValueError('CPU pool source/validated authority mismatch')
        for key, source in pool_sources.items():
            authority = pool_indexed[key]
            cpu = authority.get('observed_cpu')
            expected_bridge = dict(query_capture_id=source['query_capture_id'],
                                   pool_capture_id=key, pool_observer=source['observer'],
                                   pool_epoch_in_query_ns=0)
            bridge = cpu.get('clock_bridge') if isinstance(cpu, dict) else None
            if (type(authority.get('schema')) is not int or authority['schema'] != 3
                    or type(source['schema']) is not int
                    or not isinstance(source['query_capture_id'], str)
                    or type(source['observer']) is not int or source['observer'] <= 0
                    or not isinstance(bridge, dict)
                    or type(bridge.get('pool_observer')) is not int
                    or type(bridge.get('pool_epoch_in_query_ns')) is not int
                    or bridge != expected_bridge):
                raise ValueError('CPU pool schema or qualified clock bridge mismatch')
            pool_authorities[key] = (source, cpu)
        summaries = []
        inputs = []
        for name, content in sorted(originals):
            inputs.append(dict(name=name, sha256=hashlib.sha256(content).hexdigest(),
                               bytes=len(content)))
            if name not in source_receipts:
                continue
            receipt = source_receipts[name]
            authority = indexed[receipt['capture_id']]
            if (type(authority.get('schema')) is not int
                    or authority['schema'] != receipt['schema']):
                raise ValueError('query source/validated schema mismatch')
            timing = _timing_summary(receipt, authority, pool_authorities)
            summary = summarize(receipt, authority, timing)
            if cache_call_provenance:
                summary['cache_call_provenance'] = _cache_call_summary(authority)
            if dispatch_result_provenance:
                if authority['schema'] != 6:
                    raise ValueError('dispatch-result provenance requires native schema6 authority')
                summary['dispatch_result_provenance'] = _dispatch_result_summary(authority)
            summaries.append(summary)
        if not summaries:
            raise ValueError('no native frontend receipt')
        report = dict(schema=1, scope='native-frontend-capture-diagnostics',
                      large_capture=large_capture,
                      inputs=inputs, captures=summaries,
                      full_pipeline_complete=False, cpu_work_ns=None,
                      execution_span_ns=None, theoretical_parallelism=None)
        if artifact is not None:
            report['graph_output'] = artifact
        return report


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--capture', type=Path, required=True)
    parser.add_argument('--justrust', default='justrust')
    parser.add_argument('--large-capture', action='store_true',
                        help='Explicitly allow up to 512 MiB; forwards the public validator opt-in')
    parser.add_argument('--cache-call-provenance', action='store_true',
                        help='Opt in to native cache-result availability relations, not execution edges')
    parser.add_argument('--dispatch-result-provenance', action='store_true',
                        help='Opt in to native schema6 dispatch-result availability, not execution edges')
    parser.add_argument('--graph-output', type=Path,
                        help='Required with either provenance flag: exclusive full native graph JSON artifact outside the capture directory')
    parser.add_argument('--output', type=Path, required=True)
    args = parser.parse_args()
    try:
        provenance = args.cache_call_provenance or args.dispatch_result_provenance
        if provenance or args.graph_output is not None:
            if not provenance or args.graph_output is None:
                raise ValueError('--cache-call-provenance or --dispatch-result-provenance '
                                 'requires companion --graph-output and vice versa')
            if args.output.resolve() == args.graph_output.resolve():
                raise ValueError('report and graph output paths must differ')
            if os.path.lexists(args.output):
                raise ValueError('report output already exists')
            report = analyze(args.capture, args.justrust, args.large_capture,
                             cache_call_provenance=args.cache_call_provenance,
                             graph_output=args.graph_output,
                             dispatch_result_provenance=args.dispatch_result_provenance)
        else:
            report = analyze(args.capture, args.justrust, args.large_capture)
        encoded = (json.dumps(report, sort_keys=True, indent=2) + '\n').encode()
        fd = os.open(args.output, os.O_WRONLY | os.O_CREAT | os.O_EXCL, 0o600)
        with os.fdopen(fd, 'wb') as output:
            output.write(encoded)
    except (ValueError, OSError, KeyError, subprocess.SubprocessError) as error:
        parser.exit(2, 'frontend mapping: ' + str(error) + '\n')


if __name__ == '__main__':
    main()
