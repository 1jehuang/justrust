"""Analyzer report tests, separate from actual original-byte acceptance."""
import copy
import hashlib
import json
from pathlib import Path
import subprocess
import tempfile
import unittest
from unittest.mock import patch

import frontend_mapping as mapping
from verify_observed_task_dag_public import check_dag
from test_query_cpu_mapping import fixture as cpu_fixture


def authority():
    return dict(observed_task_dag=dict(
        scope='observed-fork-join-task-dag', topology_complete=True,
        timing_records_complete=True, records_complete=True, query_authority_valid=True,
        attached_cpu_ns=12, known_attached_cpu_ns=12, excluded_task_null_cpu_ns=3,
        excluded_query_cpu_ns=2, excluded_opaque_cpu_ns=1,
        excluded_task_null_segments=2, unmeasured_segments=0,
        nodes=[dict(task=1, cpu_ns=5, known_cpu_ns=5, measured_segments=1, unavailable_segments=0),
               dict(task=2, cpu_ns=7, known_cpu_ns=7, measured_segments=1, unavailable_segments=0)],
        edges=[dict(**{'from': 1, 'to': 2}, kind='fork')],
        components=[dict(root=1, tasks=2, edges=1, records_complete=True,
                         cpu_ns=12, known_cpu_ns=12,
                         longest_recorded_fork_join_chain_cpu_ns=12,
                         known_cpu_chain_ns=12, chain=[1, 2])], issues=[]))


class TaskDagMappingTests(unittest.TestCase):
    def test_compact_summary_retains_native_chain_not_a_ratio(self):
        summary = mapping._observed_task_dag_summary(authority())
        self.assertEqual(summary['observed_tasks'], 2)
        self.assertEqual(summary['observed_edges'], 1)
        self.assertEqual(summary['attached_cpu_ns'], 12)
        self.assertEqual(summary['excluded_query_cpu_ns'], 2)
        self.assertEqual(summary['top_components'][0]['chain'], [1, 2])
        self.assertNotIn('nodes', summary)
        self.assertNotIn('parallelism', summary)
        self.assertNotIn('execution_span_ns', summary)

    def test_partial_unknown_chain_is_not_zero(self):
        value = authority()
        dag = value['observed_task_dag']
        dag.update(records_complete=False, timing_records_complete=False,
                   attached_cpu_ns=None, known_attached_cpu_ns=None,
                   excluded_task_null_cpu_ns=None, excluded_query_cpu_ns=None,
                   excluded_opaque_cpu_ns=None, issues=['timing unavailable'])
        for node in dag['nodes']:
            node.update(cpu_ns=None, known_cpu_ns=None, measured_segments=0)
        dag['components'][0].update(records_complete=False, cpu_ns=None, known_cpu_ns=None,
            longest_recorded_fork_join_chain_cpu_ns=None, known_cpu_chain_ns=None, chain=[])
        summary = mapping._observed_task_dag_summary(value)
        self.assertIsNone(summary['attached_cpu_ns'])
        self.assertIsNone(summary['top_components'][0]['known_cpu_chain_ns'])
        self.assertEqual(summary['top_components'][0]['chain'], [])

    def test_shape_and_endpoint_mutations_fail_closed(self):
        mutations = [lambda dag: dag.update(records_complete=1),
                     lambda dag: dag.update(attached_cpu_ns=True),
                     lambda dag: dag['nodes'].append(dict(dag['nodes'][0])),
                     lambda dag: dag['edges'][0].update(to=99),
                     lambda dag: dag['edges'][0].update(kind='worker_chronology'),
                     lambda dag: dag['components'][0].update(chain=[2, 1]),
                     lambda dag: dag['components'][0].update(tasks=3),
                     lambda dag: dag['components'][0].update(known_cpu_chain_ns=None)]
        for mutation in mutations:
            value = copy.deepcopy(authority())
            mutation(value['observed_task_dag'])
            with self.subTest(mutation=mutation), self.assertRaises(ValueError):
                mapping._observed_task_dag_summary(value)

    def test_root6_option_routes_public_flag_and_exact_graph_artifact(self):
        query, query_authority, pool, pool_authority = cpu_fixture()
        query['schema'] = query_authority['schema'] = 6
        pool_authority.update(authority())
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            capture = root / 'capture'
            capture.mkdir()
            (capture / 'q.json').write_text(json.dumps(query))
            (capture / 'p.json').write_text(json.dumps(pool))
            encoded = json.dumps([query_authority, pool_authority]).encode()
            result = subprocess.CompletedProcess([], 0, encoded, b'')
            with patch.object(mapping.subprocess, 'run', return_value=result) as validator:
                report = mapping.analyze(capture, 'validator', query_cpu_ownership=True,
                    observed_task_dag=True, graph_output=root / 'graph.json')
            command = validator.call_args.args[0]
            self.assertIn('--observed-task-dag', command)
            self.assertNotIn('--dispatch-result-provenance', command)
            self.assertEqual((root / 'graph.json').read_bytes(), encoded)
            self.assertEqual(report['graph_output']['sha256'], hashlib.sha256(encoded).hexdigest())
            self.assertEqual(report['captures'][0]['observed_task_dag']['attached_cpu_ns'], 12)
            self.assertIsNone(report['execution_span_ns'])

    def test_independent_positive_gate_refuses_all_unknown_or_wrong_chain(self):
        expected = dict(tasks=[1, 2], roots=[1], edges=[(1, 2)],
            edge_kinds={(1, 2): 'fork'}, weights={1: 5, 2: 7},
            excluded={'query_execution': 2, 'opaque': 1},
            components=[dict(component=1, tasks=[1, 2], observed_task_cpu_ns=12,
                             known_cpu_chain_ns=12)])
        positive = authority()['observed_task_dag']
        check_dag(positive, expected)
        for mutation in (lambda dag: dag.update(records_complete=False),
                         lambda dag: dag['components'][0].update(chain=[2, 1]),
                         lambda dag: dag['nodes'][0].update(cpu_ns=None),
                         lambda dag: dag['edges'][0].update(kind='join')):
            changed = copy.deepcopy(positive)
            mutation(changed)
            with self.subTest(mutation=mutation), self.assertRaises(AssertionError):
                check_dag(changed, expected)

    def test_task_flag_does_not_admit_pool4_without_query_cpu_flag(self):
        query, _, pool, _ = cpu_fixture()
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            capture = root / 'capture'
            capture.mkdir()
            (capture / 'q.json').write_text(json.dumps(query))
            (capture / 'p.json').write_text(json.dumps(pool))
            with patch.object(mapping.subprocess, 'run') as validator:
                with self.assertRaisesRegex(ValueError, 'pool4 requires explicit'):
                    mapping.analyze(capture, 'validator', observed_task_dag=True,
                                    graph_output=root / 'graph.json')
                validator.assert_not_called()
                self.assertFalse((root / 'graph.json').exists())


if __name__ == '__main__':
    unittest.main()
