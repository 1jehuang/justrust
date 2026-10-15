"""Offline public-consumer checks, not native execution-flow acceptance."""
import hashlib
import json
from pathlib import Path
import subprocess
import sys
import tempfile
import unittest
from unittest.mock import patch

import frontend_mapping as mapping
from test_frontend_mapping import timing_fixture


def fixture():
    query, authority, pool, assessed = timing_fixture()
    pool['schema'] = assessed['schema'] = 5
    flow = dict(scope='observed-query-and-join-control-flow',
                pool_capture_id=pool['capture_id'], pool_observer=pool['observer'],
                records_complete=True, full_pipeline_complete=False, flow_complete=False,
                nodes=4, edges=2, scopes=1, active_scopes=0,
                lost_nodes=0, lost_edges=0, lost_scopes=0,
                assigned_cpu_segments=2, unassigned_cpu_segments=0,
                unassigned_known_cpu_ns=0, missing_measurement_references=0,
                unavailable_cpu_segments=0, cut_nodes=0, known_cpu_work_ns=9,
                known_query_cpu_ns=6, known_task_cpu_ns=None, known_opaque_cpu_ns=3,
                longest_known_chain_cpu_ns=6, longest_known_chain_path=[1, 2],
                longest_known_chain_steps=[
                    dict(node_id=1, kind='scope_enter', at_ns=1, segment_id=None,
                         scope_id=1, task=1, query_context=1, query_job=1,
                         query_name=None, cpu_ns=0),
                    dict(node_id=2, kind='cpu_segment', at_ns=10, segment_id=1,
                         scope_id=1, task=1, query_context=1, query_job=1,
                         query_name='type_of', cpu_ns=6)],
                cpu_work_ns=None, execution_span_ns=None, parallelism=None,
                issues=[], limitations=['Wait/result readiness and instrumentation gaps are outside this graph.'])
    assessed['execution_flow_dag'] = flow
    return query, authority, pool, assessed


