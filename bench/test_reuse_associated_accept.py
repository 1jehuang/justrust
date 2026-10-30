import importlib.util
from pathlib import Path
import unittest


def load_gate():
    spec = importlib.util.spec_from_file_location('associated_gate', Path(__file__).with_name('reuse-associated-accept.py'))
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    module.configure()
    return module


class AssociatedAcceptanceTests(unittest.TestCase):
    def test_independent_tracked_experiment(self):
        gate = load_gate()
        self.assertEqual(gate.query.FLAG, '-Zjr-cache-associated-items')
        self.assertEqual(gate.query.SELECTED, ('associated_items',))
        self.assertIn('Dispatch::number(&Widget)', gate.query.BASE)
        self.assertIn('fn opaque(&self) -> impl Copy', gate.query.BASE)

    def test_all_new_edits_change_real_source(self):
        gate = load_gate()
        cases = gate.cases()
        self.assertEqual(len(cases), 34)
        self.assertEqual(len({case[0] for case in cases}), len(cases))
        for name, source, dependency, expected in cases[22:]:
            if name in ('trait-method-revert', 'associated-recovery'):
                self.assertEqual(source, gate.query.BASE)
            else:
                self.assertNotEqual(source, gate.query.BASE, name)
            self.assertTrue(source.startswith('#![allow(dead_code)]'), name)
            self.assertEqual(dependency, gate.query.DEP)
        self.assertEqual([c[0] for c in cases[22:] if c[3] is None], [
            'missing-associated-item', 'duplicate-associated-item', 'ambiguous-method'])

    def test_selected_counts_keep_coverage_checks(self):
        gate = load_gate()
        markers = {name: dict(evidence='observed', records_without_query_name=0,
                             query_name_interval_records={}) for name in (
            'jr_incremental_green_query', 'jr_incremental_green_recompute')}
        self.assertEqual(gate.query.selected_recompute_counts(dict(native_green_reuse=dict(markers=markers))),
                         dict(associated_items=0))
        markers['jr_incremental_green_recompute']['records_without_query_name'] = 1
        with self.assertRaises(AssertionError):
            gate.query.selected_recompute_counts(dict(native_green_reuse=dict(markers=markers)))


if __name__ == '__main__':
    unittest.main()
