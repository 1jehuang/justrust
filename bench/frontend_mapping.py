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
               dispatch_result_provenance=False, query_cpu_ownership=False,
               observed_task_dag=False, execution_flow_dag=False, latch_readiness_dag=False):
    command = [str(justrust), 'graph', str(snapshot), '--json']
    if large_capture:
        command.append('--large-capture')
    if cache_call_provenance:
        command.append('--cache-call-provenance')
    if dispatch_result_provenance:
        command.append('--dispatch-result-provenance')
    if query_cpu_ownership:
        command.append('--query-cpu-ownership')
    if observed_task_dag:
        command.append('--observed-task-dag')
    if execution_flow_dag:
        command.append('--execution-flow-dag')
    if latch_readiness_dag:
        command.append('--latch-readiness-dag')
    if (cache_call_provenance or dispatch_result_provenance or query_cpu_ownership
            or observed_task_dag or execution_flow_dag or latch_readiness_dag):
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


def _query_cpu_summary(assessment):
    """Copy admitted execution CPU facts, never infer providers, span or ancestry."""
    cpu = assessment.get('query_cpu_ownership')
    if (not isinstance(cpu, dict) or cpu.get('scope') != 'observed-query-execution-cpu'
            or type(cpu.get('records_complete')) is not bool
            or type(cpu.get('measured_segments')) is not int or cpu['measured_segments'] < 0
            or not isinstance(cpu.get('queries'), list)
            or not isinstance(cpu.get('issues'), list)
            or any(not isinstance(issue, str) for issue in cpu['issues'])):
        raise ValueError('missing or malformed native query CPU ownership authority')
    total = cpu.get('query_execution_cpu_ns')
    if total is not None and (type(total) is not int or total < 0):
        raise ValueError('invalid native query CPU total')
    identities = set()
    kinds = collections.Counter()
    for row in cpu['queries']:
        if (not isinstance(row, dict)
                or any(type(row.get(key)) is not int or row[key] <= 0
                       for key in ('query_context', 'query_job'))
                or type(row.get('cpu_ns')) is not int or row['cpu_ns'] < 0):
            raise ValueError('invalid native query CPU row')
        name = row.get('query_name')
        if (not isinstance(name, str) or not 1 <= len(name) <= 128
                or not name.isascii() or not name.isidentifier()):
            raise ValueError('invalid native query CPU name')
        identity = row['query_context'], row['query_job']
        if identity in identities:
            raise ValueError('duplicate native query CPU owner')
        identities.add(identity)
        kinds[name] += row['cpu_ns']
    ranked = sorted(cpu['queries'], key=lambda row: (
        -row['cpu_ns'], row['query_context'], row['query_job']))
    if total is not None and sum(row['cpu_ns'] for row in ranked) != total:
        raise ValueError('native query CPU partition mismatch')
    if total is None and ranked:
        raise ValueError('unknown native query CPU cannot expose numeric rankings')
    if cpu['records_complete'] and (total is None or cpu['issues']):
        raise ValueError('complete native query CPU has unknown or issue-bearing authority')
    ranked_kinds = [dict(query_name=name, cpu_ns=cost)
                    for name, cost in sorted(kinds.items(), key=lambda row: (-row[1], row[0]))]
    return dict(scope=cpu['scope'], records_complete=cpu['records_complete'],
                query_execution_cpu_ns=total, measured_segments=cpu['measured_segments'],
                observed_query_owners=len(ranked), top_queries=ranked[:20],
                omitted_query_owners=max(0, len(ranked) - 20),
                observed_query_kinds=len(ranked_kinds), top_query_kinds=ranked_kinds[:20],
                omitted_query_kinds=max(0, len(ranked_kinds) - 20), issues=cpu['issues'])


