import unittest
import audit_hosted_evidence as audit

RID = '20261013-113748084-2040116-pre'


class EvidenceAuditTests(unittest.TestCase):
    def row(self):
        return {'id': RID, 'exit': 0, 'args': ['test'], 'toolchain': 'nightly-2026-10-08',
                'remote': {'machine': 'hosted'}, 'units': {'compiled': 0, 'fresh': 56},
                'tests': {'passed': 360, 'failed': 0, 'ignored': 2, 'filtered_out': 0}}

    def test_full_suffixed_reference_is_not_truncated(self):
        self.assertEqual(list(audit.refs('run `' + RID + '`')), [RID])

    def test_dictionary_key_references_are_checked(self):
        self.assertEqual(list(audit.refs({RID: {'other': 1}})), [RID])

    def test_nested_suite_and_round_robin_observations(self):
        sample = {'id': RID, 'exit': 0, 'tests_passed': 360}
        for value in [{'scenarios': {'x': {'samples': [sample]}}},
                      {'workloads': {'x': {'samples': {'remote': [sample]}}}}]:
            self.assertEqual(list(audit.observations(value)), [sample])

    def test_stock_sample_without_record_id_is_not_fabricated(self):
        self.assertEqual(list(audit.observations({'id': None, 'tests_passed': 3})), [])

    def test_valid_flat_acceptance_matches(self):
        sample = {'id': RID, 'exit': 0, 'compiled': 0, 'fresh': 56,
                  'compiler': 'nightly-2026-10-08', 'tests': {'passed': 360, 'failed': 0}}
        self.assertEqual(audit.differences('x', sample, self.row()), [])

    def test_mutated_test_count_exit_and_units_are_rejected(self):
        sample = {'id': RID, 'exit': 101, 'units': 1, 'tests_passed': 359}
        self.assertEqual({m['field'] for m in audit.differences('x', sample, self.row())},
                         {'exit', 'units', 'tests_passed'})

    def test_remote_fallback_and_wrong_compiler_are_rejected(self):
        row = self.row()
        row['remote'] = None
        row['toolchain'] = None
        sample = {'id': RID, 'remote': {'machine': 'hosted'}, 'compiler': 'jr-b7907f1fe8f7'}
        self.assertEqual({m['field'] for m in audit.differences('x', sample, row)},
                         {'remote.machine', 'toolchain'})

    def test_nested_counts_and_arguments_are_rejected(self):
        sample = {'id': RID, 'args': ['check'], 'tests': {'failed': 1, 'ignored': 0}}
        self.assertEqual({m['field'] for m in audit.differences('x', sample, self.row())},
                         {'args', 'tests.failed', 'tests.ignored'})


if __name__ == '__main__':
    unittest.main()
