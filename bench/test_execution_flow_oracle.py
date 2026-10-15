"""Synthetic original-wire oracle tests, never capture/build or consumer replay."""
import copy
import json
from pathlib import Path
import tempfile
import unittest

try:
    from .execution_flow_oracle import execution_flow_oracle, _known_chain, U64
except ImportError:
    from execution_flow_oracle import execution_flow_oracle, _known_chain, U64


def node(ident, kind, at, segment=None, scope=None, task=None, outcome=None, gap=None):
    return dict(id=ident, kind=kind, at_ns=at, segment_id=segment, scope_id=scope,
                task=task, outcome=outcome, gap_reason=gap)


def edge(ident, left, right, kind='strand'):
    return dict(id=ident, **{'from': left, 'to': right}, kind=kind)


def segment(ident, start, end, cpu, owner='query_execution', task=1, thread=1):
    query = owner == 'query_execution'
    return dict(segment_id=ident, thread_id=thread, task=task, owner_kind=owner,
                query_context=1 if query else None, query_job=1 if query else None,
                query_name='type_of' if query else None, start_ns=start, end_ns=end,
                cpu_start_ns=start, cpu_end_ns=start + cpu, cpu_ns=cpu, status='measured')


def recount(flow):
    for singular, plural in [('node', 'nodes'), ('edge', 'edges'), ('scope', 'scopes')]:
        flow[singular + '_allocator_baseline'] = 1
        flow[singular + '_allocator_high_water'] = len(flow[plural]) + 1
        flow['expected_' + plural] = len(flow[plural])


def fixture():
    bridge = dict(query_capture_id='q-original', pool_capture_id='p-original', pool_observer=1,
                  pool_epoch_in_query_ns=0)
    query = dict(schema=5, producer='rustc-query-admission', scope='query-job-admission-lifecycle',
                 capture_id='q-original', full_pipeline_complete=False, cpu_work_ns=None,
                 execution_span_ns=None, interface_outcome='returned', capture_complete=True,
                 lost_events=0, expected_contexts=1,
                 contexts=[dict(context_id=1, allocator_baseline=1, allocator_high_water=2, expected_jobs=1)],
                 events=[dict(kind='admitted', context_id=1, job_id=1, parent_job_id=None, at_ns=0),
                         dict(kind='completed', context_id=1, job_id=1, at_ns=100)], execution_clock=bridge)
    flow = dict(schema=1, scope='observed-query-and-join-control-flow', full_pipeline_complete=False,
                flow_complete=False, sealed=True, records_complete=True, identity_exhausted=False,
                tls_failed=False, active_scopes=0, lost_nodes=0, lost_edges=0, lost_scopes=0,
                loss_overflowed=False,
                nodes=[node(1, 'scope_enter', 1, scope=1), node(2, 'cpu_segment', 10, 1, 1, 1),
                       node(3, 'scope_exit', 20, scope=1, outcome='returned'),
                       node(4, 'cpu_segment', 30, 2, task=1), node(5, 'cpu_segment', 50, 3)],
                edges=[edge(1, 1, 2), edge(2, 2, 3)],
                scopes=[dict(id=1, query_context=1, query_job=1, query_name='type_of', entry=1, exit=3, outcome='returned')])
    recount(flow)
    timing = dict(schema=2, scope='observed-exclusive-thread-segments', clock_bridge=copy.deepcopy(bridge),
                  cpu_clock='linux-thread-cpu', segment_allocator_baseline=1, segment_allocator_high_water=4,
                  expected_segments=3, thread_allocator_high_water=2, lost_segments=0, loss_overflowed=False,
                  identity_exhausted=False, clock_failures=0, active_segments=0, sealed=True, records_complete=True,
                  segments=[segment(1, 10, 20, 7), segment(2, 30, 40, 3, 'task'),
                            segment(3, 50, 60, 5, 'opaque', None)])
    pool = dict(schema=5, producer='rustc-thread-pool-join', scope='native-join-continuations',
                capture_id='p-original', query_capture_id='q-original', observer=1,
                full_pipeline_complete=False, cpu_work_ns=None, execution_span_ns=None,
                scope_coverage=False, scheduler_coverage=False, pre_attach_coverage=False,
                sealed=True, interface_outcome='returned', task_allocator_baseline=1,
                task_allocator_high_water=2, expected_tasks=1, dropped_events=0,
                events=[dict(kind='root', task=1, at_ns=0), dict(kind='start', task=1, at_ns=0),
                        dict(kind='finish', task=1, at_ns=100, outcome='normal')],
                timing=timing, execution_flow=flow)
    return query, pool


