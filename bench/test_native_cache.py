import copy
import json
import unittest
from unittest.mock import Mock

import native_cache


class CacheComparisonTests(unittest.TestCase):
    def result(self, state='off'):
        sample = {'ok': True, 'wall_ns': 1000}
        return {'native_cache': 1, 'state': state, 'complete': True,
                'contract': {'fixtures': ['sample'], 'samples': 3, 'warmups': 1},
                'cells': [{'fixture': 'sample', 'workers': workers, 'opt': opt,
                           'samples': [sample.copy() for _ in range(3)],
                           'warmups': [sample.copy()]}
                          for workers in (1, 8) for opt in ('off', 'final')]}

    def test_matched_cache_states_compare(self):
        before = self.result()
        after = self.result('warm')
        for cell in after['cells']:
            for sample in cell['samples']:
                sample['wall_ns'] = 500
        self.assertIn('2.00x, -50.0%', native_cache.compare(before, after))

    def test_rejects_identity_mismatch_and_incomplete_cells(self):
        before = self.result()
        mutations = [lambda r: r.update(complete=False),
                     lambda r: r['contract'].update(binary_sha256='different'),
                     lambda r: r['cells'].pop(),
                     lambda r: r.update(state='invalid'),
                     lambda r: r['cells'][0]['samples'].pop(),
                     lambda r: r['cells'][0]['warmups'][0].update(ok=False),
                     lambda r: r['cells'][0]['samples'][0].update(wall_ns=0)]
        for mutate in mutations:
            with self.subTest(mutate=mutate):
                after = copy.deepcopy(before)
                mutate(after)
                with self.assertRaises(ValueError):
                    native_cache.compare(before, after)

    def test_summary_requires_complete_successful_build(self):
        summary = {'mode': 'build', 'workers': 8, 'failures': 0,
                   'capture_complete': True, 'events_dropped': 0}
        proc = Mock(returncode=0, stdout=json.dumps(summary).encode(), stderr=b'')
        self.assertEqual(native_cache.checked_summary(proc, 8), summary)
        for key, value in [('capture_complete', False), ('mode', 'check'),
                           ('workers', 1), ('failures', 1), ('events_dropped', 1)]:
            proc.stdout = json.dumps(dict(summary, **{key: value})).encode()
            with self.assertRaises(ValueError):
                native_cache.checked_summary(proc, 8)


if __name__ == '__main__':
    unittest.main()
