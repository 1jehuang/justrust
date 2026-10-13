import contextlib
import copy
import io
import tempfile
from types import SimpleNamespace
import json
from pathlib import Path
import unittest

import versus


class StockReportContentionTests(unittest.TestCase):
    def result(self):
        return {'stacks': ['stock', 'remote'], 'workloads': {'reference-loop': {
            'cold': False,
            'samples': {s: [{'noisy': False}, {'noisy': False}] for s in ['stock', 'remote']},
            'stats': {
                'stock': {'median': 10.0, 'min': 9.0, 'max': 11.0, 'instructions': 100},
                'remote': {'median': 5.0, 'min': 4.5, 'max': 5.5, 'instructions': 80},
            },
        }}}

    def test_actual_all_noisy_stock_artifact_does_not_claim_speed(self):
        path = Path(__file__).parent / 'results' / 'versus-20261013-164114-e6a1b5e-with-existing-seed-control-dirty.json'
        result = json.loads(path.read_text())
        samples = [s for w in result['workloads'].values() for ss in w['samples'].values() for s in ss]
        self.assertEqual(len(samples), 42)
        self.assertTrue(all(s['noisy'] for s in samples))
        text = versus.scoreboard(result)
        self.assertIn('contended', text)
        self.assertNotIn('**2.05x**', text)
        self.assertNotIn('**1.26x**', text)
        self.assertIn('10.97s', text, 'retain observed timings without attributing a causal speedup')

    def test_one_noisy_sample_blocks_pair_and_geomean(self):
        result = self.result()
        result['workloads']['reference-loop']['samples']['stock'][0]['noisy'] = True
        text = versus.scoreboard(result)
        self.assertIn('contended', text)
        self.assertNotIn('**2.00x**', text)
        self.assertIn('not comparable', text)

    def test_clean_samples_keep_existing_speed_verdict(self):
        text = versus.scoreboard(self.result())
        self.assertEqual(text.count('**2.00x**'), 2)
        self.assertNotIn('contended', text)

    def test_missing_contention_provenance_does_not_claim_speed(self):
        for remove in ['samples', 'noisy']:
            result = self.result()
            if remove == 'samples':
                result['workloads']['reference-loop'].pop('samples')
            else:
                result['workloads']['reference-loop']['samples']['remote'][0].pop('noisy')
            text = versus.scoreboard(result)
            self.assertIn('unverified contention', text)
            self.assertNotIn('**2.00x**', text)

    def test_geomean_does_not_silently_drop_contended_workload(self):
        result = self.result()
        result['workloads']['body-check'] = copy.deepcopy(result['workloads']['reference-loop'])
        result['workloads']['body-check']['samples']['remote'][0]['noisy'] = True
        text = versus.scoreboard(result)
        self.assertEqual(text.count('**2.00x**'), 1, 'only the clean workload has an attributable ratio')
        self.assertIn('not comparable', text.split('**geomean**')[1])

    def test_compare_command_blocks_noisy_attribution(self):
        result = self.result()
        result['env'] = {'snapshots': {}, 'justrust_rev': 'test'}
        result['workloads']['reference-loop']['samples']['remote'][0]['noisy'] = True
        with tempfile.TemporaryDirectory() as directory:
            path = Path(directory) / 'result.json'
            path.write_text(json.dumps(result))
            output = io.StringIO()
            with contextlib.redirect_stdout(output):
                versus.cmd_compare(SimpleNamespace(a=str(path), b=str(path)))
        line = next(line for line in output.getvalue().splitlines() if line.startswith('reference-loop') and 'remote' in line)
        self.assertIn('contended', line)
        self.assertEqual(line.split()[4], '-', 'no causal wall-percent claim from noisy samples')


if __name__ == '__main__':
    unittest.main()