class PositiveFlowTests(unittest.TestCase):
    def test_counts_partitions_and_original_witness(self):
        query, pool = fixture()
        before = copy.deepcopy((query, pool))
        result = execution_flow_oracle(query, pool)
        self.assertEqual((result['nodes'], result['edges'], result['scopes']), (5, 2, 1))
        self.assertEqual((result['known_cpu_work_ns'], result['known_query_cpu_ns'],
                          result['known_task_cpu_ns'], result['known_opaque_cpu_ns']), (15, 7, 3, 5))
        self.assertEqual(result['longest_known_chain_cpu_ns'], 7)
        self.assertEqual(result['longest_known_chain_path'], [1, 2])
        self.assertEqual([s['query_name'] for s in result['longest_known_chain_steps']], [None, 'type_of'])
        self.assertEqual(result['assigned_cpu_segments'], 3)
        for field in ('cpu_work_ns', 'execution_span_ns', 'parallelism'):
            self.assertIsNone(result[field])
        self.assertFalse(result['full_pipeline_complete'])
        self.assertFalse(result['flow_complete'])
        self.assertEqual((query, pool), before)

    def test_bytes_paths_and_package_import(self):
        query, pool = fixture()
        expected = execution_flow_oracle(query, pool)
        self.assertEqual(execution_flow_oracle(json.dumps(query).encode(), json.dumps(pool)), expected)
        with tempfile.TemporaryDirectory() as directory:
            path = Path(directory) / 'pool.json'
            path.write_text(json.dumps(pool))
            self.assertEqual(execution_flow_oracle(query, path), expected)

    def test_row_order_and_reversed_ids_create_no_causality(self):
        query, pool = fixture()
        flow = pool['execution_flow']
        remap = {1: 5, 2: 4, 3: 3, 4: 2, 5: 1}
        for item in flow['nodes']:
            item['id'] = remap[item['id']]
        for item in flow['edges']:
            item['from'], item['to'] = remap[item['from']], remap[item['to']]
        flow['scopes'][0]['entry'] = 5
        flow['nodes'].reverse()
        flow['edges'].reverse()
        result = execution_flow_oracle(query, pool)
        self.assertEqual(result['longest_known_chain_cpu_ns'], 7)
        self.assertEqual(result['longest_known_chain_path'], [5, 4, 3])
        self.assertEqual(result['known_cpu_work_ns'], 15)

    def test_cut_is_unknown_barrier_not_zero_cost_gate(self):
        query, pool = fixture()
        flow = pool['execution_flow']
        # Add a cut after the query scope, then connect the otherwise-disjoint
        # task CPU to it. 7 and 3 must not become a fabricated cost-10 path.
        flow['nodes'].append(node(6, 'cut', 25, gap='dispatch'))
        flow['edges'].extend([edge(3, 3, 6, 'sync_return'), edge(4, 6, 4, 'continuation')])
        # Root invocation has no caller, so use an incoming call from an opaque
        # gate instead and a true reciprocal None caller scope.
        flow['nodes'].append(node(7, 'cut', 0, gap='dispatch'))
        flow['edges'].append(edge(5, 7, 1, 'sync_call'))
        recount(flow)
        result = execution_flow_oracle(query, pool)
        self.assertEqual(result['cut_nodes'], 2)
        self.assertEqual(result['known_cpu_work_ns'], 15)
        self.assertEqual(result['longest_known_chain_cpu_ns'], 7)
        self.assertNotIn(6, result['longest_known_chain_path'])

    def test_explicit_fork_join_edges_and_longest_cost(self):
        query, pool = fixture()
        pool['expected_tasks'], pool['task_allocator_high_water'] = 4, 5
        pool['events'] = [dict(kind='root', task=1, at_ns=0), dict(kind='start', task=1, at_ns=0),
                          dict(kind='fork', parent=1, child=2, continuation=3, at_ns=10),
                          dict(kind='fork', parent=1, child=4, continuation=3, at_ns=10),
                          dict(kind='finish', task=1, at_ns=10, outcome='normal'),
                          dict(kind='start', task=2, at_ns=20),
                          dict(kind='finish', task=2, at_ns=40, outcome='normal'),
                          dict(kind='join', branch=2, child=2, continuation=3, at_ns=40),
                          dict(kind='start', task=4, at_ns=20),
                          dict(kind='finish', task=4, at_ns=45, outcome='normal'),
                          dict(kind='join', branch=4, child=4, continuation=3, at_ns=45),
                          dict(kind='resume', continuation=3, at_ns=50), dict(kind='start', task=3, at_ns=50),
                          dict(kind='finish', task=3, at_ns=90, outcome='normal')]
        pool['timing']['segments'] = [segment(1, 1, 9, 7, task=1),
                                      segment(2, 20, 30, 3, 'task', 2),
                                      segment(3, 60, 70, 5, 'task', 3)]
        flow = pool['execution_flow']
        flow['nodes'] = [node(1, 'scope_enter', 0, scope=1), node(2, 'cpu_segment', 1, 1, 1, 1),
                         node(3, 'scope_exit', 9, scope=1, outcome='returned'),
                         node(4, 'fork', 10, task=1), node(5, 'task_enter', 20, task=2),
                         node(6, 'cpu_segment', 20, 2, task=2), node(7, 'task_exit', 40, task=2, outcome='returned'),
                         node(8, 'join', 50, task=3), node(9, 'cpu_segment', 60, 3, task=3),
                         node(10, 'task_enter', 20, task=4), node(11, 'task_exit', 45, task=4, outcome='returned')]
        flow['edges'] = [edge(1, 1, 2), edge(2, 2, 3), edge(3, 4, 5, 'fork_branch'),
                         edge(4, 5, 6), edge(5, 6, 7), edge(6, 7, 8, 'join_required'),
                         edge(7, 8, 9, 'continuation'), edge(8, 4, 10, 'fork_branch'),
                         edge(9, 10, 11), edge(10, 11, 8, 'join_required')]
        recount(flow)
        result = execution_flow_oracle(query, pool)
        self.assertEqual(result['longest_known_chain_cpu_ns'], 8)
        self.assertEqual(result['longest_known_chain_path'], [4, 5, 6, 7, 8, 9])
        self.assertEqual(result['known_cpu_work_ns'], 15)
        # Complete original task facts may not be omitted from flow.
        flow['edges'].pop(2)
        for index, item in enumerate(flow['edges'], 1):
            item['id'] = index
        recount(flow)
        with self.assertRaisesRegex(ValueError, 'exact original task edges'):
            execution_flow_oracle(query, pool)

    def test_query6_zero_cpu_still_actual_measured_witness(self):
        query, pool = fixture()
        query['schema'] = 6
        for row in pool['timing']['segments']:
            row['cpu_ns'] = 0
            row['cpu_end_ns'] = row['cpu_start_ns']
        result = execution_flow_oracle(query, pool)
        self.assertEqual(result['known_cpu_work_ns'], 0)
        self.assertEqual(result['longest_known_chain_cpu_ns'], 0)
        self.assertTrue(any(row['kind'] == 'cpu_segment' for row in result['longest_known_chain_steps']))


