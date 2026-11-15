#!/usr/bin/env python3
"""Recorded remote noise provenance, without executing builds."""
import copy
import json
from pathlib import Path
import tempfile
from types import SimpleNamespace
import unittest
from unittest.mock import patch

import versus


class RemoteNoiseTests(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory()
        self.addCleanup(self.temp.cleanup)
        self.home = Path(self.temp.name)
        self.record = self.home / 'runs' / 'fixture'
        self.record.mkdir(parents=True)
        self.meta = {'id': 'fixture', 'place': 'hosted', 'ncpu': 16}
        self.rows = [{'t': 100 + i * .2, 'sys_cores': 1., 'build_cores': 1.,
                      'foreign_rustc_procs': 0} for i in range(6)]
        self.summary = {'id': 'fixture', 'place': 'hosted', 'ncpu': 16,
                        'start': 100., 'wall': 1., 'remote': {'machine': 'hosted', 'sync_ms': 2},
                        'measurement_coverage': {'samples': 6, 'sample_span_secs': 1.,
                                                 'max_sample_gap_secs': .2},
                        'resources': {'avg_other_cores': 0., 'max_foreign_rustc': 0}}
        self.patcher = patch.object(versus.suite, 'JR_HOME', self.home)
        self.patcher.start()
        self.addCleanup(self.patcher.stop)

    def write(self):
        (self.record / 'meta.json').write_text(json.dumps(self.meta))
        (self.record / 'summary.json').write_text(json.dumps(self.summary))
        (self.record / 'samples.jsonl').write_text('\n'.join(json.dumps(r) for r in self.rows))

    def mark(self, client=0., stack=None):
        row = {'id': 'fixture', 'cpu_noise_valid': True, 'avg_other_cores': client,
               'cpu_noise_scope': 'invocation-host', 'instructions': 100}
        versus._mark_remote(stack or versus.REMOTE, row, '')
        return row

    def test_quiet_and_automatic_hosted_stack(self):
        self.write()
        for stack in (versus.REMOTE, versus.STACKS[-1]):
            row = self.mark(stack=stack)
            self.assertTrue(row['cpu_noise_valid'])
            self.assertEqual(row['remote']['machine'], 'hosted')
            self.assertFalse(versus.sample_noisy(row, 0))
            self.assertIsNone(row['instructions'])

    def test_client_noise_and_foreign_processes_survive(self):
        self.write()
        row = self.mark(client=versus.suite.NOISY_CORES + 1)
        self.assertTrue(versus.sample_noisy(row, 0))
        self.assertGreater(row['client_cpu_noise']['avg_other_cores'], versus.suite.NOISY_CORES)
        self.assertTrue(versus.sample_noisy(self.mark(), 1))

    def test_remote_cpu_and_foreign_rustc_noise_independently(self):
        for other, foreign in ((15.7, 6), (0, 6), (15.7, 0)):
            with self.subTest(other=other, foreign=foreign):
                for row in self.rows:
                    row.update(sys_cores=other, build_cores=0, foreign_rustc_procs=foreign)
                self.summary['resources'] = {'avg_other_cores': other, 'max_foreign_rustc': foreign}
                self.write()
                row = self.mark()
                self.assertTrue(row['cpu_noise_valid'])
                self.assertTrue(versus.sample_noisy(row, 0))

    def test_missing_files_fail_closed(self):
        for name in ('meta.json', 'summary.json', 'samples.jsonl'):
            self.write()
            (self.record / name).unlink()
            self.assertFalse(self.mark()['cpu_noise_valid'])
            self.assertTrue(versus.sample_noisy(self.mark(), 0))

    def test_malformed_short_inconsistent_fail_closed(self):
        base = copy.deepcopy((self.meta, self.summary, self.rows))
        mutations = [lambda: self.rows.pop(),
                     lambda: self.rows.clear(),
                     lambda: self.rows[2].update(t=100),
                     lambda: self.rows[2].update(sys_cores=float('nan')),
                     lambda: self.rows[2].update(foreign_rustc_procs=-1),
                     lambda: self.summary.update(id='wrong'),
                     lambda: self.meta.update(place='local'),
                     lambda: self.summary['resources'].update(avg_other_cores=1),
                     lambda: self.summary.update(wall=10),
                     lambda: self.summary['measurement_coverage'].update(max_sample_gap_secs=0)]
        for mutate in mutations:
            self.meta, self.summary, self.rows = copy.deepcopy(base)
            mutate()
            self.write()
            row = self.mark()
            self.assertFalse(row['cpu_noise_valid'], row)
            self.assertTrue(versus.sample_noisy(row, 0))
        self.write()
        (self.record / 'samples.jsonl').write_text('{broken')
        self.assertFalse(self.mark()['cpu_noise_valid'])

    def test_actual_recorded_contention_when_available(self):
        record = Path.home() / '.justrust/runs/20261115-061530872-2556336631'
        if not record.exists():
            self.skipTest('optional recorded hosted fixture unavailable')
        evidence = versus._remote_cpu_noise(record, record.name,
                                            json.loads((record / 'summary.json').read_text()))
        self.assertTrue(evidence['cpu_noise_valid'])
        self.assertAlmostEqual(evidence['avg_other_cores'], 15.73285419791858)
        self.assertEqual(evidence['remote_foreign_rustc'], 6)
        self.assertTrue(versus.sample_noisy(evidence, 0))

    def test_synth_passes_record_identity_before_marking(self):
        synth, _ = versus.suite._synth()
        self.write()
        plan = SimpleNamespace(snap='fixture', cwd='.', point=SimpleNamespace(i=0), expect_fail=False)

        def counted(argv, cwd, env, log):
            log.write_text('')
            return {'exit': 0, 'wall': 1., 'cpu_noise_valid': True, 'avg_other_cores': 0.}

        with patch.object(synth.gen, 'snapshot_root', return_value=self.home), \
                patch.object(versus, 'WORK', self.home), \
                patch.object(versus, 'stack_env', return_value={}), \
                patch.object(versus, 'run_counted', side_effect=counted), \
                patch.object(versus.suite, 'new_id', return_value='fixture'), \
                patch.object(versus.suite, 'foreign_load', return_value=(0, 0)):
            row = synth._run(versus.REMOTE, plan, ['test'], 'run')
        self.assertTrue(row['cpu_noise_valid'])
        self.assertFalse(row['noisy'])
        self.assertNotIn('fell_back_local', row)


if __name__ == '__main__':
    unittest.main()
