import copy
import unittest

from observed_task_dag_oracle import components_and_chains, native_task_oracle


def fixture():
    events = [dict(kind='root', task=1, at_ns=0),
              dict(kind='start', task=1, at_ns=0),
              dict(kind='fork', parent=1, child=2, continuation=4, at_ns=10),
              dict(kind='fork', parent=1, child=3, continuation=4, at_ns=10),
              dict(kind='finish', task=1, at_ns=10),
              dict(kind='start', task=2, at_ns=10),
              dict(kind='start', task=3, at_ns=10),
              dict(kind='finish', task=2, at_ns=20),
              dict(kind='finish', task=3, at_ns=30),
              dict(kind='join', branch=2, child=2, continuation=4, at_ns=30),
              dict(kind='join', branch=3, child=3, continuation=4, at_ns=30),
              dict(kind='resume', continuation=4, at_ns=30),
              dict(kind='start', task=4, at_ns=30),
              dict(kind='finish', task=4, at_ns=40)]
    segments = [dict(task=task, owner_kind=kind, start_ns=start, end_ns=end,
                     cpu_ns=cpu, status='measured')
                for task, kind, start, end, cpu in
                [(1, 'task', 0, 10, 4), (2, 'query_execution', 10, 20, 7),
                 (3, 'task', 10, 30, 9), (4, 'task', 30, 40, 3),
                 (None, 'query_execution', 100, 110, 8),
                 (None, 'opaque', 110, 120, 2)]]
    return dict(expected_tasks=4, events=events, sealed=True, interface_outcome='returned',
                dropped_events=0, timing=dict(segments=segments, records_complete=True,
                    sealed=True, lost_segments=0, active_segments=0, expected_segments=len(segments)))


class TaskDagOracleTests(unittest.TestCase):
    def test_explicit_edges_and_disjoint_cpu(self):
        result = native_task_oracle(fixture())
        self.assertEqual(result['edges'], [(1, 2), (1, 3), (2, 4), (3, 4)])
        self.assertEqual(result['weights'], {1: 4, 2: 7, 3: 9, 4: 3})
        self.assertEqual(result['excluded'], {'query_execution': 8, 'opaque': 2})
        self.assertEqual(result['components'][0]['known_cpu_chain_ns'], 16)
        self.assertEqual(result['components'][0]['known_cpu_chain_tasks'], [1, 3, 4])

    def test_disconnected_work_is_not_serialized(self):
        result = components_and_chains({1, 2}, set(), {1: 5, 2: 8})
        self.assertEqual([row['known_cpu_chain_ns'] for row in result], [5, 8])
        self.assertEqual([row['tasks'] for row in result], [[1], [2]])

    def test_equal_paths_have_deterministic_lexical_tie(self):
        result = components_and_chains({1, 2, 3, 4}, {(1, 3), (1, 2), (3, 4), (2, 4)},
                                       {1: 1, 2: 5, 3: 5, 4: 1})
        self.assertEqual(result[0]['known_cpu_chain_tasks'], [1, 2, 4])

    def test_unknown_cpu_is_not_zero(self):
        pool = fixture()
        pool['timing']['segments'][0]['cpu_ns'] = None
        with self.assertRaisesRegex(ValueError, 'measured original'):
            native_task_oracle(pool)

    def test_cpu_outside_task_is_rejected(self):
        pool = fixture()
        pool['timing']['segments'][0]['end_ns'] = 11
        with self.assertRaisesRegex(ValueError, 'outside actual task'):
            native_task_oracle(pool)

    def test_reachable_cycle_is_rejected(self):
        with self.assertRaisesRegex(ValueError, 'cyclic'):
            components_and_chains({1, 2, 3}, {(1, 2), (2, 3), (3, 2)}, {1: 1, 2: 2, 3: 3})

    def test_incomplete_original_inventory_cannot_pass_positive_gate(self):
        mutations = [lambda pool: pool.update(dropped_events=1),
                     lambda pool: pool['timing'].update(records_complete=False),
                     lambda pool: pool['timing'].update(lost_segments=1),
                     lambda pool: pool['timing'].update(active_segments=1),
                     lambda pool: pool['timing'].update(expected_segments=7)]
        for mutation in mutations:
            pool = fixture()
            mutation(pool)
            with self.subTest(mutation=mutation), self.assertRaisesRegex(ValueError, 'complete original'):
                native_task_oracle(pool)

    def test_original_input_is_not_mutated(self):
        pool = fixture()
        before = copy.deepcopy(pool)
        native_task_oracle(pool)
        self.assertEqual(pool, before)


if __name__ == '__main__':
    unittest.main()
