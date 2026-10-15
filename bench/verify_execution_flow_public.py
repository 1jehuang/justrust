#!/usr/bin/env python3
"""Replay completed native execution-flow cohorts without changing original bytes.

This is independent public acceptance, not a benchmark or a full critical path.
Full stdout, copied controls and before/after inventories remain private.
"""
import argparse
from collections import Counter
import copy
import hashlib
import json
from pathlib import Path
import subprocess
import sys

from execution_flow_oracle import execution_flow_oracle, _known_chain
from verify_query_cpu_public import digest, load_capture, private_write, require

COUNTS = ('nodes', 'edges', 'scopes', 'active_scopes', 'lost_nodes', 'lost_edges',
          'lost_scopes', 'assigned_cpu_segments', 'unassigned_cpu_segments',
          'missing_measurement_references', 'unavailable_cpu_segments', 'cut_nodes')
COSTS = ('known_cpu_work_ns', 'known_query_cpu_ns', 'known_task_cpu_ns',
         'known_opaque_cpu_ns', 'unassigned_known_cpu_ns', 'longest_known_chain_cpu_ns')
LABELS = ('flow-cold-1', 'flow-warm-1', 'flow-with-dispatch')


def scoped(report):
    require(report['full_pipeline_complete'] is False and report['flow_complete'] is False,
            'flow promoted completeness beyond observed instrumentation')
    require(all(report[key] is None for key in ('cpu_work_ns', 'execution_span_ns', 'parallelism')),
            'flow promoted full work, span or parallelism')
    require(report['scope'] == 'observed-query-and-join-control-flow', 'incorrect flow scope')


def witness(report, pool, query_authorized=True):
    """Check every public witness row against original nodes, edges and CPU rows."""
    nodes = {row['id']: row for row in pool['execution_flow']['nodes']}
    scopes = {row['id']: row for row in pool['execution_flow']['scopes']}
    segments = {row['segment_id']: row for row in pool['timing']['segments']}
    edges = {(row['from'], row['to']) for row in pool['execution_flow']['edges']}
    path, steps = report['longest_known_chain_path'], report['longest_known_chain_steps']
    cost = report['longest_known_chain_cpu_ns']
    require(len(path) == len(steps) and len(set(path)) == len(path), 'bad public witness inventory')
    require(all(pair in edges for pair in zip(path, path[1:])), 'witness invented an original edge')
    measured = 0
    total = 0
    for ident, step in zip(path, steps):
        require(ident in nodes, 'witness invented an original node')
        node = nodes[ident]
        require(node['kind'] != 'cut', 'known witness traversed an unknown ancestry cut')
        scope = scopes.get(node['scope_id']) if query_authorized else None
        segment = segments.get(node['segment_id'])
        if segment:
            require(query_authorized or segment['owner_kind'] != 'query_execution',
                    'unqualified witness weighted query CPU')
            measured += 1
        expected = dict(node_id=ident, kind=node['kind'], at_ns=node['at_ns'],
                        segment_id=node['segment_id'], scope_id=node['scope_id'], task=node['task'],
                        query_context=scope['query_context'] if scope else None,
                        query_job=scope['query_job'] if scope else None,
                        query_name=segment['query_name'] if segment and query_authorized else None,
                        cpu_ns=segment['cpu_ns'] if segment else 0)
        require(step == expected, 'public witness differs from exact original scope/node/CPU fields')
        total += expected['cpu_ns']
    require((cost is None and not path) or (cost == total and measured > 0),
            'public witness CPU sum or genuine measurement mismatch')


def check_flow(report, expected, pool):
    scoped(report)
    for key, value in expected.items():
        if key not in ('longest_known_chain_path', 'longest_known_chain_steps'):
            require(report[key] == value, 'original flow oracle mismatch: ' + key)
    require(report['records_complete'] is True and report['issues'] == [],
            'healthy actual flow must have no issues')
    require(isinstance(report['limitations'], list) and report['limitations'],
            'instrumentation limitations must remain separate from issues')
    witness(report, pool)
    # A tied maximum may choose another genuine original witness.
    return dict(**{key: report[key] for key in (*COUNTS, *COSTS)},
                every_original_count_cost_and_witness_checked=True)


