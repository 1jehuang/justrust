"""Consumer-report tests, not native compiler acceptance."""
import copy
import hashlib
import json
from pathlib import Path
import subprocess
import tempfile
import unittest
from unittest.mock import patch

import frontend_mapping as mapping
from test_frontend_mapping import timing_fixture


def fixture():
    query, authority, pool, pool_authority = timing_fixture()
    pool['schema'] = pool_authority['schema'] = 4
    pool['timing']['schema'] = 2
    pool_authority['observed_cpu'].update(task_cpu_ns=30, query_execution_cpu_ns=50)
    authority['query_cpu_ownership'] = dict(
        scope='observed-query-execution-cpu', records_complete=True,
        query_execution_cpu_ns=50, measured_segments=2,
        queries=[dict(query_context=1, query_job=2, query_name='type_of', cpu_ns=35),
                 dict(query_context=1, query_job=1, query_name='mir_built', cpu_ns=15)], issues=[])
    return query, authority, pool, pool_authority


class QueryCpuMappingTests(unittest.TestCase):
    def analyze(self, mutate=None, root6=False):
        query, authority, pool, pool_authority = fixture()
        if root6:
            query['schema'] = authority['schema'] = 6
        if mutate:
            mutate(query, authority, pool, pool_authority)
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            capture = root / 'capture'
            capture.mkdir()
            # These native-router-reserved filenames must be routed neutrally.
            (capture / 'owner-query.json').write_text(json.dumps(query))
            (capture / 'owner-pool.json').write_text(json.dumps(pool))
            encoded = json.dumps([authority, pool_authority]).encode()
            result = subprocess.CompletedProcess([], 0, encoded, b'')
            output = root / 'graph.json'
            with patch.object(mapping.subprocess, 'run', return_value=result) as validator:
                report = mapping.analyze(capture, 'validator', query_cpu_ownership=True,
                                         graph_output=output)
            self.assertEqual(output.read_bytes(), encoded)
            self.assertEqual(output.stat().st_mode & 0o777, 0o600)
            self.assertEqual(report['graph_output']['sha256'], hashlib.sha256(encoded).hexdigest())
            self.assertIn('--query-cpu-ownership', validator.call_args.args[0])
            self.assertNotIn('--dispatch-result-provenance', validator.call_args.args[0])
            return report

    def test_public_query_costs_and_pool_cpu_are_independent_scoped_evidence(self):
        for root6 in (False, True):
            report = self.analyze(root6=root6)
            capture, = report['captures']
            cpu = capture['query_cpu_ownership']
            self.assertEqual(cpu['observed_query_owners'], 2)
            self.assertEqual(cpu['query_execution_cpu_ns'], 50)
            self.assertEqual(cpu['top_queries'][0]['query_job'], 2)
            self.assertEqual(cpu['top_query_kinds'][0], dict(query_name='type_of', cpu_ns=35))
            self.assertNotIn('queries', cpu)
            self.assertNotIn('dispatch_result_provenance', capture)
            self.assertEqual(capture['observed_cpu']['query_execution_cpu_ns'], 50)
            for scope in (capture, report):
                self.assertFalse(scope['full_pipeline_complete'])
                self.assertIsNone(scope['cpu_work_ns'])
                self.assertIsNone(scope['execution_span_ns'])
                self.assertIsNone(scope['theoretical_parallelism'])

    def test_raw_pool_cpu_cannot_supply_missing_query_authority(self):
        def mutate(query, authority, pool, pool_authority):
            del authority['query_cpu_ownership']
            pool['query_cpu_ownership'] = fixture()[1]['query_cpu_ownership']
        with self.assertRaisesRegex(ValueError, 'native query CPU ownership authority'):
            self.analyze(mutate)

    def test_query_cpu_summary_bounds_ranking_and_records_omission(self):
        authority = fixture()[1]
        cpu = authority['query_cpu_ownership']
        cpu['queries'] = [dict(query_context=1, query_job=n + 1, query_name='type_of', cpu_ns=n)
                          for n in range(35)]
        cpu['query_execution_cpu_ns'] = sum(range(35))
        summary = mapping._query_cpu_summary(authority)
        self.assertEqual(summary['observed_query_owners'], 35)
        self.assertEqual(summary['omitted_query_owners'], 15)
        self.assertEqual(len(summary['top_queries']), 20)
        self.assertEqual(summary['top_queries'][0]['cpu_ns'], 34)
        self.assertEqual(summary['observed_query_kinds'], 1)
        self.assertEqual(summary['top_query_kinds'], [dict(query_name='type_of', cpu_ns=sum(range(35)))])

    def test_unknown_query_cpu_stays_unknown_without_rankings(self):
        authority = fixture()[1]
        authority['query_cpu_ownership'].update(records_complete=False,
            query_execution_cpu_ns=None, queries=[], issues=['reciprocal query clock unavailable'])
        summary = mapping._query_cpu_summary(authority)
        self.assertIsNone(summary['query_execution_cpu_ns'])
        self.assertEqual(summary['top_queries'], [])
        self.assertFalse(summary['records_complete'])

    def test_query_cpu_malformed_or_inconsistent_public_facts_fail_closed(self):
        original = fixture()[1]
        mutations = [
            lambda cpu: cpu.update(records_complete=1),
            lambda cpu: cpu.update(measured_segments=True),
            lambda cpu: cpu.update(query_execution_cpu_ns=True),
            lambda cpu: cpu.update(query_execution_cpu_ns=51),
            lambda cpu: cpu.update(query_execution_cpu_ns=None),
            lambda cpu: cpu['queries'][0].update(query_context=True),
            lambda cpu: cpu['queries'][0].update(query_job=0),
            lambda cpu: cpu['queries'][0].update(cpu_ns=-1),
            lambda cpu: cpu['queries'][0].update(query_name='source/path'),
            lambda cpu: cpu['queries'][0].update(query_name='λ'),
            lambda cpu: cpu['queries'][0].update(query_name=None),
            lambda cpu: cpu['queries'].append(dict(cpu['queries'][0])),
            lambda cpu: cpu.update(issues=['clock mismatch']),
        ]
        for mutation in mutations:
            authority = copy.deepcopy(original)
            mutation(authority['query_cpu_ownership'])
            with self.subTest(mutation=mutation), self.assertRaises(ValueError):
                mapping._query_cpu_summary(authority)

    def test_pool4_requires_explicit_opt_in_before_validator_or_output(self):
        query, _, pool, _ = fixture()
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            (root / 'query.json').write_text(json.dumps(query))
            (root / 'pool.json').write_text(json.dumps(pool))
            with patch.object(mapping.subprocess, 'run') as validator:
                with self.assertRaisesRegex(ValueError, 'pool4 requires explicit'):
                    mapping.analyze(root, 'validator')
                validator.assert_not_called()

    def test_cpu_option_requires_exclusive_artifact_outside_capture(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            with patch.object(mapping.subprocess, 'run') as validator:
                with self.assertRaisesRegex(ValueError, 'requires companion'):
                    mapping.analyze(root, 'validator', query_cpu_ownership=True)
                with self.assertRaisesRegex(ValueError, 'outside the original'):
                    mapping.analyze(root, 'validator', query_cpu_ownership=True,
                                    graph_output=root / 'graph.json')
                validator.assert_not_called()

    def test_pool4_cannot_borrow_pool3_native_authority(self):
        def mutate(query, authority, pool, pool_authority):
            pool_authority['schema'] = 3
        with self.assertRaisesRegex(ValueError, 'CPU pool schema'):
            self.analyze(mutate)


if __name__ == '__main__':
    unittest.main()
