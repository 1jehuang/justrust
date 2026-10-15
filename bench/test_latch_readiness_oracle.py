"""Original-wire readiness fixtures, independent of consumer Rust assessments."""
import copy
import json
import unittest

try:
    from .latch_readiness_oracle import latch_readiness_oracle
    from .execution_flow_oracle import execution_flow_oracle
    from .test_execution_flow_oracle import fixture as v1_fixture, node, edge, segment, recount
except ImportError:
    from latch_readiness_oracle import latch_readiness_oracle
    from execution_flow_oracle import execution_flow_oracle
    from test_execution_flow_oracle import fixture as v1_fixture, node, edge, segment, recount


def fixture(registered=True):
    query, pool = v1_fixture()
    pool['schema'] = 6
    query['contexts'][0].update(allocator_high_water=3, expected_jobs=2)
    query['events'] = [dict(kind='admitted', context_id=1, job_id=1, parent_job_id=None, at_ns=0),
                       dict(kind='admitted', context_id=1, job_id=2, parent_job_id=None, at_ns=0),
                       dict(kind='completed', context_id=1, job_id=1, at_ns=25),
                       dict(kind='completed', context_id=1, job_id=2, at_ns=100)]
    query.update(wait_allocator_baseline=1, wait_allocator_high_water=2 if registered else 1,
                 expected_waits=1 if registered else 0, wait_capture_complete=True, wait_events=[])
    if registered:
        for kind, at in [('registered', 7), ('notified', 30), ('condvar_returned', 31), ('resumed', 32)]:
            row = dict(kind=kind, wait_id=1, context_id=1, waitee_job_id=1, consumer_job_id=2, at_ns=at)
            if kind == 'notified':
                row['reason'] = 'completed'
            query['wait_events'].append(row)
    timing = pool['timing']
    timing['segments'] = [segment(1, 5, 10, 3), segment(2, 1, 4, 2), segment(3, 40, 50, 7)]
    for row in timing['segments'][1:]:
        row.update(query_job=2, query_name='mir_built')
    flow = pool['execution_flow']
    flow.update(schema=2, scope='observed-query-join-and-latch-control-flow',
                readiness_allocator_baseline=1, readiness_allocator_high_water=2, expected_readiness=1,
                lost_readiness=0, active_readiness=0, wait_attempt_allocator_baseline=1,
                wait_attempt_allocator_high_water=2, expected_wait_attempts=1, lost_wait_attempts=0,
                active_wait_attempts=0,
                readiness=[dict(id=1, query_context=1, query_job=1, prefix=3, prerequisite=12,
                                visibility_gap=13, outcome='published')],
                wait_attempts=[dict(id=1, query_context=1, waitee_job=1, consumer_job=2, caller_scope=2,
                                   wait_id=1 if registered else None, mode='registered' if registered else 'already_ready',
                                   outcome='consumed', begin=6, wait_gap=7, readiness=1, wake_gap=8, consumed=9)])
    flow['nodes'] = [node(1, 'scope_enter', 1, scope=1), node(2, 'cpu_segment', 5, 1, 1, 1),
                     node(3, 'scope_exit', 11, scope=1, outcome='returned'),
                     node(4, 'scope_enter', 0, scope=2), node(5, 'cpu_segment', 1, 2, 2, 1),
                     node(6, 'wait_begin', 5, scope=2, task=1), node(7, 'cut', 6, scope=2, gap='query_wait'),
                     node(8, 'cut', 33, scope=2, task=1, gap='wake_to_consume'),
                     node(9, 'wait_consumed', 34, scope=2, task=1), node(10, 'cpu_segment', 40, 3, 2, 1),
                     node(11, 'scope_exit', 60, scope=2, outcome='returned'),
                     node(12, 'readiness_prerequisite', 12), node(13, 'cut', 13, gap='latch_visibility')]
    flow['edges'] = [edge(1, 1, 2), edge(2, 2, 3), edge(3, 4, 5), edge(4, 5, 6),
                     edge(5, 3, 12, 'readiness_prepare'), edge(6, 12, 13),
                     edge(7, 6, 8, 'wait_boundary'), edge(8, 8, 9, 'wait_required'),
                     edge(9, 13, 9, 'readiness_required'), edge(10, 9, 10, 'continuation'), edge(11, 10, 11)]
    flow['scopes'] = [dict(id=1, query_context=1, query_job=1, query_name='type_of', entry=1, exit=3, outcome='returned'),
                      dict(id=2, query_context=1, query_job=2, query_name='mir_built', entry=4, exit=11, outcome='returned')]
    recount(flow)
    return query, pool