def _execution_flow_summary(assessment, *, field='execution_flow_dag',
                            scope='observed-query-and-join-control-flow', gate_kinds=()):
    """Bound public known-chain diagnostics, never promote them to full span."""
    flow = assessment.get(field)
    counts = ('nodes', 'edges', 'scopes', 'active_scopes', 'lost_nodes', 'lost_edges',
              'lost_scopes', 'assigned_cpu_segments', 'unassigned_cpu_segments',
              'missing_measurement_references', 'unavailable_cpu_segments', 'cut_nodes')
    totals = ('unassigned_known_cpu_ns', 'known_cpu_work_ns', 'known_query_cpu_ns',
              'known_task_cpu_ns', 'known_opaque_cpu_ns', 'longest_known_chain_cpu_ns')
    flags = ('records_complete', 'full_pipeline_complete', 'flow_complete')
    if (not isinstance(flow, dict)
            or flow.get('scope') != scope
            or any(type(flow.get(key)) is not bool for key in flags)
            or flow['full_pipeline_complete'] or flow['flow_complete']
            or any(type(flow.get(key)) is not int or flow[key] < 0 for key in counts)
            or any(key not in flow or (flow[key] is not None and
                   (type(flow[key]) is not int or flow[key] < 0)) for key in totals)
            or any(key not in flow or flow[key] is not None for key in
                   ('cpu_work_ns', 'execution_span_ns', 'parallelism'))
            or any(not isinstance(flow.get(key), list) for key in
                   ('longest_known_chain_path', 'longest_known_chain_steps', 'issues', 'limitations'))
            or any(not isinstance(text, str) for key in ('issues', 'limitations') for text in flow[key])
            or not isinstance(flow.get('pool_capture_id'), str) or not flow['pool_capture_id']
            or flow['pool_capture_id'] != assessment.get('capture_id')
            or type(flow.get('pool_observer')) is not int or flow['pool_observer'] <= 0):
        raise ValueError('missing or malformed execution flow DAG authority')
    cpu = assessment.get('observed_cpu')
    bridge = cpu.get('clock_bridge') if isinstance(cpu, dict) else None
    if (not isinstance(bridge, dict) or flow['pool_observer'] != bridge.get('pool_observer')
            or type(bridge.get('pool_observer')) is not int
            or flow['pool_capture_id'] != bridge.get('pool_capture_id')):
        raise ValueError('execution flow qualified pool identity mismatch')
    if (flow['assigned_cpu_segments'] > flow['nodes']
            or flow['cut_nodes'] > flow['nodes']
            or flow['unavailable_cpu_segments'] > flow['assigned_cpu_segments']):
        raise ValueError('execution flow inventory mismatch')
    if flow['records_complete'] and (flow['issues'] or any(flow[key] for key in
            ('active_scopes', 'lost_nodes', 'lost_edges', 'lost_scopes',
             'unassigned_cpu_segments', 'missing_measurement_references', 'unavailable_cpu_segments'))):
        raise ValueError('complete execution flow has incomplete authority')
    categories = [flow[key] for key in ('known_query_cpu_ns', 'known_task_cpu_ns', 'known_opaque_cpu_ns')]
    known = flow['known_cpu_work_ns']
    if ((known is None and any(value is not None for value in categories))
            or (known is not None and (all(value is None for value in categories)
                or sum(value for value in categories if value is not None) != known))):
        raise ValueError('execution flow known CPU partition mismatch')
    path, steps = flow['longest_known_chain_path'], flow['longest_known_chain_steps']
    chain = flow['longest_known_chain_cpu_ns']
    if (len(path) != len(steps) or len(path) > flow['nodes']
            or any(type(node) is not int or node <= 0 for node in path)
            or len(set(path)) != len(path) or max(0, len(path) - 1) > flow['edges']
            or (chain is None) != (known is None)
            or (chain is None and path) or (chain is not None and not path)):
        raise ValueError('invalid execution flow chain path')
    segments = set()
    named = []
    kinds = collections.Counter()
    previous_at = None
    for node, step in zip(path, steps):
        if (not isinstance(step, dict) or type(step.get('node_id')) is not int
                or step['node_id'] != node
                or step.get('kind') not in ('cpu_segment', 'scope_enter', 'scope_exit',
                                            'fork', 'task_enter', 'task_exit', 'join') + gate_kinds
                or type(step.get('at_ns')) is not int or step['at_ns'] < 0
                or (previous_at is not None and step['at_ns'] < previous_at)
                or type(step.get('cpu_ns')) is not int or step['cpu_ns'] < 0
                or any(key not in step or (step[key] is not None and
                       (type(step[key]) is not int or step[key] <= 0)) for key in
                       ('segment_id', 'scope_id', 'task', 'query_context', 'query_job'))):
            raise ValueError('invalid execution flow chain step')
        previous_at = step['at_ns']
        if (step['query_context'] is None) != (step['query_job'] is None):
            raise ValueError('invalid execution flow qualified query identity')
        if step['query_context'] is not None and step['scope_id'] is None:
            raise ValueError('execution flow query lacks scope identity')
        if step['kind'] == 'cpu_segment':
            if step['segment_id'] is None or step['segment_id'] in segments:
                raise ValueError('invalid execution flow segment identity')
            segments.add(step['segment_id'])
        elif step['segment_id'] is not None or step['cpu_ns'] != 0:
            raise ValueError('execution flow gate has CPU measurement')
        if 'query_name' not in step:
            raise ValueError('execution flow name must be explicitly nullable')
        name = step['query_name']
        if name is not None:
            if (step['kind'] != 'cpu_segment' or step['query_context'] is None
                    or not isinstance(name, str) or not 1 <= len(name) <= 128
                    or not name.isascii() or not name.isidentifier()):
                raise ValueError('invalid execution flow query name authority')
            named.append({key: step[key] for key in ('node_id', 'kind', 'at_ns',
                'segment_id', 'scope_id', 'task', 'query_context', 'query_job', 'query_name', 'cpu_ns')})
            kinds[name] += step['cpu_ns']
    if (len(segments) > flow['assigned_cpu_segments']
            or (chain is not None and not segments)
            or (named and (flow['known_query_cpu_ns'] is None
                           or sum(kinds.values()) > flow['known_query_cpu_ns']))
            or (chain is not None and (sum(step['cpu_ns'] for step in steps) != chain
                or (known is None and segments) or (known is not None and chain > known)))):
        raise ValueError('execution flow chain CPU mismatch')
    ranked = sorted(named, key=lambda row: (-row['cpu_ns'], row['node_id']))
    ranked_kinds = [dict(query_name=name, cpu_ns=cost)
                    for name, cost in sorted(kinds.items(), key=lambda row: (-row[1], row[0]))]
    return dict(scope=flow['scope'], pool_capture_id=flow['pool_capture_id'],
                pool_observer=flow['pool_observer'],
                **{key: flow[key] for key in (*flags, *counts, *totals)},
                cpu_work_ns=None, execution_span_ns=None, parallelism=None,
                longest_known_chain_nodes=len(path),
                longest_known_chain_cpu_segments=len(segments),
                named_chain_cpu_segments=len(ranked),
                named_chain_cpu_ns=sum(kinds.values()) if chain is not None else None,
                top_named_chain_cpu_segments=ranked[:20],
                omitted_named_chain_cpu_segments=max(0, len(ranked) - 20),
                chain_query_kinds=len(ranked_kinds), top_chain_query_kinds=ranked_kinds[:20],
                omitted_chain_query_kinds=max(0, len(ranked_kinds) - 20),
                issues=flow['issues'], limitations=flow['limitations'])


