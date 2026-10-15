#!/usr/bin/env python3
"""Compare scoped mapping coverage within identical original readiness captures.

This is source-to-public identity and CPU-mass accounting, not a benchmark.
Task-null measurements excluded by the explicit task DAG may still be assigned
by the separately validated query/control-flow graph. Neither is full work/span.
"""
import argparse
from collections import Counter
import hashlib
import json
from pathlib import Path
import subprocess

from latch_readiness_oracle import latch_readiness_oracle
from observed_task_dag_oracle import native_task_oracle
from verify_latch_readiness_public import check_flow
from verify_observed_task_dag_public import check_dag
from verify_query_cpu_public import digest, load_capture, private_write, require

LABELS = ('readiness-cold-1', 'readiness-warm-1', 'readiness-with-dispatch')
FLAGS = ('--json', '--large-capture', '--latch-readiness-dag',
         '--query-cpu-ownership', '--observed-task-dag')


def coverage(query, pool, public):
    """Reconcile exact original task identities, query owners and flow witnesses."""
    expected_flow = latch_readiness_oracle(query, pool, allow_unbound=True)
    expected_task = native_task_oracle(pool)
    pools = [r for r in public if r['producer'] == 'rustc-thread-pool-join']
    queries = [r for r in public if r['producer'] == 'rustc-query-admission']
    require(len(pools) == len(queries) == 1, 'exact original public producer inventories required')
    p, q = pools[0], queries[0]
    for row in public:
        require(row['full_pipeline_complete'] is False and row['cpu_work_ns'] is None
                and row['execution_span_ns'] is None, 'coverage became full work/span')
    flow, task = p['latch_readiness_dag'], p['observed_task_dag']
    check_flow(flow, expected_flow, pool)
    check_dag(task, expected_task)
    require(task['query_authority_valid'] is True, 'task DAG lacks actual query admission authority')
    require(len(task['nodes']) == len(expected_task['tasks'])
            and len(task['edges']) == len(expected_task['edges']), 'duplicated public task identities/edges')
    rows = pool['timing']['segments']
    attached, excluded, by_query, names = Counter(), Counter(), Counter(), {}
    for row in rows:
        bucket = attached if row['task'] is not None else excluded
        bucket['segments'] += 1
        bucket['cpu_ns'] += row['cpu_ns']
        if row['owner_kind'] == 'query_execution':
            owner = row['query_context'], row['query_job']
            require(names.setdefault(owner, row['query_name']) == row['query_name'], 'query kind changed')
            by_query[owner] += row['cpu_ns']
    original_total = sum(row['cpu_ns'] for row in rows)
    require(attached['cpu_ns'] == task['attached_cpu_ns']
            and excluded['cpu_ns'] == task['excluded_task_null_cpu_ns'], 'public task partition changed')
    require(task['attached_cpu_ns'] + task['excluded_task_null_cpu_ns'] == original_total
            == flow['known_cpu_work_ns'], 'same-original task/flow CPU mass does not reconcile')
    require(flow['assigned_cpu_segments'] == len(rows) and flow['unassigned_cpu_segments'] == 0,
            'positive coverage must assign every original CPU segment exactly once')
    expected_queries = [dict(query_context=c, query_job=j, query_name=names[c, j], cpu_ns=cost)
                        for (c, j), cost in by_query.items()]
    expected_queries.sort(key=lambda r: (-r['cpu_ns'], r['query_context'], r['query_job']))
    cpu = q['query_cpu_ownership']
    require(cpu['scope'] == 'observed-query-execution-cpu' and cpu['records_complete']
            and not cpu['issues'] and cpu['queries'] == expected_queries,
            'exact original named query CPU identities/costs changed')
    require(cpu['query_execution_cpu_ns'] == sum(by_query.values()) == flow['known_query_cpu_ns'],
            'query ownership and flow CPU disagree on same original measurements')
    require(cpu['measured_segments'] == sum(r['owner_kind'] == 'query_execution' for r in rows),
            'public query segment inventory changed')
    task_null_query = [r for r in rows if r['task'] is None and r['owner_kind'] == 'query_execution']
    task_null_opaque = [r for r in rows if r['task'] is None and r['owner_kind'] == 'opaque']
    return dict(label=None, original_segments=len(rows), task_attached_segments=attached['segments'],
                task_attached_cpu_ns=attached['cpu_ns'], excluded_task_null_segments=excluded['segments'],
                excluded_task_null_cpu_ns=excluded['cpu_ns'],
                excluded_task_null_query_segments=len(task_null_query),
                excluded_task_null_query_cpu_ns=sum(r['cpu_ns'] for r in task_null_query),
                excluded_task_null_opaque_segments=len(task_null_opaque),
                excluded_task_null_opaque_cpu_ns=sum(r['cpu_ns'] for r in task_null_opaque),
                flow_assigned_segments=flow['assigned_cpu_segments'], flow_known_cpu_ns=flow['known_cpu_work_ns'],
                flow_unassigned_segments=flow['unassigned_cpu_segments'],
                named_query_owners=len(by_query), named_query_cpu_ns=cpu['query_execution_cpu_ns'],
                original_tasks=len(expected_task['tasks']), original_task_edges=len(expected_task['edges']),
                flow_nodes=flow['nodes'], flow_edges=flow['edges'], flow_scopes=flow['scopes'],
                readiness_rows=flow['readiness_rows'], wait_attempts=flow['wait_attempts'],
                matched_readiness_transfers=flow['matched_readiness_transfers'],
                unbound_wait_attempts=flow['unbound_wait_attempts'],
                readiness_records_complete=flow['records_complete'],
                original_task_edge_weight_query_identity_and_flow_witness_checked=True)