def unbound_fixture(registered=True):
    query, pool = fixture(registered)
    flow = pool['execution_flow']
    row = flow['wait_attempts'][0]
    row.update(outcome='consumed_unbound', consumed=None, readiness=None)
    flow['nodes'][7]['gap_reason'] = 'readiness_unavailable'
    flow['nodes'] = [n for n in flow['nodes'] if n['id'] != 9]
    flow['edges'] = [e for e in flow['edges'] if e['kind'] not in ('wait_required', 'readiness_required')]
    for e in flow['edges']:
        if e['from'] == 9:
            e['from'] = 8
    mapping = {n['id']: index for index, n in enumerate(flow['nodes'], 1)}
    for n in flow['nodes']:
        n['id'] = mapping[n['id']]
    for index, e in enumerate(flow['edges'], 1):
        e.update(id=index, **{'from': mapping[e['from']], 'to': mapping[e['to']]})
    for scope in flow['scopes']:
        for key in ('entry', 'exit'):
            scope[key] = mapping[scope[key]]
    for ready in flow['readiness']:
        for key in ('prefix', 'prerequisite', 'visibility_gap'):
            ready[key] = mapping[ready[key]]
    recount(flow)
    return query, pool


class ConservativeUnboundTests(unittest.TestCase):
    def test_explicit_policy_retains_costs_but_never_certifies_waits(self):
        for registered in (True, False):
            query, pool = unbound_fixture(registered)
            before = copy.deepcopy((query, pool))
            with self.assertRaisesRegex(ValueError, 'rejects'):
                latch_readiness_oracle(query, pool)
            result = latch_readiness_oracle(query, pool, allow_unbound=True)
            self.assertFalse(result['records_complete'] or result['wait_records_complete'])
            self.assertTrue(result['ready_records_complete'])
            self.assertEqual(result['unbound_wait_attempts'], 1)
            self.assertEqual(result['matched_readiness_transfers'], 0)
            self.assertEqual(result['known_cpu_work_ns'], 12)
            self.assertEqual(result['longest_known_chain_cpu_ns'], 7)
            self.assertEqual((query, pool), before)

    def test_unbound_cannot_invent_success_or_diagnostic_transfer(self):
        for mutation in (
                lambda q, p: p['execution_flow']['wait_attempts'][0].__setitem__('consumed', 9),
                lambda q, p: p['execution_flow']['nodes'][7].__setitem__('gap_reason', 'wake_to_consume'),
                lambda q, p: p['execution_flow']['wait_attempts'][0].__setitem__('readiness', 99),
                lambda q, p: p['execution_flow']['edges'][-2].__setitem__('kind', 'wait_required'),
                lambda q, p: q['wait_events'][1].__setitem__('reason', 'cycle')):
            query, pool = unbound_fixture()
            mutation(query, pool)
            with self.assertRaises(ValueError):
                latch_readiness_oracle(query, pool, allow_unbound=True)