def _latch_readiness_summary(assessment):
    """Retain actual same-latch constraints, not publication latency or full span."""
    readiness = assessment.get('latch_readiness_dag')
    flags = ('ready_records_complete', 'wait_records_complete')
    counts = ('readiness_rows', 'active_readiness', 'lost_readiness', 'wait_attempts',
              'active_wait_attempts', 'lost_wait_attempts', 'matched_readiness_transfers',
              'unbound_wait_attempts', 'unavailable_readiness_references')
    if (not isinstance(readiness, dict)
            or any(type(readiness.get(key)) is not bool for key in flags)
            or any(type(readiness.get(key)) is not int or readiness[key] < 0 for key in counts)
            or readiness['matched_readiness_transfers'] > readiness['wait_attempts']
            or readiness['unbound_wait_attempts'] > readiness['wait_attempts']):
        raise ValueError('missing or malformed latch readiness DAG authority')
    if readiness.get('records_complete') is True and (
            not all(readiness[key] for key in flags) or any(readiness[key] for key in
            ('active_readiness', 'lost_readiness', 'active_wait_attempts', 'lost_wait_attempts'))):
        raise ValueError('complete latch readiness DAG has incomplete independent inventories')
    summary = _execution_flow_summary(assessment, field='latch_readiness_dag',
        scope='observed-query-join-and-latch-control-flow',
        gate_kinds=('readiness_prerequisite', 'wait_begin', 'wait_consumed'))
    if any(step['kind'] == 'readiness_prerequisite'
           and any(step[key] is not None for key in
                   ('scope_id', 'task', 'query_context', 'query_job'))
           for step in readiness['longest_known_chain_steps']):
        raise ValueError('readiness prerequisite must not acquire caller query or task attribution')
    # Matched/unbound are not a partition of attempts. Active, poisoned, cyclic,
    # unwound or unqualified observations remain distinct native unknowns. One
    # missing source can invalidate multiple endpoint references. Never infer
    # zero work, successful wake visibility or invented reason counters.
    summary.update({key: readiness[key] for key in (*flags, *counts)})
    return summary


