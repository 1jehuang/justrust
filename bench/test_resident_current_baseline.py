import argparse
from pathlib import Path
import tempfile
import unittest
from unittest.mock import patch

import resident_current_baseline as baseline


class BaselineTests(unittest.TestCase):
    def test_clean_environment_and_profiles(self):
        with patch.dict('os.environ', {'PATH': '/bin', 'HOME': '/home/test',
                'JUSTRUST_TOOLCHAIN': 'bad', 'RUSTC': 'bad', 'RA_JIT_HOME': 'bad',
                'CARGO_TARGET_DIR': '/wrong', 'JR_ENGINE_HOME': '/wrong'}, clear=True):
            env = baseline.environment(Path('/private/result'))
        self.assertEqual(env['CARGO_TARGET_DIR'], '/private/result/target')
        self.assertEqual(env['JUSTRUST_REMOTE'], 'off')
        self.assertIn('controlled same-host', env['JUSTRUST_LOCAL_REASON'])
        self.assertEqual(env['JUSTRUST_SLOTS'], '0')
        self.assertEqual(env['HOME'], '/home/test')
        self.assertNotIn('RUSTC', env)
        self.assertNotIn('JUSTRUST_TOOLCHAIN', env)
        self.assertNotIn('RA_JIT_HOME', env)
        self.assertNotIn('JR_ENGINE_HOME', env)
        for key, value in baseline.runtime.PROFILES.items():
            self.assertEqual(env[key], value)

    def test_command_uses_justrust_and_exact_filtered_contract(self):
        argv = baseline.command(Path('/candidate/justrust'))
        self.assertEqual(argv[:2], ['/candidate/justrust', 'test'])
        self.assertTrue(argv[2].startswith('--local='))
        self.assertEqual(argv[3:], baseline.shared.FILTER)

    def test_summary_refuses_incomplete_or_failed_samples(self):
        rows = [dict(phase=phase, accepted=True, elapsed_seconds=float(i + 1), failures=[])
                for phase in baseline.tier.PHASES for i in range(5)]
        summary = baseline.summarize(rows, 5, True)
        self.assertTrue(all(r['accepted'] for r in summary.values()))
        self.assertTrue(all(r['median_seconds'] == 3 for r in summary.values()))
        self.assertIsNone(baseline.summarize(rows, 5, False)['edit']['median_seconds'])
        rows[0].update(accepted=False, failures=['fresh marker missing'])
        self.assertFalse(baseline.summarize(rows, 5, True)['edit']['accepted'])
        self.assertIsNone(baseline.summarize(rows[1:], 5, True)['edit']['median_seconds'])

    def test_arguments_reject_unsafe_inputs_before_compiling(self):
        args = argparse.Namespace(samples=4, timeout=30, output=Path('/a'), source=Path('/b'))
        with self.assertRaises(ValueError):
            baseline.execute(args)
        args.samples = 5
        args.timeout = float('nan')
        with self.assertRaises(ValueError):
            baseline.execute(args)
        with tempfile.TemporaryDirectory() as root:
            args.timeout = 30
            args.source = Path(root)
            args.output = Path(root) / 'inside'
            with self.assertRaises(ValueError):
                baseline.execute(args)

    def test_exact_resident_edit_recipe_is_reused(self):
        source = b'fn shares_overlapping_time() {\n}\n'
        edited = baseline.tier.contents(source, 'edit', 3, 'invocation')
        self.assertIn(b'JR_RUNTIME_EXECUTED_4', edited)
        self.assertIn(b'JR_MATRIX_NONCE', edited)
        self.assertIn(b'black_box(4u64)', edited)
        error = baseline.tier.contents(source, 'type-error', 3, 'invocation')
        self.assertIn(b'JR_MATRIX_TYPE_invocation', error)


if __name__ == '__main__':
    unittest.main()