class PositiveReadinessTests(unittest.TestCase):
    def test_registered_original_wait_and_two_obligations(self):
        query, pool = fixture()
        before = copy.deepcopy((query, pool))
        result = latch_readiness_oracle(query, pool)
        self.assertEqual((result['nodes'], result['edges'], result['scopes']), (13, 11, 2))
        self.assertEqual((result['readiness_rows'], result['wait_attempts'], result['matched_readiness_transfers']), (1, 1, 1))
        self.assertEqual(result['known_cpu_work_ns'], 12)
        self.assertEqual(result['longest_known_chain_cpu_ns'], 7)
        self.assertEqual(result['longest_known_chain_path'], [9, 10])
        self.assertEqual(result['cut_nodes'], 3)
        self.assertTrue(result['ready_records_complete'] and result['wait_records_complete'])
        self.assertEqual(result['unassigned_cpu_segments'], 0)
        for key in ('cpu_work_ns', 'execution_span_ns', 'parallelism'):
            self.assertIsNone(result[key])
        self.assertFalse(result['full_pipeline_complete'] or result['flow_complete'])
        self.assertEqual((query, pool), before)
        self.assertEqual(execution_flow_oracle(query, pool, readiness=True), result)

    def test_zero_cpu_resume_and_nullable_consumed_task_are_real_segments(self):
        query, pool = fixture()
        pool['execution_flow']['nodes'][8]['task'] = None
        resumed = pool['timing']['segments'][2]
        resumed.update(cpu_ns=0, cpu_end_ns=resumed['cpu_start_ns'])
        result = latch_readiness_oracle(query, pool)
        self.assertEqual(result['known_cpu_work_ns'], 5)
        self.assertEqual(result['matched_readiness_transfers'], 1)
        self.assertEqual(result['unassigned_cpu_segments'], 0)

    def test_already_ready_rejects_original_poisoned_producer(self):
        query, pool = fixture(registered=False)
        query['events'][2]['kind'] = 'poisoned'
        with self.assertRaisesRegex(ValueError, 'failed or unpublished'):
            latch_readiness_oracle(query, pool)

    def test_already_ready_never_fabricates_registered_lifecycle(self):
        query, pool = fixture(registered=False)
        result = latch_readiness_oracle(query, pool)
        self.assertEqual(query['expected_waits'], 0)
        self.assertEqual(query['wait_events'], [])
        self.assertEqual(result['matched_readiness_transfers'], 1)
        self.assertEqual(result['known_cpu_work_ns'], 12)

    def test_v1_default_and_outputs_unchanged_explicit_v2_policy_only(self):
        query, pool = v1_fixture()
        result = execution_flow_oracle(query, pool)
        self.assertEqual(result, execution_flow_oracle(query, pool, readiness=False))
        self.assertNotIn('readiness_rows', result)
        self.assertEqual(result['known_cpu_work_ns'], 15)
        with self.assertRaisesRegex(ValueError, 'pool6'):
            latch_readiness_oracle(query, pool)
        query, pool = fixture()
        with self.assertRaisesRegex(ValueError, 'pool5'):
            execution_flow_oracle(query, pool)
        self.assertEqual(latch_readiness_oracle(json.dumps(query).encode(), json.dumps(pool))['known_cpu_work_ns'], 12)

    def test_detached_producer_prerequisite_can_have_no_prefix(self):
        query, pool = fixture()
        flow = pool['execution_flow']
        flow['readiness'][0]['prefix'] = None
        flow['edges'] = [e for e in flow['edges'] if e['kind'] != 'readiness_prepare']
        for index, item in enumerate(flow['edges'], 1):
            item['id'] = index
        recount(flow)
        self.assertEqual(latch_readiness_oracle(query, pool)['matched_readiness_transfers'], 1)

    def test_detached_prepare_preserves_ordinary_pending_sync_return(self):
        query, pool = fixture()
        flow = pool['execution_flow']
        flow['nodes'][0]['at_ns'] = 5
        flow['nodes'][5]['at_ns'] = 15
        flow['nodes'][6]['at_ns'] = 16
        query['wait_events'][0]['at_ns'] = 17
        flow['edges'][3] = edge(4, 5, 1, 'sync_call')
        flow['edges'].append(edge(12, 3, 6, 'sync_return'))
        recount(flow)
        result = latch_readiness_oracle(query, pool)
        self.assertEqual(result['matched_readiness_transfers'], 1)
        self.assertEqual(result['known_cpu_work_ns'], 12)
        self.assertEqual(result['longest_known_chain_cpu_ns'], 7)

    def test_same_published_latch_may_fan_out_to_distinct_exact_encounters(self):
        query, pool = fixture()
        flow = pool['execution_flow']
        flow['nodes'].extend([node(14, 'wait_begin', 51, scope=2, task=1),
                              node(15, 'cut', 52, scope=2, gap='query_wait'),
                              node(16, 'cut', 54, scope=2, task=1, gap='wake_to_consume'),
                              node(17, 'wait_consumed', 55, scope=2, task=1),
                              node(18, 'cpu_segment', 56, 4, 2, 1)])
        flow['edges'][10]['to'] = 14
        flow['edges'].extend([edge(12, 14, 16, 'wait_boundary'), edge(13, 16, 17, 'wait_required'),
                              edge(14, 13, 17, 'readiness_required'), edge(15, 17, 18, 'continuation'),
                              edge(16, 18, 11)])
        flow['wait_attempts'].append(dict(id=2, query_context=1, waitee_job=1, consumer_job=2,
                                          caller_scope=2, wait_id=None, mode='already_ready', outcome='consumed',
                                          begin=14, wait_gap=15, readiness=1, wake_gap=16, consumed=17))
        flow.update(expected_wait_attempts=2, wait_attempt_allocator_high_water=3)
        pool['timing']['segments'].append(segment(4, 56, 58, 1))
        pool['timing']['segments'][-1].update(query_job=2, query_name='mir_built')
        pool['timing'].update(expected_segments=4, segment_allocator_high_water=5)
        recount(flow)
        result = latch_readiness_oracle(query, pool)
        self.assertEqual(result['matched_readiness_transfers'], 2)
        self.assertEqual(result['readiness_rows'], 1)
        self.assertEqual(result['known_cpu_work_ns'], 13)
        self.assertEqual(result['longest_known_chain_cpu_ns'], 7)

    def test_original_node_row_order_and_numeric_ids_not_causality(self):
        query, pool = fixture()
        flow = pool['execution_flow']
        mapping = {ident: 14 - ident for ident in range(1, 14)}
        for item in flow['nodes']:
            item['id'] = mapping[item['id']]
        for item in flow['edges']:
            item['from'], item['to'] = mapping[item['from']], mapping[item['to']]
        for item in flow['scopes']:
            item['entry'], item['exit'] = mapping[item['entry']], mapping[item['exit']]
        for item in flow['readiness']:
            for key in ('prefix', 'prerequisite', 'visibility_gap'):
                item[key] = mapping[item[key]]
        for item in flow['wait_attempts']:
            for key in ('begin', 'wait_gap', 'wake_gap', 'consumed'):
                item[key] = mapping[item[key]]
        flow['nodes'].reverse()
        flow['edges'].reverse()
        result = latch_readiness_oracle(query, pool)
        self.assertEqual(result['known_cpu_work_ns'], 12)
        self.assertEqual(result['longest_known_chain_cpu_ns'], 7)