def _observed_task_dag_summary(assessment):
    """Retain only the public validator's scoped causal task diagnostics."""
    dag = assessment.get('observed_task_dag')
    flags = ('topology_complete', 'timing_records_complete', 'records_complete',
             'query_authority_valid')
    totals = ('attached_cpu_ns', 'known_attached_cpu_ns', 'excluded_task_null_cpu_ns',
              'excluded_query_cpu_ns', 'excluded_opaque_cpu_ns')
    counts = ('excluded_task_null_segments', 'unmeasured_segments')
    if (not isinstance(dag, dict) or dag.get('scope') != 'observed-fork-join-task-dag'
            or any(type(dag.get(key)) is not bool for key in flags)
            or any(key not in dag or (dag[key] is not None
                   and (type(dag[key]) is not int or dag[key] < 0)) for key in totals)
            or any(type(dag.get(key)) is not int or dag[key] < 0 for key in counts)
            or any(not isinstance(dag.get(key), list) for key in
                   ('nodes', 'edges', 'components', 'issues'))
            or any(not isinstance(issue, str) for issue in dag['issues'])):
        raise ValueError('missing or malformed observed task DAG authority')
    if dag['records_complete'] and (not dag['topology_complete']
                                   or not dag['timing_records_complete'] or dag['issues']):
        raise ValueError('complete observed task DAG lacks complete authority')
    nodes = {}
    for node in dag['nodes']:
        task = node.get('task')
        if type(task) is not int or task <= 0 or task in nodes:
            raise ValueError('invalid observed task DAG node identity')
        for key in ('cpu_ns', 'known_cpu_ns'):
            if key not in node or (node[key] is not None
                                  and (type(node[key]) is not int or node[key] < 0)):
                raise ValueError('invalid observed task DAG node cost')
        nodes[task] = node
    edges = set()
    for edge in dag['edges']:
        if (edge.get('kind') not in ('fork', 'join')
                or type(edge.get('from')) is not int or type(edge.get('to')) is not int
                or edge['from'] not in nodes or edge['to'] not in nodes
                or (edge['from'], edge['to']) in edges):
            raise ValueError('invalid observed task DAG edge')
        edges.add((edge['from'], edge['to']))
    roots = set()
    for component in dag['components']:
        if (type(component.get('root')) is not int or component['root'] not in nodes
                or component['root'] in roots or type(component.get('records_complete')) is not bool
                or any(type(component.get(key)) is not int or component[key] < 0
                       for key in ('tasks', 'edges'))
                or not isinstance(component.get('chain'), list)):
            raise ValueError('invalid observed task DAG component')
        roots.add(component['root'])
        for key in ('cpu_ns', 'known_cpu_ns', 'longest_recorded_fork_join_chain_cpu_ns',
                    'known_cpu_chain_ns'):
            if key not in component or (component[key] is not None
                    and (type(component[key]) is not int or component[key] < 0)):
                raise ValueError('invalid observed task DAG chain cost')
        chain = component['chain']
        if (any(type(task) is not int or task not in nodes for task in chain)
                or len(set(chain)) != len(chain)
                or any(pair not in edges for pair in zip(chain, chain[1:]))
                or (component['known_cpu_chain_ns'] is None and chain)):
            raise ValueError('invalid observed task DAG chain authority')
    if (sum(row['tasks'] for row in dag['components']) != len(nodes)
            or sum(row['edges'] for row in dag['components']) != len(edges)):
        raise ValueError('observed task DAG component inventory mismatch')
    ranked = sorted(dag['components'], key=lambda row: (
        row['known_cpu_chain_ns'] is None, -(row['known_cpu_chain_ns'] or 0), row['root']))
    return dict(scope=dag['scope'], **{key: dag[key] for key in (*flags, *totals, *counts)},
                observed_tasks=len(nodes), observed_edges=len(edges),
                observed_components=len(ranked), top_components=ranked[:20],
                omitted_components=max(0, len(ranked) - 20), issues=dag['issues'],
                limitations=[
                    'Only explicitly instrumented native fork/join edges are represented.',
                    'CPU with no genuine pool-task owner is excluded, including named query CPU.',
                    'Known CPU chains sum retained measurements, not a full execution span.',
                    'Missing records and instrumentation gaps are unknown, not zero work.',
                    'No work/span ratio or useful-thread ceiling follows from this partial DAG.',
                ])


