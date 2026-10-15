#!/usr/bin/env python3
"""Check installed task DAG reports against original native fork/join records.

All original bytes remain untouched. Full CLI stdout is private, compact receipts
contain only hashes and scoped costs. No work/span ratio or speed claim is made.
"""
import argparse
import json
from pathlib import Path
import subprocess
import shutil
import sys

from observed_task_dag_oracle import native_task_oracle
from verify_query_cpu_public import digest, load_capture, oracle as query_oracle, private_write, require


def check_dag(report, expected):
    require(report['scope'] == 'observed-fork-join-task-dag', 'wrong DAG scope')
    require(report['topology_complete'] and report['records_complete']
            and report['timing_records_complete'] and not report['issues'],
            'positive DAG acceptance needs complete original authority')
    require({node['task']: node['cpu_ns'] for node in report['nodes']} == expected['weights'],
            'not every native task weight was preserved')
    require({node['task']: node['known_cpu_ns'] for node in report['nodes']} == expected['weights'],
            'known native task costs disagree')
    actual_edges = {(edge['from'], edge['to']) for edge in report['edges']}
    require(actual_edges == set(expected['edges']), 'not every genuine native edge was preserved')
    require({(edge['from'], edge['to']): edge['kind'] for edge in report['edges']}
            == expected['edge_kinds'], 'original fork/join edge categories changed')
    require(sum(expected['weights'].values()) > 0, 'positive actual acceptance needs measured attached CPU')
    require(report['attached_cpu_ns'] == report['known_attached_cpu_ns']
            == sum(expected['weights'].values()), 'attached original CPU partition mismatch')
    require(report['excluded_task_null_cpu_ns'] == sum(expected['excluded'].values()),
            'excluded original CPU mass mismatch')
    require(report['excluded_query_cpu_ns'] == expected['excluded'].get('query_execution', 0)
            and report['excluded_opaque_cpu_ns'] == expected['excluded'].get('opaque', 0),
            'excluded original CPU category mismatch')
    by_root = {row['root']: row for row in report['components']}
    require(set(by_root) == set(expected['roots']), 'component native roots mismatch')
    for component in expected['components']:
        row = by_root[component['component']]
        require(row['records_complete'] and row['tasks'] == len(component['tasks']),
                'component original task inventory mismatch')
        require(row['cpu_ns'] == row['known_cpu_ns'] == component['observed_task_cpu_ns'],
                'component original CPU partition mismatch')
        require(row['known_cpu_chain_ns'] == row['longest_recorded_fork_join_chain_cpu_ns']
                == component['known_cpu_chain_ns'], 'longest native-edge CPU chain mismatch')
        chain = row['chain']
        require(chain and len(set(chain)) == len(chain)
                and set(chain) <= set(component['tasks'])
                and all(pair in actual_edges for pair in zip(chain, chain[1:])),
                'public chain is not a witness in original native edges')
        require(sum(expected['weights'][task] for task in chain) == row['known_cpu_chain_ns'],
                'public chain witness does not sum to its reported cost')
    return dict(tasks=len(expected['tasks']), edges=len(expected['edges']),
                attached_cpu_ns=report['attached_cpu_ns'],
                excluded_query_cpu_ns=report['excluded_query_cpu_ns'],
                excluded_opaque_cpu_ns=report['excluded_opaque_cpu_ns'],
                components=report['components'], every_original_task_edge_weight_checked=True)


