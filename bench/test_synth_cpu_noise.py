"""Synthetic noise must measure external CPU, not the preceding prebuild's load."""
import contextlib
import io
import json
from pathlib import Path
import tempfile
from types import SimpleNamespace
import unittest
from unittest.mock import patch

import suite
import versus
from test_synth_reporting import fixture

synth, _ = suite._synth()


class SyntheticCpuNoiseTests(unittest.TestCase):
    def sample(self, other=0.0, valid=True, foreign=0, remote=False):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            plan = SimpleNamespace(snap='fixture', cwd='.', point=SimpleNamespace(i=0), expect_fail=False)
            stack = versus.Stack('fixture', 'fixture', False,
                                 {'JUSTRUST_REMOTE': 'force'} if remote else {})
            def counted(argv, cwd, env, log):
                log.write_text('test result: ok. 1 passed; 0 failed\n')
                return {'exit': 0, 'wall': 1.0, 'cpu_noise_valid': valid,
                        'avg_other_cores': other}
            with patch.object(synth.gen, 'snapshot_root', return_value=root), \
                    patch.object(versus, 'WORK', root), \
                    patch.object(versus, 'stack_env', return_value={}), \
                    patch.object(versus, 'run_counted', side_effect=counted), \
                    patch.object(suite, 'foreign_load', return_value=(27.47, foreign)):
                return synth._run(stack, plan, ['test'], 'run')

    def test_prior_prebuild_load_does_not_mark_actual_quiet_sample(self):
        row = self.sample()
        self.assertEqual(row['load_before'], 27.47)
        self.assertFalse(row['noisy'])

    def test_external_cpu_foreign_compiler_and_unknown_still_noisy(self):
        for other, valid, foreign in ((4.1, True, 0), (0, True, 1),
                                      (None, False, 0), (float('nan'), True, 0)):
            with self.subTest(other=other, valid=valid, foreign=foreign):
                self.assertTrue(self.sample(other, valid, foreign)['noisy'])
        self.assertFalse(self.sample(4.0)['noisy'])

    def test_remote_client_coverage_cannot_be_reported_quiet(self):
        row = self.sample(remote=True)
        self.assertTrue(row['noisy'])
        self.assertFalse(row['cpu_noise_valid'])
        self.assertEqual(row['cpu_noise_scope'], 'client-host-only')

    def test_new_policy_and_threshold_changes_cannot_mix_with_old_results(self):
        base = {'justrust_rev': 'fixture', 'snapshots': {'fixture': 'pin'},
                'host': 'fixture', 'cpu': 'fixture', 'ncpu': 16}
        for other in (None, {**suite.NOISE_POLICY, 'max_other_cores': 8},
                      {**suite.NOISE_POLICY, 'counted': 'loadavg-v0'}):
            with self.subTest(other=other), tempfile.TemporaryDirectory() as directory:
                paths = [Path(directory) / name for name in ('a.json', 'b.json')]
                for path, policy in zip(paths, (suite.NOISE_POLICY, other)):
                    env = dict(base)
                    if policy is not None:
                        env['noise_policy'] = policy
                    path.write_text(json.dumps({'env': env, 'synth': fixture()}))
                args = SimpleNamespace(a=str(paths[0]), b=str(paths[1]), require_valid=True)
                output = io.StringIO()
                with contextlib.redirect_stdout(output), self.assertRaises(SystemExit) as raised:
                    suite.cmd_compare(args)
                self.assertEqual(raised.exception.code, 2)
                self.assertIn('different noise_policy', output.getvalue())


if __name__ == '__main__':
    unittest.main()