def analyze(directory, justrust, large_capture=False, cache_call_provenance=False,
            graph_output=None, dispatch_result_provenance=False, query_cpu_ownership=False,
            observed_task_dag=False, execution_flow_dag=False, latch_readiness_dag=False):
    provenance = (cache_call_provenance or dispatch_result_provenance or query_cpu_ownership
                  or observed_task_dag or execution_flow_dag or latch_readiness_dag)
    if provenance != (graph_output is not None):
        if latch_readiness_dag:
            raise ValueError('--latch-readiness-dag requires companion --graph-output and vice versa')
        raise ValueError('--cache-call-provenance or --dispatch-result-provenance '
                         'or --query-cpu-ownership or --observed-task-dag or --execution-flow-dag requires companion --graph-output and vice versa')
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
        if receipt.get('producer') == 'rustc-thread-pool-join' and receipt.get('schema') in (3, 4, 5, 6):
            if receipt['schema'] == 6 and not latch_readiness_dag:
                raise ValueError('pool6 requires explicit --latch-readiness-dag')
            if receipt['schema'] == 5 and not execution_flow_dag:
                raise ValueError('pool5 requires explicit --execution-flow-dag')
            if receipt['schema'] == 4 and not query_cpu_ownership:
                raise ValueError('pool4 requires explicit --query-cpu-ownership')
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
        allowed_schemas = (4, 5, 6) if (dispatch_result_provenance or query_cpu_ownership
                                               or observed_task_dag or execution_flow_dag or latch_readiness_dag) else (4, 5)
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
                                          dispatch_result_provenance, query_cpu_ownership,
                                          observed_task_dag, execution_flow_dag, latch_readiness_dag)
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
                      if value.get('schema') in (3, 4, 5, 6) or value.get('observed_cpu') is not None}
        if timing_ids != set(pool_sources):
            raise ValueError('CPU pool source/validated authority mismatch')
        for key, source in pool_sources.items():
            authority = pool_indexed[key]
            cpu = authority.get('observed_cpu')
            expected_bridge = dict(query_capture_id=source['query_capture_id'],
                                   pool_capture_id=key, pool_observer=source['observer'],
                                   pool_epoch_in_query_ns=0)
            bridge = cpu.get('clock_bridge') if isinstance(cpu, dict) else None
            if (type(authority.get('schema')) is not int or authority['schema'] != source['schema']
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
            if query_cpu_ownership:
                summary['query_cpu_ownership'] = _query_cpu_summary(authority)
            if observed_task_dag:
                reference = receipt['task_bindings']['pool']
                pool_authority = pool_indexed.get(reference['capture_id']) if reference else None
                if pool_authority is None:
                    raise ValueError('observed task DAG requires original matching pool authority')
                summary['observed_task_dag'] = _observed_task_dag_summary(pool_authority)
            if execution_flow_dag:
                reference = receipt['task_bindings']['pool']
                pool_authority = pool_indexed.get(reference['capture_id']) if reference else None
                if (latch_readiness_dag and pool_authority is not None
                        and pool_authority.get('schema') == 6):
                    if pool_authority.get('execution_flow_dag') is not None:
                        raise ValueError('pool6 cannot supply an execution flow DAG v1 projection')
                else:
                    if pool_authority is None or pool_authority.get('schema') != 5:
                        raise ValueError('execution flow DAG requires original matching pool5 authority')
                    summary['execution_flow_dag'] = _execution_flow_summary(pool_authority)
            if latch_readiness_dag:
                reference = receipt['task_bindings']['pool']
                pool_authority = pool_indexed.get(reference['capture_id']) if reference else None
                if pool_authority is not None:
                    if pool_authority.get('schema') == 6:
                        summary['latch_readiness_dag'] = _latch_readiness_summary(pool_authority)
                    elif pool_authority.get('latch_readiness_dag') is not None:
                        raise ValueError('latch readiness DAG requires original pool6 authority')
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
    parser.add_argument('--query-cpu-ownership', action='store_true',
                        help='Opt in to validated exclusive query-execution CPU ownership, not a critical path')
    parser.add_argument('--observed-task-dag', action='store_true',
                        help='Retain explicit native fork/join task DAG and scoped known CPU chains, not full span')
    parser.add_argument('--execution-flow-dag', action='store_true',
                        help='Retain observed query/join flow and known CPU chains, not full span')
    parser.add_argument('--latch-readiness-dag', action='store_true',
                        help='Retain observed same-latch readiness constraints, not publication latency or full span')
    parser.add_argument('--graph-output', type=Path,
                        help='Required with any diagnostic opt-in: exclusive full native graph JSON artifact outside the capture directory')
    parser.add_argument('--output', type=Path, required=True)
    args = parser.parse_args()
    try:
        provenance = (args.cache_call_provenance or args.dispatch_result_provenance
                      or args.query_cpu_ownership or args.observed_task_dag or args.execution_flow_dag
                      or args.latch_readiness_dag)
        if provenance or args.graph_output is not None:
            if not provenance or args.graph_output is None:
                if args.latch_readiness_dag:
                    raise ValueError('--latch-readiness-dag requires companion --graph-output and vice versa')
                raise ValueError('--cache-call-provenance or --dispatch-result-provenance '
                                 'or --query-cpu-ownership or --observed-task-dag or --execution-flow-dag requires companion --graph-output and vice versa')
            if args.output.resolve() == args.graph_output.resolve():
                raise ValueError('report and graph output paths must differ')
            if os.path.lexists(args.output):
                raise ValueError('report output already exists')
            report = analyze(args.capture, args.justrust, args.large_capture,
                             cache_call_provenance=args.cache_call_provenance,
                             graph_output=args.graph_output,
                             dispatch_result_provenance=args.dispatch_result_provenance,
                             query_cpu_ownership=args.query_cpu_ownership,
                             observed_task_dag=args.observed_task_dag,
                             execution_flow_dag=args.execution_flow_dag,
                             latch_readiness_dag=args.latch_readiness_dag)
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