def main():
    parser = argparse.ArgumentParser(__doc__)
    parser.add_argument('--accepted', required=True, type=Path)
    parser.add_argument('--justrust', required=True, type=Path)
    parser.add_argument('--evidence', required=True, type=Path)
    args = parser.parse_args()
    accepted, consumer, evidence = args.accepted.resolve(), args.justrust.resolve(), args.evidence.resolve()
    require(not evidence.is_relative_to(accepted), 'evidence must be outside accepted originals')
    marker_path = accepted / 'latch-readiness-acceptance.json'
    marker = json.loads(marker_path.read_bytes())
    require([r['label'] for r in marker['observations']] == list(LABELS), 'completed native marker required')
    evidence.mkdir(mode=0o700, parents=True, exist_ok=False)
    require(evidence.stat().st_mode & 0o777 == 0o700, 'private evidence directory required')
    binary_before = digest(consumer)
    originals = {str(marker_path): digest(marker_path)}
    cases = []
    try:
        for label in LABELS:
            directory = accepted / (label + '-capture')
            (query_path, query), (pool_path, pool) = load_capture(directory)
            require(marker['original_sha256'][label] == {p.name: digest(p) for p in directory.glob('*.json')},
                    'capture hashes differ from completed native marker')
            originals.update({str(p): digest(p) for p in (query_path, pool_path)})
            command = [str(consumer), 'graph', str(directory), *FLAGS]
            result = subprocess.run(command, capture_output=True, timeout=300)
            private_write(evidence / (label + '-public.json'), result.stdout)
            private_write(evidence / (label + '-stderr.txt'), result.stderr)
            require(result.returncode == 0, 'combined public mapping failed: ' + result.stderr.decode(errors='replace'))
            public = json.loads(result.stdout)
            case = coverage(query, pool, public)
            case.update(label=label, public_sha256=hashlib.sha256(result.stdout).hexdigest(),
                        public_bytes=len(result.stdout), query_sha256=originals[str(query_path)],
                        pool_sha256=originals[str(pool_path)])
            cases.append(case)
    finally:
        require(all(digest(Path(p)) == value for p, value in originals.items()), 'original bytes changed')
        require(digest(consumer) == binary_before, 'consumer changed')
        private_write(evidence / 'originals.json', json.dumps(originals, sort_keys=True).encode())
    receipt = dict(schema=1, scope='same-original-scoped-mapping-coverage-not-a-benchmark',
                   consumer_sha256=binary_before, cases=cases, all_checks_passed=True,
                   originals_and_consumer_unchanged=True, public_flags=list(FLAGS),
                   full_pipeline_complete=False, cpu_work_ns=None, execution_span_ns=None,
                   parallelism=None,
                   limitations=['Same original measurement bytes, not cross-cohort performance.',
                                'Task-null CPU is deliberately excluded by task DAG, not lost measurement.',
                                'Flow assignment adds validated coverage, not full span or a speed claim.',
                                'Original identities, full query rows and witnesses remain private.'])
    private_write(evidence / 'summary.json', (json.dumps(receipt, indent=2) + '\n').encode())
    print(json.dumps(receipt, indent=2))


if __name__ == '__main__':
    main()