def check_unqualified(report, query, pool):
    """Withdraw only query authority, retaining independently measured local CPU."""
    expected = execution_flow_oracle(query, pool)
    nodes = {row['id']: row for row in pool['execution_flow']['nodes']}
    edges = {(row['from'], row['to']) for row in pool['execution_flow']['edges']}
    segments = {row['segment_id']: row for row in pool['timing']['segments']}
    costs = {}
    totals, counts = Counter(), Counter()
    unavailable = 0
    for ident, node in nodes.items():
        segment = segments.get(node['segment_id'])
        if segment is not None and segment['owner_kind'] == 'query_execution':
            costs[ident] = None
            unavailable += 1
        elif segment is not None:
            costs[ident] = segment['cpu_ns']
            totals[segment['owner_kind']] += segment['cpu_ns']
            counts[segment['owner_kind']] += 1
        else:
            costs[ident] = None if node['kind'] == 'cut' else 0
    maximum, _ = _known_chain(nodes, edges, costs)
    expected.update(records_complete=False, unavailable_cpu_segments=unavailable,
                    known_cpu_work_ns=sum(totals.values()) if sum(counts.values()) else None,
                    known_query_cpu_ns=None,
                    known_task_cpu_ns=totals['task'] if counts['task'] else None,
                    known_opaque_cpu_ns=totals['opaque'] if counts['opaque'] else None,
                    longest_known_chain_cpu_ns=maximum)
    for key in (*COUNTS, *COSTS):
        require(report[key] == expected[key], 'unqualified original local count/cost changed: ' + key)
    witness(report, pool, query_authorized=False)


def check_compact(compact, public):
    for key, value in public.items():
        if key not in ('longest_known_chain_path', 'longest_known_chain_steps'):
            require(compact[key] == value, 'compact public fact changed: ' + key)
    steps, path = public['longest_known_chain_steps'], public['longest_known_chain_path']
    named = sorted((row for row in steps if row['query_name'] is not None),
                   key=lambda row: (-row['cpu_ns'], row['node_id']))
    kinds = Counter()
    for row in named:
        kinds[row['query_name']] += row['cpu_ns']
    ranked_kinds = [dict(query_name=name, cpu_ns=cpu)
                    for name, cpu in sorted(kinds.items(), key=lambda row: (-row[1], row[0]))]
    expected = dict(longest_known_chain_nodes=len(path),
                    longest_known_chain_cpu_segments=sum(row['kind'] == 'cpu_segment' for row in steps),
                    named_chain_cpu_segments=len(named),
                    named_chain_cpu_ns=sum(kinds.values()) if public['longest_known_chain_cpu_ns'] is not None else None,
                    top_named_chain_cpu_segments=named[:20],
                    omitted_named_chain_cpu_segments=max(0, len(named) - 20),
                    chain_query_kinds=len(kinds), top_chain_query_kinds=ranked_kinds[:20],
                    omitted_chain_query_kinds=max(0, len(kinds) - 20))
    require(all(compact[key] == value for key, value in expected.items()),
            'compact named-chain rankings/counts/costs changed')
    require('longest_known_chain_path' not in compact and 'longest_known_chain_steps' not in compact,
            'compact output duplicated the full private witness')