class ExecutionFlowMappingTests(unittest.TestCase):
    def test_summary_preserves_scoped_costs_and_separates_limitations(self):
        assessed = fixture()[3]
        summary = mapping._execution_flow_summary(assessed)
        self.assertEqual(summary['known_cpu_work_ns'], 9)
        self.assertIsNone(summary['known_task_cpu_ns'])
        self.assertEqual(summary['longest_known_chain_nodes'], 2)
        self.assertEqual(summary['top_named_chain_cpu_segments'],
                         [assessed['execution_flow_dag']['longest_known_chain_steps'][1]])
        self.assertEqual(summary['top_chain_query_kinds'], [dict(query_name='type_of', cpu_ns=6)])
        self.assertEqual(summary['issues'], [])
        self.assertTrue(summary['limitations'])
        self.assertNotIn('longest_known_chain_path', summary)
        self.assertNotIn('longest_known_chain_steps', summary)
        for key in ('cpu_work_ns', 'execution_span_ns', 'parallelism'):
            self.assertIsNone(summary[key])
        self.assertFalse(summary['flow_complete'])
        self.assertFalse(summary['full_pipeline_complete'])

    def test_top20_rankings_preserve_original_counts_and_costs(self):
        assessed = fixture()[3]
        flow = assessed['execution_flow_dag']
        flow['longest_known_chain_steps'] = [dict(
            node_id=n, kind='cpu_segment', at_ns=n, segment_id=n, scope_id=n,
            task=1, query_context=1, query_job=n, query_name=f'kind_{n}', cpu_ns=n)
            for n in range(1, 36)]
        flow.update(nodes=35, edges=34, scopes=35, assigned_cpu_segments=35,
                    known_cpu_work_ns=630, known_query_cpu_ns=630, known_opaque_cpu_ns=None,
                    longest_known_chain_cpu_ns=630, longest_known_chain_path=list(range(1, 36)))
        summary = mapping._execution_flow_summary(assessed)
        self.assertEqual(summary['longest_known_chain_cpu_ns'], 630)
        self.assertEqual(summary['named_chain_cpu_ns'], 630)
        self.assertEqual(summary['named_chain_cpu_segments'], 35)
        self.assertEqual(summary['chain_query_kinds'], 35)
        for key in ('top_named_chain_cpu_segments', 'top_chain_query_kinds'):
            self.assertEqual(len(summary[key]), 20)
            self.assertEqual(summary[key][0]['cpu_ns'], 35)
        self.assertEqual(summary['omitted_named_chain_cpu_segments'], 15)
        self.assertEqual(summary['omitted_chain_query_kinds'], 15)

    def test_unavailable_authority_stays_nullable(self):
        assessed = fixture()[3]
        flow = assessed['execution_flow_dag']
        for key in ('known_cpu_work_ns', 'known_query_cpu_ns', 'known_task_cpu_ns',
                    'known_opaque_cpu_ns', 'unassigned_known_cpu_ns', 'longest_known_chain_cpu_ns'):
            flow[key] = None
        flow.update(records_complete=False, longest_known_chain_path=[],
                    longest_known_chain_steps=[], issues=['measurement authority unavailable'])
        summary = mapping._execution_flow_summary(assessed)
        self.assertIsNone(summary['known_cpu_work_ns'])
        self.assertIsNone(summary['longest_known_chain_cpu_ns'])
        self.assertIsNone(summary['named_chain_cpu_ns'])
        self.assertEqual(summary['top_named_chain_cpu_segments'], [])
        self.assertEqual(summary['top_chain_query_kinds'], [])

    def test_partial_known_chain_and_measured_zero_remain_actionable(self):
        assessed = fixture()[3]
        flow = assessed['execution_flow_dag']
        flow.update(records_complete=False, lost_edges=1, issues=['lost flow edge'])
        summary = mapping._execution_flow_summary(assessed)
        self.assertEqual(summary['named_chain_cpu_ns'], 6)
        self.assertEqual(summary['issues'], ['lost flow edge'])
        flow.update(known_cpu_work_ns=3, known_query_cpu_ns=0, longest_known_chain_cpu_ns=0)
        flow['longest_known_chain_steps'][1]['cpu_ns'] = 0
        summary = mapping._execution_flow_summary(assessed)
        self.assertEqual(summary['known_query_cpu_ns'], 0)
        self.assertEqual(summary['named_chain_cpu_ns'], 0)
        self.assertEqual(summary['top_chain_query_kinds'], [dict(query_name='type_of', cpu_ns=0)])

    def test_mutations_fail_closed(self):
        mutations = [
            lambda f: f.update(edges=0),
            lambda f: f.update(assigned_cpu_segments=0),
            lambda f: f.update(longest_known_chain_cpu_ns=None),
            lambda f: f.update(scope='full-critical-path'),
            lambda f: f.update(records_complete=1),
            lambda f: f.update(full_pipeline_complete=True),
            lambda f: f.update(flow_complete=True),
            lambda f: f.update(cpu_work_ns=9),
            lambda f: f.pop('parallelism'),
            lambda f: f.update(execution_span_ns=6),
            lambda f: f.update(parallelism=1.5),
            lambda f: f.update(nodes=True),
            lambda f: f.update(lost_edges=-1),
            lambda f: f.update(known_cpu_work_ns=True),
            lambda f: f.update(known_query_cpu_ns=7),
            lambda f: f.update(known_cpu_work_ns=None),
            lambda f: f.update(pool_capture_id='foreign'),
            lambda f: f.update(pool_observer=True),
            lambda f: f.update(pool_observer=999),
            lambda f: f.update(issues=['incomplete']),
            lambda f: f.update(limitations=[False]),
            lambda f: f.update(active_scopes=1),
            lambda f: f.update(unassigned_cpu_segments=1),
            lambda f: f.update(longest_known_chain_cpu_ns=7),
            lambda f: f.update(longest_known_chain_path=[1, 1]),
            lambda f: f.update(longest_known_chain_path=[True, 2]),
            lambda f: f.update(longest_known_chain_steps=[]),
            lambda f: f['longest_known_chain_steps'][1].update(node_id=3),
            lambda f: f['longest_known_chain_steps'][1].update(segment_id=None),
            lambda f: f['longest_known_chain_steps'][1].update(query_context=None),
            lambda f: f['longest_known_chain_steps'][1].update(query_job=True),
            lambda f: f['longest_known_chain_steps'][1].update(scope_id=None),
            lambda f: f['longest_known_chain_steps'][1].update(query_name='source/path'),
            lambda f: f['longest_known_chain_steps'][1].update(query_name='λ'),
            lambda f: f['longest_known_chain_steps'][1].pop('query_name'),
            lambda f: f['longest_known_chain_steps'][1].update(cpu_ns=None),
            lambda f: f['longest_known_chain_steps'][1].update(at_ns=0),
            lambda f: f['longest_known_chain_steps'][0].update(cpu_ns=1),
            lambda f: f['longest_known_chain_steps'][0].update(query_name='type_of'),
            lambda f: f['longest_known_chain_steps'][0].update(kind='cut'),
        ]
        for index, mutate in enumerate(mutations):
            assessed = fixture()[3]
            mutate(assessed['execution_flow_dag'])
            with self.subTest(index=index), self.assertRaises(ValueError):
                mapping._execution_flow_summary(assessed)

    def run_analyze(self, root, schema=5, use_cli=False, **options):
        query, authority, pool, assessed = fixture()
        query['schema'] = authority['schema'] = schema
        capture = root / 'capture'
        capture.mkdir()
        originals = {}
        for name, value in (('owner-query.json', query), ('owner-pool.json', pool)):
            originals[name] = json.dumps(value).encode()
            (capture / name).write_bytes(originals[name])
        encoded = (' \n' + json.dumps([authority, assessed], indent=1) + '\n').encode()
        result = subprocess.CompletedProcess([], 0, encoded, b'')
        artifact = root / 'graph.json'
        def validate(command, **kwargs):
            self.assertFalse(kwargs['text'])
            snapshot = Path(command[2])
            self.assertEqual(sorted(p.read_bytes() for p in snapshot.iterdir()), sorted(originals.values()))
            self.assertTrue(all(not p.name.startswith('owner-') for p in snapshot.iterdir()))
            return result
        with patch.object(mapping.subprocess, 'run', side_effect=validate) as validator:
            if use_cli:
                output = root / 'report.json'
                args = ['frontend_mapping.py', '--capture', str(capture), '--justrust', 'validator',
                        '--execution-flow-dag', '--graph-output', str(artifact), '--output', str(output)]
                with patch.object(sys, 'argv', args):
                    mapping.main()
                report = json.loads(output.read_bytes())
                self.assertEqual(output.stat().st_mode & 0o777, 0o600)
            else:
                report = mapping.analyze(capture, 'validator', execution_flow_dag=True,
                                         graph_output=artifact, **options)
        self.assertEqual(artifact.read_bytes(), encoded)
        self.assertEqual(artifact.stat().st_mode & 0o777, 0o600)
        self.assertEqual(report['graph_output']['sha256'], hashlib.sha256(encoded).hexdigest())
        self.assertEqual(report['graph_output']['bytes'], len(encoded))
        command = validator.call_args.args[0]
        self.assertIn('--execution-flow-dag', command)
        for flag in ('--query-cpu-ownership', '--cache-call-provenance',
                     '--dispatch-result-provenance', '--observed-task-dag'):
            self.assertNotIn(flag, command)
        for name, content in originals.items():
            self.assertEqual((capture / name).read_bytes(), content)
        return report

    def test_flow_only_admits_root5_and_root6_and_forwards_exact_artifact(self):
        for schema in (5, 6):
            with self.subTest(schema=schema), tempfile.TemporaryDirectory() as temporary:
                report = self.run_analyze(Path(temporary), schema)
                capture, = report['captures']
                self.assertEqual(capture['execution_flow_dag']['known_cpu_work_ns'], 9)
                for field in ('query_cpu_ownership', 'cache_call_provenance',
                              'dispatch_result_provenance', 'observed_task_dag'):
                    self.assertNotIn(field, capture)

    def test_cli_actual_analyzer_preserves_full_graph_stdout(self):
        with tempfile.TemporaryDirectory() as temporary:
            report = self.run_analyze(Path(temporary), schema=6, use_cli=True)
            self.assertEqual(report['captures'][0]['execution_flow_dag']['named_chain_cpu_ns'], 6)

    def test_missing_or_foreign_public_flow_authority_is_not_fabricated(self):
        for mutate in (lambda a: a.pop('execution_flow_dag'),
                       lambda a: a.update(execution_flow_dag=None),
                       lambda a: a.update(observed_cpu=None),
                       lambda a: a['observed_cpu']['clock_bridge'].update(pool_observer=True)):
            assessed = fixture()[3]
            mutate(assessed)
            with self.subTest(mutate=mutate), self.assertRaises(ValueError):
                mapping._execution_flow_summary(assessed)

    def test_old_flags_cannot_admit_pool5_and_flow_does_not_admit_pool4(self):
        for options in ({}, {'query_cpu_ownership': True}, {'cache_call_provenance': True},
                        {'dispatch_result_provenance': True}, {'observed_task_dag': True},
                        {'execution_flow_dag': True}):
            with self.subTest(options=options), tempfile.TemporaryDirectory() as temporary:
                root = Path(temporary)
                capture = root / 'capture'
                capture.mkdir()
                query, _, pool, _ = fixture()
                if options.get('execution_flow_dag'):
                    pool['schema'] = 4
                (capture / 'pool.json').write_text(json.dumps(pool))
                (capture / 'query.json').write_text(json.dumps(query))
                with patch.object(mapping.subprocess, 'run') as validator:
                    with self.assertRaisesRegex(ValueError, 'pool[45] requires explicit'):
                        mapping.analyze(capture, 'validator',
                                        graph_output=root / 'graph.json' if options else None, **options)
                    validator.assert_not_called()
                    self.assertFalse((root / 'graph.json').exists())

    def test_artifact_requirements_and_collision_precede_validator(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            query, _, pool, _ = fixture()
            capture = root / 'capture'
            capture.mkdir()
            (capture / 'query.json').write_text(json.dumps(query))
            (capture / 'pool.json').write_text(json.dumps(pool))
            existing = root / 'graph.json'
            existing.write_bytes(b'untouched')
            with patch.object(mapping.subprocess, 'run') as validator:
                for output, error in ((None, ValueError), (capture / 'graph.json', ValueError),
                                      (existing, FileExistsError)):
                    with self.subTest(output=output), self.assertRaises(error):
                        mapping.analyze(capture, 'validator', execution_flow_dag=True, graph_output=output)
                validator.assert_not_called()
            self.assertEqual(existing.read_bytes(), b'untouched')

    def test_cli_forwards_flow_only_and_protects_output_paths(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            output, artifact = root / 'report.json', root / 'graph.json'
            args = ['frontend_mapping.py', '--capture', str(root), '--output', str(output),
                    '--graph-output', str(artifact), '--execution-flow-dag']
            with patch.object(sys, 'argv', args), patch.object(mapping, 'analyze', return_value={}) as analyze:
                mapping.main()
            self.assertTrue(analyze.call_args.kwargs['execution_flow_dag'])
            self.assertFalse(analyze.call_args.kwargs['query_cpu_ownership'])
            self.assertEqual(output.stat().st_mode & 0o777, 0o600)
            with patch.object(sys, 'argv', args), patch.object(mapping, 'analyze') as analyze:
                with self.assertRaises(SystemExit):
                    mapping.main()
                analyze.assert_not_called()
            args[args.index(str(artifact))] = str(output)
            with patch.object(sys, 'argv', args), patch.object(mapping, 'analyze') as analyze:
                with self.assertRaises(SystemExit):
                    mapping.main()
                analyze.assert_not_called()


if __name__ == '__main__':
    unittest.main()
