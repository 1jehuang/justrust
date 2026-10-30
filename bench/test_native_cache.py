import copy
import json
import unittest
from unittest.mock import Mock

import native_cache


class CacheComparisonTests(unittest.TestCase):
    def result(self, state='off'):
        sample = {'ok': True, 'wall_ns': 1000}
        return {'native_cache': 1, 'state': state, 'complete': True,
                'contract': {'fixtures': ['sample'], 'samples': 3, 'warmups': 1,
                             'workers': [1, 8], 'opts': ['off', 'final'],
                             'binary_sha256': 'a' * 64, 'runner_sha256': 'c' * 64,
                             'suite_sha256': 'd' * 64, 'inputs': {'sample.rs': 'e' * 64},
                             'host': 'benchmark-host', 'timeout': 120},
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

    def test_binary_only_change_requires_explicit_api_opt_in(self):
        before, after = self.result(), self.result()
        after['contract']['binary_sha256'] = 'b' * 64
        after['allow_binary_change'] = True
        with self.assertRaises(ValueError):
            native_cache.compare(before, after)
        report = native_cache.compare(before, after, allow_binary_change=True)
        self.assertIn('a' * 64 + ' -> ' + 'b' * 64, report)
        self.assertIn('binary change explicitly allowed', report)

    def test_cross_binary_rejects_every_other_contract_difference(self):
        before = self.result()
        for key in before['contract']:
            if key == 'binary_sha256':
                continue
            with self.subTest(key=key):
                after = copy.deepcopy(before)
                after['contract'].update(binary_sha256='b' * 64)
                after['contract'][key] = 'changed'
                with self.assertRaises(ValueError):
                    native_cache.compare(before, after, allow_binary_change=True)
        after = copy.deepcopy(before)
        after['contract']['new_option'] = True
        with self.assertRaises(ValueError):
            native_cache.compare(before, after, allow_binary_change=True)

    def test_cross_binary_rejects_incomplete_or_malformed_results_on_either_side(self):
        mutations = [lambda r: r.update(complete=False),
                     lambda r: r['cells'].pop(),
                     lambda r: r['cells'].append(copy.deepcopy(r['cells'][0])),
                     lambda r: r.update(cells=None),
                     lambda r: r['cells'][0].pop('fixture'),
                     lambda r: r['cells'][0]['samples'].pop(),
                     lambda r: r['cells'][0].update(warmups=[]),
                     lambda r: r['cells'][0].update(samples=None),
                     lambda r: r['cells'][0]['samples'].__setitem__(0, None),
                     lambda r: r['cells'][0]['samples'][0].update(ok=False),
                     lambda r: r['cells'][0]['samples'][0].update(wall_ns=True)]
        for side in (0, 1):
            for mutation in mutations:
                with self.subTest(side=side, mutation=mutation):
                    pair = [self.result(), self.result()]
                    pair[1]['contract']['binary_sha256'] = 'b' * 64
                    mutation(pair[side])
                    with self.assertRaises(ValueError):
                        native_cache.compare(*pair, allow_binary_change=True)

    def test_binary_hashes_must_be_valid_on_both_sides(self):
        for side in (0, 1):
            for value in (None, '', 'a' * 63, 'g' * 64, 123, 'b' * 64 + '\n'):
                with self.subTest(side=side, value=value):
                    pair = [self.result(), self.result()]
                    pair[side]['contract']['binary_sha256'] = value
                    with self.assertRaises(ValueError):
                        native_cache.compare(*pair, allow_binary_change=True)

    def test_empty_coverage_cannot_validate_vacuously(self):
        for field, value in [('fixtures', []), ('samples', 0), ('warmups', 0),
                             ('workers', [1]), ('opts', ['off'])]:
            with self.subTest(field=field):
                before = self.result()
                before['contract'][field] = value
                with self.assertRaises(ValueError):
                    native_cache.compare(before, copy.deepcopy(before), allow_binary_change=True)


if __name__ == '__main__':
    unittest.main()
