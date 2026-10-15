"""Synthetic public-consumer tests only, not native readiness acceptance."""
import copy
import hashlib
import json
from pathlib import Path
import subprocess
import sys
import tempfile
import unittest
from unittest.mock import patch

import frontend_mapping as mapping
from test_execution_flow_mapping import fixture as flow_fixture
from test_frontend_mapping import timing_fixture
from test_observed_task_dag_mapping import authority as task_authority
from test_query_cpu_mapping import fixture as cpu_fixture

FLAGS = ('ready_records_complete', 'wait_records_complete')
COUNTS = ('readiness_rows', 'active_readiness', 'lost_readiness', 'wait_attempts',
          'active_wait_attempts', 'lost_wait_attempts', 'matched_readiness_transfers',
          'unbound_wait_attempts', 'unavailable_readiness_references')


def task_fixture():
    documents = timing_fixture()
    documents[3].update(task_authority())
    return documents


def fixture():
    query, authority, pool, assessed = flow_fixture()
    pool['schema'] = assessed['schema'] = 6
    readiness = assessed.pop('execution_flow_dag')
    readiness.update(scope='observed-query-join-and-latch-control-flow',
                     ready_records_complete=True, wait_records_complete=True,
                     readiness_rows=1, active_readiness=0, lost_readiness=0,
                     wait_attempts=1, active_wait_attempts=0, lost_wait_attempts=0,
                     matched_readiness_transfers=1, unbound_wait_attempts=0,
                     unavailable_readiness_references=0)
    readiness['limitations'] = ['Same-latch prerequisites only, not publication latency or scheduler visibility.']
    assessed['latch_readiness_dag'] = readiness
    return query, authority, pool, assessed