class NegativeReadinessTests(unittest.TestCase):
    def reject(self, change, message='.'):
        query, pool = fixture()
        change(query, pool)
        with self.assertRaisesRegex(ValueError, message):
            latch_readiness_oracle(query, pool)

    def test_incomplete_readiness_or_wait_inventories_never_positive(self):
        for key in ('lost_readiness', 'active_readiness', 'lost_wait_attempts', 'active_wait_attempts'):
            with self.subTest(key=key):
                self.reject(lambda q, p: p['execution_flow'].__setitem__(key, 1), 'complete row')
        self.reject(lambda q, p: p['execution_flow'].__setitem__('readiness_allocator_high_water', None), 'unsigned')
        self.reject(lambda q, p: p['execution_flow'].__setitem__('expected_wait_attempts', 2), 'complete row')
        self.reject(lambda q, p: p['execution_flow'].__setitem__('tls_failed', True), 'incomplete or lost')

    def test_original_wait_exhaustion_missing_or_reused_id_never_success(self):
        self.reject(lambda q, p: q.__setitem__('wait_allocator_high_water', None), 'unsigned')
        self.reject(lambda q, p: q.__setitem__('wait_capture_complete', False), 'unexhausted')
        self.reject(lambda q, p: p['execution_flow']['wait_attempts'][0].__setitem__('wait_id', None), 'unsigned')
        self.reject(lambda q, p: q['wait_events'].pop(), 'stages')
        self.reject(lambda q, p: q['wait_events'].append(copy.deepcopy(q['wait_events'][0])), 'duplicate')
        self.reject(lambda q, p: p['execution_flow']['wait_attempts'][0].__setitem__('wait_id', 2), 'original wait')

    def test_cycle_poison_unbound_and_unwound_never_success(self):
        for outcome in ('active', 'consumed_unbound', 'poisoned', 'cycle', 'unwound'):
            with self.subTest(outcome=outcome):
                self.reject(lambda q, p: p['execution_flow']['wait_attempts'][0].__setitem__('outcome', outcome), 'rejects')
        for outcome in ('active', 'poisoned', 'aborted', 'unavailable'):
            with self.subTest(producer=outcome):
                self.reject(lambda q, p: p['execution_flow']['readiness'][0].__setitem__('outcome', outcome), 'unpublished')
        self.reject(lambda q, p: q['wait_events'][1].__setitem__('reason', 'cycle'), 'completed reason')
        self.reject(lambda q, p: p['execution_flow']['wait_attempts'][0].__setitem__('consumer_job', None), 'unbound')

    def test_actual_original_wait_consumer_waitee_and_lifecycle_required(self):
        self.reject(lambda q, p: q['wait_events'][1].__setitem__('consumer_job_id', 1), 'inconsistent')
        self.reject(lambda q, p: q['wait_events'][2].__setitem__('waitee_job_id', 2), 'inconsistent')
        self.reject(lambda q, p: q['wait_events'][1].__setitem__('at_ns', 6), 'chronology')
        self.reject(lambda q, p: q['events'][2].__setitem__('kind', 'poisoned'), 'unpublished|terminal mismatch')
        self.reject(lambda q, p: p['execution_flow']['wait_attempts'][0].__setitem__('caller_scope', 1), 'another')

    def test_exact_saved_invocation_and_producer_preterminal_bounds(self):
        self.reject(lambda q, p: p['execution_flow']['nodes'][8].__setitem__('at_ns', 101), 'invocation')
        self.reject(lambda q, p: p['execution_flow']['nodes'][11].__setitem__('at_ns', 26), 'preterminal')
        self.reject(lambda q, p: p['execution_flow']['nodes'][5].__setitem__('scope_id', 1), 'original node')
        self.reject(lambda q, p: p['execution_flow']['nodes'][7].__setitem__('task', 2), 'task lifecycle')

    def test_two_exact_obligations_cannot_be_replaced_by_help_tail(self):
        def remove(q, p):
            flow = p['execution_flow']
            flow['edges'] = [e for e in flow['edges'] if e['kind'] != 'readiness_required']
            for index, item in enumerate(flow['edges'], 1):
                item['id'] = index
            recount(flow)
        self.reject(remove, 'two original')
        self.reject(lambda q, p: p['execution_flow']['edges'][8].__setitem__('from', 7), 'two original')
        self.reject(lambda q, p: p['execution_flow']['wait_attempts'][0].__setitem__('readiness', 2), 'published readiness')
        self.reject(lambda q, p: p['execution_flow']['readiness'][0].__setitem__('query_job', 2), 'latch/producer')

    def test_gap_roots_and_visibility_barriers_cannot_bridge_numerically(self):
        def add(q, p, item):
            p['execution_flow']['edges'].append(item)
            recount(p['execution_flow'])
        self.reject(lambda q, p: add(q, p, edge(12, 6, 7)), 'independent root')
        self.reject(lambda q, p: add(q, p, edge(12, 13, 10, 'continuation')), 'invented normal')
        self.reject(lambda q, p: add(q, p, edge(12, 5, 9)), 'two original')
        self.reject(lambda q, p: p['execution_flow']['nodes'][12].__setitem__('kind', 'readiness_prerequisite'), 'original node')

    def test_detached_prefix_is_exact_and_cannot_consume_pending_return(self):
        self.reject(lambda q, p: p['execution_flow']['readiness'][0].__setitem__('prefix', 2), 'saved prefix')
        self.reject(lambda q, p: p['execution_flow']['edges'][4].__setitem__('kind', 'sync_return'), 'saved prefix')
        self.reject(lambda q, p: p['execution_flow']['nodes'][11].__setitem__('scope_id', 1), 'original node')

    def test_original_cpu_and_clock_and_full_metric_contract_still_strict(self):
        self.reject(lambda q, p: q['execution_clock'].__setitem__('pool_observer', 2), 'reciprocal')
        self.reject(lambda q, p: p['timing']['segments'][2].__setitem__('cpu_ns', 20), 'endpoints/delta')
        self.reject(lambda q, p: p['execution_flow']['nodes'][9].__setitem__('segment_id', 2), 'reused')
        self.reject(lambda q, p: p['execution_flow'].__setitem__('flow_complete', True), 'full coverage')


if __name__ == '__main__':
    unittest.main()
