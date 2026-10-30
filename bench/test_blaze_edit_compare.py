"""Mock-only tests. Never compile or start Blaze daemons."""
import argparse
import importlib.util
import json
import os
from pathlib import Path
import tempfile
import unittest
from unittest.mock import patch, MagicMock

spec = importlib.util.spec_from_file_location('blaze_edit_compare', Path(__file__).with_name('blaze_edit_compare.py'))
b = importlib.util.module_from_spec(spec)
spec.loader.exec_module(b)
ORIGINAL = b'// private\r\nfn ago(t: f64) -> String {\n    format!("{t}")\n}\n'


class HarnessTests(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory()
        self.addCleanup(self.temp.cleanup)
        root = Path(self.temp.name)
        source = root / 'source'
        (source / 'src').mkdir(parents=True)
        (source / 'src/runs.rs').write_bytes(ORIGINAL)
        self.args = argparse.Namespace(source=source, output=root / 'out', samples=3,
                                       command='check', rustc=root / 'tc/bin/rustc',
                                       cargo=root / 'cargo', justrust=root / 'justrust', blaze=root / 'blaze')
        self.pinned = source / 'src/runs.rs'

    def orchestration(self, fail=None, interrupt=None, mutate=False, gate_accepts=False):
        calls = []
        def fake(args, stack, phase, replicate, env):
            calls.append((stack, phase, replicate, self.pinned.read_bytes()))
            if interrupt == (stack, phase, replicate):
                raise KeyboardInterrupt()
            if phase == 'type-error':
                (args.output / f'type-error-000-{stack}.stdout').write_bytes(b'')
                (args.output / f'type-error-000-{stack}.stderr').write_bytes(b'error[E0308]: mismatched types: blaze_edit_compare_type_error')
            return {'stack': stack, 'phase': phase, 'replicate': replicate,
                    'status': 101 if fail == (stack, phase, replicate) or (phase == 'type-error' and not gate_accepts) else 0,
                    'elapsed_seconds': float(replicate + 1)}
        with patch.object(b, 'identities', side_effect=[{'sha': 'before'}, {'sha': 'after' if mutate else 'before'}]), patch.object(b, 'environment', return_value={}), patch.object(b, 'run_command', side_effect=fake):
            result = b.execute(self.args)
        return result, calls

    def test_edit_is_exact_and_unique(self):
        edited = b.edit_bytes(ORIGINAL, 2)
        self.assertEqual(edited.replace(b'\n    std::hint::black_box(3u64);', b''), ORIGINAL)
        self.assertNotEqual(edited, b.edit_bytes(ORIGINAL, 1))

    def test_reject_missing_or_ambiguous_anchor(self):
        for original in (b'fn other() {}', ORIGINAL * 2):
            with self.assertRaises(ValueError):
                b.edit_bytes(original, 0)

    def test_rotation_and_identical_source(self):
        status, calls = self.orchestration()
        self.assertEqual(status, 0)
        edits = [c for c in calls if c[1] == 'edit']
        self.assertEqual([c[0] for c in edits], ['owned-off', 'owned-on', 'blaze', 'owned-on', 'blaze', 'owned-off', 'blaze', 'owned-off', 'owned-on'])
        for replicate in range(3):
            self.assertEqual(len({c[3] for c in edits if c[2] == replicate}), 1)
        self.assertEqual(self.pinned.read_bytes(), ORIGINAL)

    def test_failed_preparation_blocks_only_stack(self):
        status, calls = self.orchestration(fail=('blaze', 'prepare', 0))
        self.assertEqual(status, 1)
        self.assertFalse(any(c[0] == 'blaze' and c[1] == 'edit' for c in calls))
        summary = json.loads((self.args.output / 'summary.json').read_text())
        self.assertNotIn('blaze', summary['medians_seconds'])
        self.assertEqual(len(summary['medians_seconds']), 2)

    def test_failed_sample_never_ranked(self):
        status, _ = self.orchestration(fail=('owned-off', 'edit', 1))
        self.assertEqual(status, 1)
        summary = json.loads((self.args.output / 'summary.json').read_text())
        self.assertNotIn('owned-off', summary['medians_seconds'])
        self.assertEqual(summary['descriptive_ratios_owned_off_over'], {})

    def test_identity_mutation_rejects_every_stack(self):
        status, _ = self.orchestration(mutate=True)
        self.assertEqual(status, 1)
        self.assertEqual(json.loads((self.args.output / 'summary.json').read_text())['medians_seconds'], {})

    def test_interrupt_restores_source_and_keeps_partial(self):
        with self.assertRaises(KeyboardInterrupt):
            self.orchestration(interrupt=('owned-on', 'edit', 0))
        self.assertEqual(self.pinned.read_bytes(), ORIGINAL)
        self.assertEqual(len((self.args.output / 'measurements.jsonl').read_text().splitlines()), 1)
        self.assertTrue((self.args.output / 'summary.json').exists())

    def test_existing_output_rejected(self):
        self.args.output.mkdir()
        with self.assertRaises(FileExistsError):
            b.execute(self.args)
        self.assertEqual(self.pinned.read_bytes(), ORIGINAL)

    def test_environment_policy(self):
        installed = Path(self.temp.name) / 'installed'
        (installed / 'toolchains').mkdir(parents=True)
        with patch.dict(os.environ, {'BLAZE_HOME': str(installed), 'RUSTFLAGS': '-Zself-profile', 'RUSTC': 'bad', 'JUSTRUST_REMOTE': 'force'}):
            off = b.environment(self.args, 'owned-off')
            on = b.environment(self.args, 'owned-on')
            blaze = b.environment(self.args, 'blaze')
        for flag in b.FLAGS:
            self.assertIn(f'-Z{flag}=no', off['RUSTFLAGS'])
            self.assertIn(f'-Z{flag}=yes', on['RUSTFLAGS'])
        self.assertNotIn('RUSTFLAGS', blaze)
        self.assertNotIn('RUSTC', blaze)
        self.assertEqual(off['JUSTRUST_REMOTE'], 'off')
        self.assertEqual(off['JUSTRUST_DEPCACHE'], '0')
        self.assertEqual(off['JUSTRUST_BUILDD'], '0')
        self.assertEqual(off['JUSTRUST_REAL_CARGO'], str(self.args.cargo))
        self.assertEqual(off['JUSTRUST_LOCALCACHE'], '0')
        self.assertEqual(off['JUSTRUST_LOCALCACHE_TARGET_BOUND'], '0')
        self.assertEqual(off['JUSTRUST_PREBUILT'], 'off')
        self.assertTrue(off['JUSTRUST_HOME'].startswith(off['HOME']))
        self.assertTrue(blaze['BLAZE_CACHE'].startswith(blaze['XDG_CACHE_HOME']))
        self.assertEqual(off['CARGO_HOME'], blaze['CARGO_HOME'])
        self.assertNotEqual(off['HOME'], blaze['HOME'])
        self.assertEqual((Path(blaze['BLAZE_HOME']) / 'toolchains').resolve(), installed / 'toolchains')

    def test_run_logs_and_unknown_contamination(self):
        self.args.output.mkdir()
        process = MagicMock()
        process.__enter__.return_value = process
        process.wait.return_value = 101
        with patch.object(b.subprocess, 'Popen', return_value=process) as run:
            result = b.run_command(self.args, 'blaze', 'prepare', 0, {'API_TOKEN': 'NEVER_LOG', 'HOME': '/isolated'})
        self.assertEqual(result['status'], 101)
        self.assertEqual(result['noise']['contamination'], 'unknown')
        self.assertEqual(run.call_args.args[0], [str(self.args.blaze), 'check', '--lib'])
        self.assertTrue(run.call_args.kwargs['start_new_session'])
        process.wait.assert_called_once_with(timeout=360)
        self.assertEqual(result['environment'], {'HOME': '/isolated'})
        self.assertNotIn('NEVER_LOG', (self.args.output / 'prepare-000-blaze.json').read_text())
        for suffix in ('stdout', 'stderr', 'json'):
            self.assertTrue((self.args.output / ('prepare-000-blaze.' + suffix)).exists())

    def test_spawn_failure_recorded(self):
        self.args.output.mkdir()
        with patch.object(b.subprocess, 'Popen', side_effect=FileNotFoundError('missing')):
            result = b.run_command(self.args, 'owned-on', 'prepare', 0, {})
        self.assertIsNone(result['status'])
        self.assertIn('missing', result['error'])

    def test_type_error_false_success_blocks_every_stack(self):
        status, calls = self.orchestration(gate_accepts=True)
        self.assertEqual(status, 1)
        self.assertFalse(any(c[1] == 'edit' for c in calls))
        self.assertEqual(sum(c[1] == 'recovery' for c in calls), 3)

    def test_failed_recovery_blocks_stack(self):
        status, calls = self.orchestration(fail=('blaze', 'recovery', 0))
        self.assertEqual(status, 1)
        self.assertFalse(any(c[0] == 'blaze' and c[1] == 'edit' for c in calls))
        self.assertEqual(self.pinned.read_bytes(), ORIGINAL)

    def test_type_error_requires_real_diagnostic_not_timeout(self):
        self.args.output.mkdir()
        prefix = self.args.output / 'type-error-000-blaze'
        prefix.with_suffix('.stdout').write_bytes(b'')
        prefix.with_suffix('.stderr').write_bytes(b'network unavailable')
        self.assertFalse(b.type_error_detected(self.args, 'blaze', {'status': 1}))
        prefix.with_suffix('.stderr').write_bytes(b'E0308 blaze_edit_compare_type_error')
        self.assertTrue(b.type_error_detected(self.args, 'blaze', {'status': 101}))
        self.assertFalse(b.type_error_detected(self.args, 'blaze', {'status': 101, 'timed_out': True}))

    def test_timeout_kills_group_and_preserves_result(self):
        self.args.output.mkdir()
        process = MagicMock(pid=12345)
        process.__enter__.return_value = process
        process.wait.side_effect = [b.subprocess.TimeoutExpired(['blaze'], 360), -9]
        with patch.object(b.subprocess, 'Popen', return_value=process), patch.object(b.os, 'killpg') as kill:
            row = b.run_command(self.args, 'blaze', 'prepare', 0, {})
        kill.assert_called_once_with(12345, b.signal.SIGKILL)
        self.assertTrue(row['timed_out'])
        self.assertEqual(row['status'], -9)

    def test_type_error_source_from_exact_pristine(self):
        data = b.type_error_bytes(ORIGINAL)
        self.assertEqual(data.replace(b'\n    let _: u64 = "blaze_edit_compare_type_error";', b''), ORIGINAL)

    def test_timeout_with_racing_zero_exit_is_not_success(self):
        self.assertFalse(b.succeeded({'status': 0, 'timed_out': True}))
        self.assertFalse(b.succeeded({'status': 0, 'error': 'spawn failed'}))
        self.assertTrue(b.succeeded({'status': 0}))

    def test_identity_hash_covers_driver(self):
        self.args.rustc.parent.mkdir(parents=True)
        driver = self.args.rustc.parent.parent / 'lib/librustc_driver-test.so'
        driver.parent.mkdir()
        driver.write_bytes(b'driver')
        for name in ('rustc', 'cargo', 'justrust', 'blaze'):
            getattr(self.args, name).write_bytes(name.encode())
        before = b.identities(self.args)
        driver.write_bytes(b'changed')
        self.assertNotEqual(before, b.identities(self.args))


if __name__ == '__main__':
    unittest.main()
