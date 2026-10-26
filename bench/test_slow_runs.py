import json
from pathlib import Path
import tempfile
import unittest

from slow_runs import aggregate, inventory, number, report


class SlowRunsTests(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory()
        self.addCleanup(self.temp.cleanup)
        self.root = Path(self.temp.name)

    def run_fixture(self, name, summary=None, meta=None):
        path = self.root / name
        path.mkdir()
        if summary is not None:
            (path / 'summary.json').write_text(json.dumps(summary))
        if meta is not None:
            (path / 'meta.json').write_text(json.dumps(meta))
        return path

    def test_invalid_numbers(self):
        for value in (None, True, False, '12', [], {}, -1, float('inf'), float('nan'), 10**1000):
            with self.subTest(value=repr(value)):
                self.assertIsNone(number(value))
        self.assertEqual(number(12), 12)

    def test_inventory_coverage_and_read_only(self):
        self.run_fixture('ok', {'wall': 12, 'phases': None, 'subcommand': []})
        self.run_fixture('missing')
        malformed = self.run_fixture('bad')
        (malformed / 'summary.json').write_text('{')
        self.run_fixture('list', [])
        self.run_fixture('invalid', {'wall': True})
        before = {p: p.read_bytes() for p in self.root.rglob('*') if p.is_file()}
        rows, coverage = inventory(self.root)
        self.assertEqual(coverage['run_directories'], 5)
        self.assertEqual(coverage['summary_missing'], 1)
        self.assertEqual(coverage['summary_malformed_or_unreadable'], 2)
        self.assertEqual(coverage['unmeasured_wall'], 4)
        self.assertEqual(rows[0]['cause'], 'unmeasured')
        self.assertEqual(before, {p: p.read_bytes() for p in self.root.rglob('*') if p.is_file()})

    def test_threshold_remote_recent_and_privacy(self):
        self.run_fixture('remote', {'wall': 10, 'start': 999, 'remote': {'client_wall': 40}, 'cwd': '/private/customer/project'})
        self.run_fixture('old', {'wall': 100, 'start': 1, 'exit': 1, 'cwd': '/home/user/repo/bench/synth-x'})
        self.run_fixture('new', {'wall': 20, 'start': 999, 'exit': 0, 'cwd': '/home/user/justrust', 'phases': {'compile': 15, 'lock_wait': 2}})
        self.run_fixture('undated', {'wall': 25})
        result = report(self.root, 10, 1000, days=0.001)
        self.assertEqual(result['all']['slow_runs'], 3)
        self.assertEqual(result['all']['excess_secs'], 115)
        self.assertEqual(result['recent']['slow_runs'], 1)
        self.assertEqual(result['recent_real_workload_candidates']['slow_runs'], 1)
        self.assertEqual(result['all']['remote_client_wall_coverage']['client_slow_but_recorded_wall_not_slow'], 1)
        self.assertNotIn('/private/', json.dumps(result))
        self.assertNotIn('/home/', json.dumps(result))
        self.assertEqual(result['recent']['slowest'][0]['cause'], 'compile')

    def test_metadata_fallback_and_phase_validation(self):
        self.run_fixture('fallback', {'wall': 12, 'phases': {'compile': True, 'tail': 1, 'lock_wait': -1}, 'exit': True},
                         {'cwd': '/home/user/justrust', 'start': 100, 'subcommand': 'check', 'place': 'local'})
        self.run_fixture('malformed-fields', {'wall': 11, 'cwd': [], 'remote': [], 'phases': [], 'subcommand': {}})
        rows, _ = inventory(self.root)
        row = rows[0]
        self.assertEqual((row['workspace'], row['command'], row['start']), ('justrust', 'check', 100))
        self.assertEqual(row['phases'], {'tail': 1})
        self.assertEqual(row['outcome'], 'unknown')
        self.assertEqual(rows[1]['category'], 'unknown')

    def test_last_day_window_is_independent_and_excludes_future(self):
        for name, start in [('day', 100000 - 80000), ('future', 100001)]:
            self.run_fixture(name, {'wall': 20, 'start': start, 'cwd': '/home/user/justrust'})
        result = report(self.root, 10, 100000, days=0)
        self.assertEqual(result['recent']['runs'], 0)
        self.assertEqual(result['recent_24h_real_workload_candidates']['slow_runs'], 1)

    def test_deterministic_ties_and_bounded_groups(self):
        for i in range(25):
            self.run_fixture(f'run-{i:02}', {'wall': 20, 'cwd': f'/private/{i}', 'phases': {'tail': 5, 'compile': 5}})
        rows, _ = inventory(self.root)
        a = aggregate(rows, 10, 2)
        self.assertEqual(a, aggregate(list(reversed(rows)), 10, 2))
        self.assertEqual([r['id'] for r in a['slowest']], ['run-00', 'run-01'])
        self.assertEqual(a['slowest'][0]['cause'], 'compile')
        groups = a['groups']['workspace']
        self.assertEqual(len(groups['by_excess']), 20)
        self.assertEqual(groups['omitted_by_excess']['slow_runs'], 5)


if __name__ == '__main__':
    unittest.main()