def main():
    parser = argparse.ArgumentParser(__doc__)
    parser.add_argument('--accepted', type=Path, required=True)
    parser.add_argument('--justrust', type=Path, required=True)
    parser.add_argument('--evidence', type=Path, required=True)
    args = parser.parse_args()
    accepted, evidence, consumer = args.accepted.resolve(), args.evidence.resolve(), args.justrust.resolve()
    require(not evidence.is_relative_to(accepted), 'evidence must be outside original accepted cohort')
    require((accepted / 'execution-flow-acceptance.json').is_file(),
            'wait for completed native execution-flow-acceptance.json')
    evidence.mkdir(mode=0o700, parents=True, exist_ok=False)
    require(evidence.stat().st_mode & 0o777 == 0o700, 'evidence directory is not private')

    def inventory():
        return {str(path.relative_to(accepted)): dict(bytes=path.stat().st_size, sha256=digest(path))
                for path in sorted(accepted.rglob('*')) if path.is_file()}

    before = inventory()
    binary_before = dict(bytes=consumer.stat().st_size, sha256=digest(consumer))
    private_write(evidence / 'inputs-before.json', json.dumps(before, sort_keys=True).encode())
    private_write(evidence / 'binary-before.json', json.dumps(binary_before, sort_keys=True).encode())
    cases = []

    def execute(label, command, refusal=False, parse_json=True):
        result = subprocess.run(command, capture_output=True, timeout=300)
        private_write(evidence / (label + '.stdout'), result.stdout)
        private_write(evidence / (label + '.stderr'), result.stderr)
        record = dict(case=label, exit=result.returncode,
                      stdout_sha256=hashlib.sha256(result.stdout).hexdigest(), stdout_bytes=len(result.stdout))
        cases.append(record)
        if refusal:
            require(result.returncode != 0, label + ': required public refusal succeeded')
            record['public_refusal_checked'] = True
            return None, record
        require(result.returncode == 0, label + ': public replay failed: ' + result.stderr.decode(errors='replace'))
        return (json.loads(result.stdout) if parse_json else None), record

    def graph(label, directory, flags=(), refusal=False):
        return execute(label, [str(consumer), 'graph', str(directory), '--json', '--large-capture', *flags], refusal)

    def pool_flow(values):
        rows = [row for row in values if row['producer'] == 'rustc-thread-pool-join']
        require(len(rows) == 1, 'public replay requires one original pool assessment')
        return rows[0]['execution_flow_dag']

    def no_full(values):
        for row in values:
            require(row['full_pipeline_complete'] is False and row['cpu_work_ns'] is None
                    and row['execution_span_ns'] is None, 'public replay promoted full work/span')

    try:
        warm = None
        for label in LABELS:
            directory = accepted / (label + '-capture')
            (query_path, query), (pool_path, pool) = load_capture(directory)
            require(query['schema'] == (6 if label == 'flow-with-dispatch' else 5),
                    'actual cohort query schema mismatch')
            expected = execution_flow_oracle(query_path, pool_path)
            values, record = graph(label + '-flow-only', directory, ['--execution-flow-dag'])
            no_full(values)
            public = pool_flow(values)
            record['checks'] = check_flow(public, expected, pool)
            for row in values:
                require(all(row.get(key) is None for key in
                            ('query_cpu_ownership', 'cache_call_provenance', 'dispatch_result_provenance', 'observed_task_dag')),
                        'flow-only implied unrelated public diagnostics')
            flags = ['--execution-flow-dag', '--query-cpu-ownership']
            if query['schema'] == 6:
                flags.append('--dispatch-result-provenance')
            combined, record = graph(label + '-combined', directory, flags)
            no_full(combined)
            require(pool_flow(combined) == public, 'CPU/dispatch opt-ins changed independent flow assessment')
            record['independent_flag_equality_checked'] = True
            graph(label + '-default-pool5-refusal', pool_path, refusal=True)
            graph(label + '-old-cpu-pool5-refusal', pool_path, ['--query-cpu-ownership'], refusal=True)
            output, artifact = evidence / (label + '-compact.json'), evidence / (label + '-artifact.json')
            command = [sys.executable, str(Path(__file__).with_name('frontend_mapping.py')),
                       '--capture', str(directory), '--justrust', str(consumer), '--large-capture',
                       '--execution-flow-dag', '--graph-output', str(artifact), '--output', str(output)]
            _, analyzer_record = execute(label + '-analyzer', command, parse_json=False)
            summary = json.loads(output.read_bytes())
            require(len(summary['captures']) == 1, 'actual analyzer capture inventory mismatch')
            compact = summary['captures'][0]
            require(all(key not in compact for key in ('query_cpu_ownership', 'cache_call_provenance',
                        'dispatch_result_provenance', 'observed_task_dag')), 'compact flow implied unrelated diagnostics')
            artifact_values = json.loads(artifact.read_bytes())
            artifact_flow = pool_flow(artifact_values)
            check_flow(artifact_flow, expected, pool)
            check_compact(compact['execution_flow_dag'], artifact_flow)
            require(artifact_flow == public, 'analyzer companion changed original flow assessment')
            require(artifact.stat().st_mode & 0o777 == output.stat().st_mode & 0o777 == 0o600,
                    'actual analyzer artifacts are not mode0600')
            require(summary['graph_output']['sha256'] == digest(artifact)
                    and summary['graph_output']['bytes'] == artifact.stat().st_size,
                    'exact companion stdout hash/size mismatch')
            artifact_hash, output_hash = digest(artifact), digest(output)
            execute(label + '-output-collision', command, refusal=True)
            require(digest(artifact) == artifact_hash and digest(output) == output_hash,
                    'collision overwrote existing report or full graph')
            collision_command = command[:-1] + [str(evidence / (label + '-collision-unused.json'))]
            execute(label + '-artifact-collision', collision_command, refusal=True)
            require(digest(artifact) == artifact_hash and not Path(collision_command[-1]).exists(),
                    'artifact collision overwrote stdout or created report')
            analyzer_record.update(compact_rankings_artifact_hash_mode_and_collisions_checked=True,
                                   artifact_sha256=artifact_hash, artifact_bytes=artifact.stat().st_size,
                                   compact_sha256=output_hash)
            if label == 'flow-warm-1':
                warm = query_path, query, pool_path, pool

        query_path, query, pool_path, pool = warm
        for variant in ('missing-query', 'clock-mismatch', 'duplicate-pool', 'cross-producer-collision'):
            directory = evidence / (variant + '-copy')
            directory.mkdir(mode=0o700)
            private_write(directory / 'pool.json', pool_path.read_bytes())
            if variant != 'missing-query':
                changed = copy.deepcopy(query)
                if variant == 'clock-mismatch':
                    changed['execution_clock']['pool_observer'] += 1
                private_write(directory / 'query.json', json.dumps(changed).encode())
            if variant == 'duplicate-pool':
                private_write(directory / 'pool-copy.json', pool_path.read_bytes())
            if variant == 'cross-producer-collision':
                scheduler = dict(schema=1, producer='cargo-scheduler', scope='cargo-unit-scheduler',
                                 capture_id=pool['capture_id'], declared_nodes=0, declared_edges=0,
                                 structural_complete=False, full_pipeline_complete=False,
                                 issues=[], opaque_boundaries=['native'], events=[], nodes=[], edges=[])
                private_write(directory / 'scheduler.json', json.dumps(scheduler).encode())
            values, record = graph(variant, directory, ['--execution-flow-dag'])
            no_full(values)
            pool_rows = [row for row in values if row['producer'] == 'rustc-thread-pool-join']
            require(len(pool_rows) == (2 if variant == 'duplicate-pool' else 1),
                    'negative replay omitted an original pool or a duplicate copy')
            for row in pool_rows:
                partial = row['execution_flow_dag']
                scoped(partial)
                require(partial['records_complete'] is False and partial['issues'],
                        'unqualified negative flow acquired complete authority')
                require(partial['known_query_cpu_ns'] is None
                        and all(step['query_name'] is None for step in partial['longest_known_chain_steps']),
                        'unqualified negative flow acquired named query CPU')
                if variant in ('duplicate-pool', 'cross-producer-collision'):
                    require(all(partial[key] == 0 for key in COUNTS)
                            and all(partial[key] is None for key in COSTS)
                            and partial['longest_known_chain_path'] == partial['longest_known_chain_steps'] == [],
                            'colliding source retained unavailable flow authority')
                else:
                    check_unqualified(partial, query, pool)
            record['unknown_query_cpu_no_names_or_false_completeness_checked'] = True
    finally:
        after = inventory()
        binary_after = dict(bytes=consumer.stat().st_size, sha256=digest(consumer))
        private_write(evidence / 'inputs-after.json', json.dumps(after, sort_keys=True).encode())
        private_write(evidence / 'binary-after.json', json.dumps(binary_after, sort_keys=True).encode())
        require(before == after, 'accepted original inputs changed during replay')
        require(binary_before == binary_after, 'consumer binary changed during replay')

    receipt = dict(schema=1, scope='original-native-execution-flow-public-acceptance-not-a-benchmark',
                   consumer_sha256=binary_before['sha256'], cases=cases, all_checks_passed=True,
                   original_files_unchanged=len(before), original_inventory_sha256=hashlib.sha256(
                       json.dumps(before, sort_keys=True).encode()).hexdigest(),
                   full_pipeline_complete=False, cpu_work_ns=None, execution_span_ns=None,
                   theoretical_parallelism=None,
                   limitations=['Replays completed real compiler captures, not a compiler rerun or benchmark.',
                                'Known retained CPU chains are not full span, work/span ratios or thread ceilings.',
                                'Full query identities and witnesses remain only in private evidence.'])
    private_write(evidence / 'summary.json', (json.dumps(receipt, indent=2) + '\n').encode())
    print(json.dumps(receipt, indent=2))


if __name__ == '__main__':
    main()
