import json
from pathlib import Path
import subprocess
import sys
import tempfile
import unittest

from reuse_captured import report


class ReuseEvidence(unittest.TestCase):
    def setUp(self):
        temp = tempfile.TemporaryDirectory()
        self.addCleanup(temp.cleanup)
        self.root = Path(temp.name)
        self.units = self.root / 'units.jsonl'

    def test_observations_do_not_prove_savings(self):
        reasons = ['flags: -C linker=x', 'env: CARGO_RESOLVER_X', 'evicted', 'first-seen']
        rows = [dict(depcache='miss', depcache_why=r, user=1, sys=2) for r in reasons]
        rows[0]['reuse'] = [dict(query='q', outcome='same', count=3, secs=2.5)]
        rows.append(dict(depcache='peer', depcache_saved_secs=30))
        self.units.write_text('\n'.join(map(json.dumps, rows)))
        (self.root / 'fresh.jsonl').write_text('{"cpu_secs":9,"basis":"wall"}\n{"basis":"none"}')
        result = report(self.units)
        self.assertIsNone(result['maximum_safe_reuse'])
        self.assertFalse({'actual', 'missed', 'necessary', 'captured_hi'} & result.keys())
        for reason in reasons:
            self.assertEqual(result['counts']['miss_reason:' + reason.split(':')[0]], 1)
        self.assertEqual(result['recorded_unit_cpu_seconds'], {'dependency_cache:miss': 12})
        self.assertEqual(result['counts']['query:same'], 3)
        self.assertEqual(result['recorded_ledger_seconds'], {'query:same': 2.5})
        self.assertEqual(result['estimated_gross_costs'], {'dependency_hit_original_wall_seconds': 30,
                                                          'cargo_fresh_historical_wall_seconds': 9})

    def test_empty_archive_and_empty_unit_file(self):
        out = self.root / 'out'
        command = [sys.executable, str(Path(__file__).with_name('reuse_captured.py')),
                   '--runs', str(self.root), '--output', str(out)]
        subprocess.run(command, check=True, capture_output=True)
        self.assertEqual(out.read_text(), '')
        run = self.root / 'run'
        run.mkdir()
        (run / 'units.jsonl').touch()
        subprocess.run(command, check=True, capture_output=True)
        self.assertIsNone(json.loads(out.read_text())['maximum_safe_reuse'])


if __name__ == '__main__':
    unittest.main()
