import copy
import importlib.util
from pathlib import Path
import unittest


spec = importlib.util.spec_from_file_location('gate', Path(__file__).with_name('reuse-cgu-native-gate.py'))
gate = importlib.util.module_from_spec(spec)
spec.loader.exec_module(gate)


class BoundedCguAdmission(unittest.TestCase):
    def setUp(self):
        self.full = [(1304, 0, 10, 0)] * 2
        self.rows = []
        for i in range(12):
            for arm in ('control', 'candidate'):
                cgu = {'new': 250} if not i or i >= 8 else ({'new': 130, 'loaded': 126} if arm == 'control'
                                                           else {'changed': 5, 'loaded': 249})
                wall = 15 if not i or i >= 8 else (8 if arm == 'control' else 6)
                self.rows.append(dict(i=i, arm=arm, cgu=cgu, units=1, source=f'{i:064x}',
                                      wall=wall, compile=wall - 1, link=0.5, rss_mb=2000))

    def test_valid(self):
        self.assertEqual(gate.evaluate(self.rows, self.full)['wins'], 6)

    def test_reject_invalid(self):
        for index, fields in [(3, {'source': 'a' * 64}), (4, {'units': 2}), (5, {'cgu': {'new': 254}}),
                              (19, {'cgu': {'loaded': 254}}), (3, {'cgu': {'changed': 300}})]:
            with self.subTest(fields=fields):
                rows = copy.deepcopy(self.rows)
                rows[index].update(fields)
                with self.assertRaises(ValueError):
                    gate.evaluate(rows, self.full)
        for rows, full in [(self.rows[:-1], self.full), (self.rows, [(1, 0, 0, 0), self.full[1]]),
                           ([{**r, 'wall': 20} if r['i'] >= 8 and r['arm'] == 'candidate' else r for r in self.rows], self.full)]:
            with self.assertRaises(ValueError):
                gate.evaluate(rows, full)


if __name__ == '__main__':
    unittest.main()