class NegativeFlowTests(unittest.TestCase):
    def mutate(self, fn, message=None):
        query, pool = fixture()
        fn(query, pool)
        with self.assertRaisesRegex((ValueError, KeyError, TypeError), message or '.'):
            execution_flow_oracle(query, pool)

    def test_strict_positive_rejects_every_loss_integrity_and_completeness_gate(self):
        for key, value in [('sealed', False), ('records_complete', False), ('identity_exhausted', True),
                           ('tls_failed', True), ('loss_overflowed', True), ('active_scopes', 1),
                           ('lost_nodes', 1), ('lost_edges', 1), ('lost_scopes', 1)]:
            with self.subTest(key=key):
                self.mutate(lambda q, p: p['execution_flow'].__setitem__(key, value), 'incomplete or lost')
        for key, value in [('records_complete', False), ('sealed', False), ('lost_segments', 1),
                           ('active_segments', 1), ('clock_failures', 1), ('identity_exhausted', True)]:
            with self.subTest(timing=key):
                self.mutate(lambda q, p: p['timing'].__setitem__(key, value))
        self.mutate(lambda q, p: q.__setitem__('capture_complete', False))
        self.mutate(lambda q, p: p.__setitem__('dropped_events', 1))

    def test_pool5_and_no_full_metrics_are_mandatory(self):
        self.mutate(lambda q, p: p.__setitem__('schema', 4), 'pool5')
        self.mutate(lambda q, p: p['timing'].__setitem__('schema', 1), 'timing2')
        self.mutate(lambda q, p: p['execution_flow'].__setitem__('flow_complete', True), 'full coverage')
        self.mutate(lambda q, p: p.__setitem__('cpu_work_ns', 15), 'full metrics')

    def test_nonreciprocal_clock_or_foreign_original_query_rejected(self):
        self.mutate(lambda q, p: q['execution_clock'].__setitem__('pool_observer', 2), 'reciprocal')
        self.mutate(lambda q, p: p['timing']['segments'][0].__setitem__('query_job', 2), 'unadmitted')
        self.mutate(lambda q, p: p['execution_flow']['scopes'][0].__setitem__('query_job', 2), 'admission')
        self.mutate(lambda q, p: q['events'][-1].__setitem__('at_ns', 19), 'query lifetime')
        self.mutate(lambda q, p: q['events'].append(copy.deepcopy(q['events'][0])), 'duplicate')

    def test_cpu_end_not_merely_start_orders_edges(self):
        self.mutate(lambda q, p: p['execution_flow']['nodes'][2].__setitem__('at_ns', 19), 'beyond exact scope')
        def early_edge(q, p):
            # Incoming scope valid, CPU source start unchanged. Only target gate
            # clock violates the CPU END, which must independently be checked.
            f = p['execution_flow']
            f['nodes'].append(node(6, 'cut', 15, scope=1, gap='dispatch'))
            f['edges'][1]['to'] = 6
            recount(f)
        self.mutate(early_edge, 'CPU END')

    def test_unassigned_unknown_and_reused_measurements_rejected(self):
        def unassign(q, p):
            f = p['execution_flow']
            f['nodes'].pop()
            recount(f)
        self.mutate(unassign, 'unassigned')
        self.mutate(lambda q, p: p['timing']['segments'][0].__setitem__('status', 'clock_failed'), 'unknown CPU')
        self.mutate(lambda q, p: p['execution_flow']['nodes'][4].__setitem__('segment_id', 2), 'reused')
        self.mutate(lambda q, p: p['execution_flow']['nodes'][1].__setitem__('at_ns', 11), 'exact original segment')

    def test_category_scope_gate_and_lifecycle_forgery_rejected(self):
        self.mutate(lambda q, p: p['execution_flow']['scopes'][0].__setitem__('query_name', 'other'), 'category')
        self.mutate(lambda q, p: p['execution_flow']['scopes'][0].__setitem__('entry', 2), 'scope gate')
        self.mutate(lambda q, p: p['execution_flow']['nodes'][1].__setitem__('scope_id', None), 'missing invocation')
        self.mutate(lambda q, p: p['timing']['segments'][1].__setitem__('cpu_ns', 4), 'delta')
        self.mutate(lambda q, p: p['execution_flow']['scopes'][0].__setitem__('outcome', 'active'), 'unfinished')

    def test_allocator_unknown_fields_duplicate_and_missing_endpoints(self):
        self.mutate(lambda q, p: p['execution_flow'].__setitem__('expected_nodes', 6), 'inventory')
        self.mutate(lambda q, p: p['execution_flow']['nodes'][4].__setitem__('id', 2), 'duplicate')
        self.mutate(lambda q, p: p['execution_flow']['edges'][0].__setitem__('to', 99), 'missing')
        self.mutate(lambda q, p: p['execution_flow']['nodes'][0].__setitem__('invented', 0), 'fields')
        query, pool = fixture()
        original = json.dumps(pool)
        with self.assertRaisesRegex(ValueError, 'duplicate original JSON'):
            execution_flow_oracle(query, original.replace('{', '{"schema":5,', 1))

    def test_no_implicit_fork_or_sync_return_without_caller(self):
        def branch(q, p):
            f = p['execution_flow']
            f['edges'].append(edge(3, 2, 4))
            recount(f)
        self.mutate(branch, 'strand|implicit')
        def root_return(q, p):
            f = p['execution_flow']
            f['edges'].append(edge(3, 3, 4, 'sync_return'))
            recount(f)
        self.mutate(root_return, 'caller return')

    def test_bool_and_null_are_not_original_u64_identities(self):
        self.mutate(lambda q, p: p['execution_flow']['nodes'][1].__setitem__('task', True), 'unsigned')
        self.mutate(lambda q, p: p['execution_flow'].__setitem__('lost_nodes', False), 'unsigned')
        self.mutate(lambda q, p: p['timing'].__setitem__('segment_allocator_high_water', None), 'unsigned')
        self.mutate(lambda q, p: p['execution_flow']['edges'][0].__setitem__('to', True), 'unsigned')

    def test_original_task_authority_not_only_consumer_flow_rows(self):
        self.mutate(lambda q, p: p['events'][0].__setitem__('at_ns', 1), 'definition/lifetime')
        self.mutate(lambda q, p: p['events'].append(dict(kind='resume', continuation=1, at_ns=0)),
                    'noncontinuation')
        self.mutate(lambda q, p: p['events'][-1].__setitem__('outcome', 'invented'), 'terminal')
        self.mutate(lambda q, p: p['timing']['segments'][1].__setitem__('task', 2), 'outside actual task')

    def test_known_chain_rejects_cycles_and_overflow_not_zero_fallback(self):
        nodes = {1: {'kind': 'cpu_segment'}, 2: {'kind': 'cpu_segment'}}
        with self.assertRaisesRegex(ValueError, 'cyclic'):
            _known_chain(nodes, {(1, 2), (2, 1)}, {1: 0, 2: 0})
        with self.assertRaisesRegex(ValueError, 'unsigned'):
            _known_chain(nodes, {(1, 2)}, {1: U64, 2: 1})
        maximum, path = _known_chain({1: {'kind': 'cpu_segment'}, 2: {'kind': 'cut'},
                                     3: {'kind': 'cpu_segment'}}, {(1, 2), (2, 3)}, {1: 9, 2: None, 3: 10})
        self.assertEqual((maximum, path), (10, [3]))


if __name__ == '__main__':
    unittest.main()