def main():
    parser = argparse.ArgumentParser(__doc__)
    parser.add_argument('--justrust', type=Path, required=True)
    parser.add_argument('--accepted', type=Path, required=True)
    parser.add_argument('--evidence', type=Path, required=True)
    args = parser.parse_args()
    accepted, evidence = args.accepted.resolve(), args.evidence.resolve()
    require(not evidence.is_relative_to(accepted), 'evidence must be outside original capture')
    require((accepted / 'query-cpu-acceptance.json').is_file(), 'native acceptance marker required')
    evidence.mkdir(mode=0o700, parents=True, exist_ok=False)
    consumer = args.justrust.resolve()
    before_binary = digest(consumer)
    originals, cases = {}, []
    labels = ('query-cpu-cold-1', 'query-cpu-warm-1', 'query-cpu-with-dispatch')
    for label in labels:
        directory = accepted / (label + '-capture')
        (query_path, query), (pool_path, pool) = load_capture(directory)
        for path in (query_path, pool_path):
            originals[str(path)] = digest(path)
        query_oracle(query, pool)
        expected = native_task_oracle(pool)
        command = [str(consumer), 'graph', str(directory), '--large-capture',
                   '--query-cpu-ownership', '--observed-task-dag', '--json']
        result = subprocess.run(command, capture_output=True, timeout=300)
        private_write(evidence / (label + '-graph.json'), result.stdout)
        private_write(evidence / (label + '-stderr.txt'), result.stderr)
        require(result.returncode == 0, 'actual task DAG CLI failed: ' + result.stderr.decode(errors='replace'))
        assessments = json.loads(result.stdout)
        for assessment in assessments:
            require(not assessment['full_pipeline_complete'] and assessment['cpu_work_ns'] is None
                    and assessment['execution_span_ns'] is None, 'DAG promoted full CPU/span')
        pool_report = next(row for row in assessments if row['producer'] == 'rustc-thread-pool-join')
        checks = check_dag(pool_report['observed_task_dag'], expected)
        analyzer_output = evidence / (label + '-summary.json')
        graph_output = evidence / (label + '-analyzer-graph.json')
        analyze_command = [sys.executable, str(Path(__file__).with_name('frontend_mapping.py')),
            '--capture', str(directory), '--justrust', str(consumer), '--large-capture',
            '--query-cpu-ownership', '--observed-task-dag', '--graph-output', str(graph_output),
            '--output', str(analyzer_output)]
        analysis = subprocess.run(analyze_command, capture_output=True, timeout=300)
        private_write(evidence / (label + '-analyzer-stderr.txt'), analysis.stderr)
        require(analysis.returncode == 0, 'actual task DAG analyzer failed: ' + analysis.stderr.decode(errors='replace'))
        summary = json.loads(analyzer_output.read_bytes())
        compact = summary['captures'][0]['observed_task_dag']
        require(compact['records_complete'] and compact['observed_tasks'] == len(expected['tasks'])
                and compact['observed_edges'] == len(expected['edges']), 'actual compact inventory mismatch')
        require(compact['attached_cpu_ns'] == checks['attached_cpu_ns']
                and compact['top_components'] == sorted(checks['components'],
                    key=lambda row: (-row['known_cpu_chain_ns'], row['root']))[:20],
                'actual compact costs/chains mismatch')
        require(graph_output.stat().st_mode & 0o777 == 0o600
                and summary['graph_output']['sha256'] == digest(graph_output)
                and summary['graph_output']['bytes'] == graph_output.stat().st_size,
                'actual companion graph hash/mode mismatch')
        original_graph_hash = digest(graph_output)
        collision = subprocess.run(analyze_command, capture_output=True, timeout=300)
        require(collision.returncode != 0 and digest(graph_output) == original_graph_hash,
                'existing analyzer outputs were overwritten')
        cases.append(dict(case=label, checks=checks, graph_sha256=digest(evidence / (label + '-graph.json')),
                          analyzer_sha256=digest(analyzer_output), collision_preserved=True))
        if label == 'query-cpu-warm-1':
            missing = evidence / 'missing-query-copy'
            missing.mkdir(mode=0o700)
            shutil.copyfile(pool_path, missing / 'pool.json')
            negative = subprocess.run([str(consumer), 'graph', str(missing), '--json',
                '--query-cpu-ownership', '--observed-task-dag'], capture_output=True, timeout=300)
            private_write(evidence / 'missing-query-graph.json', negative.stdout)
            private_write(evidence / 'missing-query-stderr.txt', negative.stderr)
            require(negative.returncode == 0, 'missing query should preserve local task topology')
            partial = json.loads(negative.stdout)[0]['observed_task_dag']
            require(partial['topology_complete'] and not partial['records_complete']
                    and not partial['query_authority_valid'] and partial['attached_cpu_ns'] is None,
                    'missing query authority incorrectly certified complete task weights')
            local_cpu = sum(row['cpu_ns'] for row in pool['timing']['segments']
                            if row['owner_kind'] == 'task')
            require(partial['known_attached_cpu_ns'] == local_cpu,
                    'missing query erased local task CPU or authorized query labels')
            require({(edge['from'], edge['to']) for edge in partial['edges']}
                    == set(expected['edges']), 'missing query erased genuine local topology')
            cases.append(dict(case='missing-query-copy', topology_preserved=True,
                query_weights_unknown=True, known_attached_cpu_ns=local_cpu,
                graph_sha256=digest(evidence / 'missing-query-graph.json')))
    require(digest(consumer) == before_binary, 'installed client changed during replay')
    require(all(digest(Path(path)) == value for path, value in originals.items()),
            'original native capture changed during replay')
    receipt = dict(schema=1, scope='original-native-observed-task-dag-acceptance-not-a-benchmark',
                   consumer_sha256=before_binary, cases=cases,
                   original_files_unchanged=len(originals), full_pipeline_complete=False,
                   cpu_work_ns=None, execution_span_ns=None, theoretical_parallelism=None)
    private_write(evidence / 'summary.json', (json.dumps(receipt, indent=2) + '\n').encode())
    print(json.dumps(receipt, indent=2))


if __name__ == '__main__':
    main()