class LatchReadinessMappingTests(unittest.TestCase):
    def test_exact_public_field_scope_and_nullable_known_costs(self):
        assessed = fixture()[3]
        public = assessed['latch_readiness_dag']
        summary = mapping._latch_readiness_summary(assessed)
        for key in (*FLAGS, *COUNTS, 'scope', 'known_cpu_work_ns', 'known_query_cpu_ns',
                    'known_task_cpu_ns', 'known_opaque_cpu_ns', 'issues', 'limitations'):
            self.assertEqual(summary[key], public[key])
        self.assertIsNone(summary['known_task_cpu_ns'])
        self.assertEqual(summary['top_chain_query_kinds'], [dict(query_name='type_of', cpu_ns=6)])
        self.assertNotIn('longest_known_chain_path', summary)
        self.assertNotIn('longest_known_chain_steps', summary)
        self.assertFalse(summary['full_pipeline_complete'])
        self.assertFalse(summary['flow_complete'])
        for key in ('cpu_work_ns', 'execution_span_ns', 'parallelism'):
            self.assertIsNone(summary[key])
        self.assertEqual(summary['issues'], [])
        self.assertTrue(summary['limitations'])
        self.assertNotIn('unbound_reasons', summary)

    def test_readiness_gate_kinds_are_scoped_and_never_cpu_or_query_names(self):
        for kind in ('readiness_prerequisite', 'wait_begin', 'wait_consumed'):
            assessed = fixture()[3]
            gate = assessed['latch_readiness_dag']['longest_known_chain_steps'][0]
            gate['kind'] = kind
            if kind == 'readiness_prerequisite':
                gate.update(scope_id=None, task=None, query_context=None, query_job=None)
            with self.subTest(kind=kind):
                summary = mapping._latch_readiness_summary(assessed)
                self.assertEqual(summary['named_chain_cpu_ns'], 6)
                old = dict(assessed)
                old['execution_flow_dag'] = dict(assessed['latch_readiness_dag'],
                                                scope='observed-query-and-join-control-flow')
                with self.assertRaises(ValueError):
                    mapping._execution_flow_summary(old)
            for mutate in (lambda row: row.update(cpu_ns=1),
                           lambda row: row.update(segment_id=2),
                           lambda row: row.update(query_name='type_of')):
                broken = copy.deepcopy(assessed)
                mutate(broken['latch_readiness_dag']['longest_known_chain_steps'][0])
                with self.subTest(kind=kind, mutate=mutate), self.assertRaises(ValueError):
                    mapping._latch_readiness_summary(broken)
        assessed = fixture()[3]
        assessed['latch_readiness_dag']['longest_known_chain_steps'][0]['kind'] = 'readiness_prerequisite'
        with self.assertRaisesRegex(ValueError, 'caller query or task'):
            mapping._latch_readiness_summary(assessed)

    def test_counts_are_bounded_but_not_an_invented_partition(self):
        assessed = fixture()[3]
        public = assessed['latch_readiness_dag']
        public.update(records_complete=False, wait_attempts=4, matched_readiness_transfers=1,
                      unbound_wait_attempts=1, unavailable_readiness_references=15,
                      issues=['retained poisoned/unqualified wait observations'])
        summary = mapping._latch_readiness_summary(assessed)
        self.assertEqual(summary['unavailable_readiness_references'], 15)
        self.assertEqual(summary['wait_attempts'], 4)
        self.assertEqual(summary['issues'], public['issues'])
        self.assertEqual(summary['limitations'], public['limitations'])

    def test_unknown_authority_is_not_fabricated_zero_and_cuts_cannot_bridge(self):
        assessed = fixture()[3]
        public = assessed['latch_readiness_dag']
        for key in ('known_cpu_work_ns', 'known_query_cpu_ns', 'known_task_cpu_ns',
                    'known_opaque_cpu_ns', 'unassigned_known_cpu_ns', 'longest_known_chain_cpu_ns'):
            public[key] = None
        public.update(records_complete=False, ready_records_complete=False, wait_records_complete=False,
                      matched_readiness_transfers=0, longest_known_chain_path=[],
                      longest_known_chain_steps=[], issues=['original query/wait authority unavailable'])
        summary = mapping._latch_readiness_summary(assessed)
        self.assertIsNone(summary['known_cpu_work_ns'])
        self.assertIsNone(summary['named_chain_cpu_ns'])
        self.assertEqual(summary['top_named_chain_cpu_segments'], [])
        assessed = fixture()[3]
        assessed['latch_readiness_dag']['longest_known_chain_steps'][0]['kind'] = 'cut'
        with self.assertRaises(ValueError):
            mapping._latch_readiness_summary(assessed)

    def test_mutated_public_flags_counts_and_witness_fail_closed(self):
        mutations = [lambda r: r.update(scope='observed-query-and-join-control-flow'),
                     lambda r: r.update(matched_readiness_transfers=2),
                     lambda r: r.update(unbound_wait_attempts=2),
                     lambda r: r.update(full_pipeline_complete=True),
                     lambda r: r.update(flow_complete=True),
                     lambda r: r.update(parallelism=1),
                     lambda r: r.update(known_query_cpu_ns=7),
                     lambda r: r.update(longest_known_chain_cpu_ns=7),
                     lambda r: r['longest_known_chain_steps'][1].update(query_context=None),
                     lambda r: r.update(ready_records_complete=False),
                     lambda r: r.update(wait_records_complete=False)]
        for field in (*FLAGS, *COUNTS):
            mutations.append(lambda r, field=field: r.pop(field))
            mutations.append(lambda r, field=field: r.update({field: 1 if field in FLAGS else True}))
        for field in COUNTS:
            mutations.append(lambda r, field=field: r.update({field: -1}))
        for field in ('active_readiness', 'lost_readiness', 'active_wait_attempts', 'lost_wait_attempts'):
            mutations.append(lambda r, field=field: r.update({field: 1}))
        for index, mutate in enumerate(mutations):
            assessed = fixture()[3]
            mutate(assessed['latch_readiness_dag'])
            with self.subTest(index=index), self.assertRaises(ValueError):
                mapping._latch_readiness_summary(assessed)
        assessed = fixture()[3]
        assessed['readiness_dag'] = assessed.pop('latch_readiness_dag')
        with self.assertRaises(ValueError):
            mapping._latch_readiness_summary(assessed)

    def test_partial_query_authority_retains_independent_nonquery_cpu(self):
        assessed = fixture()[3]
        public = assessed['latch_readiness_dag']
        public.update(records_complete=False, wait_records_complete=False, lost_wait_attempts=1,
                      matched_readiness_transfers=0, known_cpu_work_ns=3, known_query_cpu_ns=None,
                      unavailable_cpu_segments=1, longest_known_chain_cpu_ns=3,
                      longest_known_chain_path=[3], longest_known_chain_steps=[dict(
                          node_id=3, kind='cpu_segment', at_ns=30, segment_id=2, scope_id=None,
                          task=None, query_context=None, query_job=None, query_name=None, cpu_ns=3)],
                      issues=['original wait authority lost'])
        summary = mapping._latch_readiness_summary(assessed)
        self.assertTrue(summary['ready_records_complete'])
        self.assertFalse(summary['wait_records_complete'])
        self.assertEqual(summary['known_opaque_cpu_ns'], 3)
        self.assertIsNone(summary['known_query_cpu_ns'])
        self.assertEqual(summary['longest_known_chain_cpu_ns'], 3)
        self.assertEqual(summary['top_chain_query_kinds'], [])

    def test_source_or_raw_rows_cannot_replace_public_pool6_readiness_authority(self):
        for variant in ('missing-public', 'older-authority', 'forged-old-source'):
            documents = fixture()
            query, authority, pool, assessed = documents
            if variant == 'missing-public':
                pool['latch_readiness_dag'] = assessed.pop('latch_readiness_dag')
            elif variant == 'older-authority':
                assessed['schema'] = 5
            else:
                pool['schema'] = assessed['schema'] = 5
                assessed['execution_flow_dag'] = flow_fixture()[3]['execution_flow_dag']
            options = dict(latch_readiness_dag=True, execution_flow_dag=variant == 'forged-old-source')
            with self.subTest(variant=variant), tempfile.TemporaryDirectory() as temporary:
                with self.assertRaises(ValueError):
                    self.analyze(Path(temporary), documents, options)

    def test_top20_retains_original_named_counts_and_cpu_costs(self):
        assessed = fixture()[3]
        public = assessed['latch_readiness_dag']
        public.update(nodes=35, edges=34, scopes=35, assigned_cpu_segments=35,
                      known_cpu_work_ns=630, known_query_cpu_ns=630, known_opaque_cpu_ns=None,
                      longest_known_chain_cpu_ns=630, longest_known_chain_path=list(range(1, 36)),
                      longest_known_chain_steps=[dict(node_id=n, kind='cpu_segment', at_ns=n,
                          segment_id=n, scope_id=n, task=1, query_context=1, query_job=n,
                          query_name=f'kind_{n}', cpu_ns=n) for n in range(1, 36)])
        summary = mapping._latch_readiness_summary(assessed)
        self.assertEqual(summary['named_chain_cpu_ns'], 630)
        self.assertEqual(summary['named_chain_cpu_segments'], 35)
        self.assertEqual(summary['chain_query_kinds'], 35)
        for key in ('top_named_chain_cpu_segments', 'top_chain_query_kinds'):
            self.assertEqual(len(summary[key]), 20)
            self.assertEqual(summary[key][0]['cpu_ns'], 35)
        self.assertEqual(summary['omitted_named_chain_cpu_segments'], 15)
        self.assertEqual(summary['omitted_chain_query_kinds'], 15)

    def analyze(self, root, documents=None, options=None, use_cli=False):
        query, authority, pool, assessed = documents or fixture()
        options = dict(latch_readiness_dag=True) if options is None else options
        capture = root / 'capture'
        capture.mkdir()
        originals = {name: json.dumps(value).encode() for name, value in
                     (('owner-query.json', query), ('owner-pool.json', pool))}
        for name, content in originals.items():
            (capture / name).write_bytes(content)
        encoded = (' \n' + json.dumps([authority, assessed], indent=1) + '\n').encode()
        artifact = root / 'graph.json'
        def native(command, **kwargs):
            self.assertFalse(kwargs['text'])
            snapshot = Path(command[2])
            self.assertEqual(sorted(p.read_bytes() for p in snapshot.iterdir()), sorted(originals.values()))
            self.assertTrue(all(not p.name.startswith('owner-') for p in snapshot.iterdir()))
            return subprocess.CompletedProcess(command, 0, encoded, b'')
        with patch.object(mapping.subprocess, 'run', side_effect=native) as validator:
            if use_cli:
                output = root / 'report.json'
                args = ['frontend_mapping.py', '--capture', str(capture), '--justrust', 'validator',
                        '--graph-output', str(artifact), '--output', str(output)]
                args.extend('--' + key.replace('_', '-') for key, value in options.items() if value)
                with patch.object(sys, 'argv', args):
                    mapping.main()
                report = json.loads(output.read_bytes())
                self.assertEqual(output.stat().st_mode & 0o777, 0o600)
            else:
                report = mapping.analyze(capture, 'validator', graph_output=artifact, **options)
        self.assertEqual(artifact.read_bytes(), encoded)
        self.assertEqual(artifact.stat().st_mode & 0o777, 0o600)
        self.assertEqual(report['graph_output']['sha256'], hashlib.sha256(encoded).hexdigest())
        self.assertEqual(report['graph_output']['bytes'], len(encoded))
        for name, content in originals.items():
            self.assertEqual((capture / name).read_bytes(), content)
        return report, validator.call_args.args[0]

    def test_readiness_only_root5_root6_actual_cli_forwarding_and_exact_artifact(self):
        for schema in (5, 6):
            documents = fixture()
            documents[0]['schema'] = documents[1]['schema'] = schema
            with self.subTest(schema=schema), tempfile.TemporaryDirectory() as temporary:
                report, command = self.analyze(Path(temporary), documents, use_cli=True)
                capture, = report['captures']
                self.assertIn('latch_readiness_dag', capture)
                self.assertIn('--latch-readiness-dag', command)
                for field in ('execution_flow_dag', 'query_cpu_ownership', 'cache_call_provenance',
                              'dispatch_result_provenance', 'observed_task_dag'):
                    self.assertNotIn(field, capture)
                    self.assertNotIn('--' + field.replace('_', '-'), command)

    def test_additive_flow_flag_on_pool6_does_not_invent_v1_projection(self):
        with tempfile.TemporaryDirectory() as temporary:
            report, command = self.analyze(Path(temporary), options=dict(
                latch_readiness_dag=True, execution_flow_dag=True))
            self.assertIn('--execution-flow-dag', command)
            self.assertIn('latch_readiness_dag', report['captures'][0])
            self.assertNotIn('execution_flow_dag', report['captures'][0])

    def test_old_schema_outputs_are_exactly_preserved_with_additive_readiness_flag(self):
        for make, options in ((task_fixture, dict(observed_task_dag=True)),
                              (cpu_fixture, dict(query_cpu_ownership=True)),
                              (flow_fixture, dict(execution_flow_dag=True))):
            documents = make()
            with self.subTest(schema=documents[2]['schema']), tempfile.TemporaryDirectory() as first, tempfile.TemporaryDirectory() as second:
                before, _ = self.analyze(Path(first), copy.deepcopy(documents), options)
                after, _ = self.analyze(Path(second), copy.deepcopy(documents), dict(options, latch_readiness_dag=True))
                self.assertEqual(json.dumps(before, sort_keys=True, indent=2),
                                 json.dumps(after, sort_keys=True, indent=2))
                self.assertNotIn('latch_readiness_dag', after['captures'][0])

    def test_pool6_requires_new_flag_and_new_flag_does_not_admit_pool4_or_pool5(self):
        cases = [(6, {}), (6, dict(query_cpu_ownership=True)), (6, dict(execution_flow_dag=True)),
                 (6, dict(cache_call_provenance=True)), (6, dict(dispatch_result_provenance=True)),
                 (6, dict(observed_task_dag=True)), (4, dict(latch_readiness_dag=True)),
                 (5, dict(latch_readiness_dag=True))]
        for schema, options in cases:
            with self.subTest(schema=schema, options=options), tempfile.TemporaryDirectory() as temporary:
                root = Path(temporary)
                query, _, pool, _ = fixture()
                pool['schema'] = schema
                capture = root / 'capture'
                capture.mkdir()
                (capture / 'query.json').write_text(json.dumps(query))
                (capture / 'pool.json').write_text(json.dumps(pool))
                with patch.object(mapping.subprocess, 'run') as validator:
                    with self.assertRaisesRegex(ValueError, f'pool{schema} requires explicit'):
                        mapping.analyze(capture, 'validator', graph_output=root / 'graph.json' if options else None, **options)
                    validator.assert_not_called()
                    self.assertFalse((root / 'graph.json').exists())

    def test_companion_artifact_and_cli_collision_guards_precede_validator(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            capture = root / 'capture'
            capture.mkdir()
            query, _, pool, _ = fixture()
            (capture / 'query.json').write_text(json.dumps(query))
            (capture / 'pool.json').write_text(json.dumps(pool))
            graph = root / 'graph.json'
            graph.write_bytes(b'untouched')
            with patch.object(mapping.subprocess, 'run') as validator:
                for path, error in ((None, ValueError), (capture / 'graph.json', ValueError), (graph, FileExistsError)):
                    with self.subTest(path=path), self.assertRaises(error):
                        mapping.analyze(capture, 'validator', latch_readiness_dag=True, graph_output=path)
                validator.assert_not_called()
            self.assertEqual(graph.read_bytes(), b'untouched')
            output = root / 'report.json'
            output.write_bytes(b'report unchanged')
            args = ['frontend_mapping.py', '--capture', str(capture), '--latch-readiness-dag',
                    '--graph-output', str(graph), '--output', str(output)]
            with patch.object(sys, 'argv', args), patch.object(mapping.subprocess, 'run') as validator:
                with self.assertRaises(SystemExit):
                    mapping.main()
                validator.assert_not_called()
            args[args.index(str(graph))] = str(output)
            with patch.object(sys, 'argv', args), patch.object(mapping.subprocess, 'run') as validator:
                with self.assertRaises(SystemExit):
                    mapping.main()
                validator.assert_not_called()
            self.assertEqual(output.read_bytes(), b'report unchanged')


if __name__ == '__main__':
    unittest.main()
