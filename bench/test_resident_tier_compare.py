#!/usr/bin/env python3
"""Synthetic harness tests only. Never invoke an engine, compiler or Rust build."""
import argparse
from contextlib import ExitStack
import json
from pathlib import Path
import tempfile
import unittest
from unittest.mock import patch

import resident_tier_compare as tier


def parsed(failing=False):
    return dict(tests=[(name, 'FAILED' if failing and name == tier.shared.EDIT_TEST else 'ok')
                       for name in tier.shared.NAMES], passed=3 if failing else 4,
                failed=int(failing), ignored=0, filtered=144)


def test_output(failing=False):
    value = parsed(failing)
    text = '\n'.join(f'test {name} ... {state}' for name, state in value['tests'])
    return text + (f"\ntest result: {'FAILED' if failing else 'ok'}. {value['passed']} passed; "
                   f"{value['failed']} failed; 0 ignored; 0 measured; 144 filtered out\n")


class MockNative:
    def __init__(self, stale=False, fallback=False, crash=False):
        self.states = {}
        self.next_pid = 1000
        self.stale = stale
        self.fallback = fallback
        self.crash = crash
        self.calls = []

    def run(self, command, cwd, env, prefix, timeout):
        self.calls.append((list(command), dict(env)))
        state = self.states.setdefault(env['JR_ENGINE_HOME'], dict(pid=None, workers=[], served=0))
        operation = command[1]
        log = Path(env['JR_ENGINE_HOME']) / 'mock-worker.log'
        status = 0
        if operation == '--version':
            text = 'jr-engine experimental synthetic-never-executed\n'
        elif operation == 'stop':
            state.update(pid=None, workers=[])
            text = 'daemon: stopped\n'
        elif operation == 'status':
            text = 'daemon: not running\n' if not state['pid'] else (
                f"daemon: pid {state['pid']}, version mock, exits after 3600s idle\n"
                f"  worker: pid {state['workers'][0]}, {cwd}, holds 1 MB, log {log}\n")
            text += f"project (test, key): {state['served']} builds by the daemon, 0 by cargo\n"
        else:
            if self.crash and prefix.name.endswith('-edit'):
                raise RuntimeError('synthetic interruption')
            self.assert_controls(env)
            if not state['pid']:
                log.write_text('worker loading\n')
                self.next_pid += 2
                state.update(pid=self.next_pid, workers=[self.next_pid + 1])
            if not self.fallback:
                state['served'] += 1
                with log.open('a') as stream:
                    stream.write('edit-to-binary: 10ms\n')
            data = (cwd / 'src/summary.rs').read_text()
            nonce = env['JR_MATRIX_NONCE']
            if 'JR_MATRIX_TYPE_' in data:
                status, text = 101, 'error[E0308]: JR_MATRIX_TYPE_' + nonce
            else:
                failing = 'JR_MATRIX_FAILURE_' in data
                status = 101 if failing else 0
                text = test_output(failing)
                text += 'JR_MATRIX_RUNTIME_' + ('stale' if self.stale else nonce) + '\n'
                if failing:
                    text += 'JR_MATRIX_FAILURE_' + nonce + '\n'
                else:
                    import re
                    marker = re.search(r'JR_RUNTIME_EXECUTED_\d+', data).group()
                    text += marker + '\n'
        prefix.with_suffix('.stdout').write_text(text)
        prefix.with_suffix('.stderr').write_text('')
        return dict(command=command, status=status, timed_out=False, error=None,
                    elapsed_seconds=0.02 if operation == 'stop' else 0.1)

    @staticmethod
    def assert_controls(env):
        assert env['RA_JIT_MEMORY_MAX'] == 'off'
        assert env['RA_JIT_SPARE'] == '0'
        assert env['RA_JIT_IDLE_SECS'] == '3600'


class HarnessTests(unittest.TestCase):
    def test_status_parse_and_not_running(self):
        value = tier.parse_status('daemon: pid 12, version abc\n'
            '  worker: pid 15, /x, log /y\n/x: 2 builds by the daemon, 1 by cargo\n')
        self.assertEqual(value, dict(daemon=12, workers=[15], served=2, counters_available=True,
            worker_details=[dict(pid=15, project='/x', log='/y')]))
        self.assertIsNone(tier.parse_status('daemon: not running')['daemon'])

    def test_lifecycle_strictly_distinguishes_modes(self):
        before = dict(daemon=1, workers=[2])
        after = dict(daemon=3, workers=[4])
        stopped = dict(daemon=None, workers=[])
        self.assertEqual(tier.lifecycle_errors(tier.MODES[0], before, None, before), [])
        self.assertTrue(tier.lifecycle_errors(tier.MODES[0], before, None, after))
        self.assertEqual(tier.lifecycle_errors(tier.MODES[1], before, stopped, after), [])
        self.assertTrue(tier.lifecycle_errors(tier.MODES[1], before, before, after))
        self.assertTrue(tier.lifecycle_errors(tier.MODES[1], before, stopped, before))

    def test_native_status_without_served_counters(self):
        status = tier.parse_status('daemon: pid 421, version native, exits never, no limit\n'
            '  worker: pid 422, /source63, holds 832 MB, log /private/workers/abc/log\n'
            '    /source63: warm\nstore: 12 MB, no limit, /private/store\n')
        self.assertFalse(status['counters_available'])
        self.assertEqual(status['worker_details'], [dict(pid=422, project='/source63',
            log='/private/workers/abc/log')])

    def test_worker_log_requires_fresh_non_refused_edit(self):
        with tempfile.TemporaryDirectory() as directory:
            home = Path(directory)
            log = home / 'log'
            log.write_text('edit-to-binary: historical\n')
            status = dict(worker_details=[dict(pid=42, project='/source', log=str(log))])
            before = tier.worker_log_snapshot(status, home)
            stale = tier.worker_log_evidence(before, status, home, home / 'stale')
            self.assertTrue(stale['failures'])
            with log.open('a') as stream:
                stream.write('edit-to-binary: 10ms\n')
            fresh = tier.worker_log_evidence(before, status, home, home / 'fresh')
            self.assertEqual(fresh['failures'], [])
            self.assertEqual(fresh['classification'], 'native-worker-edit')
            before = tier.worker_log_snapshot(status, home)
            with log.open('a') as stream:
                stream.write('refused: unsupported\nedit-to-binary: 1ms\n')
            refused = tier.worker_log_evidence(before, status, home, home / 'refused')
            self.assertTrue(refused['failures'])
            error = tier.worker_log_evidence(before, status, home, home / 'type-error', phase='type-error')
            self.assertEqual(error['failures'], [])

    def test_memory_present_and_missing(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            proc = root / '12'
            proc.mkdir()
            (proc / 'stat').write_text('12 (name with space) ' + ' '.join(['0'] * 19 + ['123']))
            (proc / 'smaps_rollup').write_text('Rss: 32 kB\nPss: 21 kB\n')
            row = tier.memory_snapshot([12], root)[0]
            self.assertEqual((row['rss_kib'], row['pss_kib'], row['start_ticks']), (32, 21, 123))
            missing = tier.memory_snapshot([42], root)[0]
            self.assertIsNone(missing['rss_kib'])
            self.assertIsNone(missing['pss_kib'])
            self.assertTrue(missing['errors'])

    def test_edit_recipes_and_assert_probe_order(self):
        original = tier.matrix.ANCHOR + b' assert!(true); }'
        for phase in tier.PHASES:
            data = tier.contents(original, phase, 0, 'nonce')
            self.assertEqual(data.count(tier.matrix.PROBE), 1)
            if phase == 'assert-failure':
                self.assertLess(data.index(tier.matrix.PROBE), data.index(b'JR_MATRIX_FAILURE_nonce'))
        self.assertEqual(tier.contents(original, 'edit', 2, 'a'),
                         tier.contents(original, 'edit', 2, 'b'))

    def test_oracle_rejects_stale_nonce_and_bad_diagnostics(self):
        row = dict(status=0, timed_out=False, error=None, raw_error=None, parsed=parsed())
        text = 'JR_MATRIX_RUNTIME_nonce\nJR_RUNTIME_EXECUTED_1\n'
        self.assertEqual(tier.correctness(row, text, 'edit', 0, 'nonce', parsed()), [])
        self.assertTrue(tier.correctness(row, text, 'edit', 0, 'other', parsed()))
        row.update(status=101, parsed=None)
        self.assertEqual(tier.correctness(row, 'E0308 JR_MATRIX_TYPE_nonce', 'type-error', 0, 'nonce', parsed()), [])
        self.assertTrue(tier.correctness(row, 'E0308 old', 'type-error', 0, 'nonce', parsed()))

    def run_fixture(self, mock):
        with tempfile.TemporaryDirectory(prefix='rt-', dir='/tmp') as directory:
            root = Path(directory)
            source = root / 'source'
            (source / 'src').mkdir(parents=True)
            original = tier.matrix.ANCHOR + b' assert!(true); }\n'
            (source / 'src/summary.rs').write_bytes(original)
            engine = root / 'native'
            engine.write_bytes(b'\x7fELFfake-never-executed')
            engine.chmod(0o700)
            tc = root / tier.shared.TOOLCHAIN_NAME
            tc.mkdir()
            args = argparse.Namespace(engine=engine, source=source, output=root / 'out',
                toolchain=tc, proc_macro_server=None, samples=5, timeout=10)
            with ExitStack() as stack:
                stack.enter_context(patch.object(tier.shared, 'pinned_source'))
                stack.enter_context(patch.object(tier.shared, 'toolchain_identity', return_value={'mock': 'fixed'}))
                stack.enter_context(patch.object(tier.runtime, 'run_process', side_effect=mock.run))
                stack.enter_context(patch.object(tier, 'memory_snapshot', return_value=[]))
                try:
                    result = tier.execute(args)
                except (RuntimeError, ValueError):
                    result = 'raised'
            self.assertEqual((source / 'src/summary.rs').read_bytes(), original)
            self.assertTrue(all(not state['pid'] for state in mock.states.values()))
            return result, {name: json.loads((args.output / name).read_text())
                for name in ('metadata.json', 'records.json', 'receipts.json', 'summary.json')}

    def test_mock_matched_complete_and_receipts(self):
        native = MockNative()
        result, files = self.run_fixture(native)
        self.assertEqual(result, 0)
        self.assertEqual(files['metadata.json']['status'], 'complete')
        rows = files['records.json']
        self.assertEqual(len(rows), 50)
        for phase in tier.PHASES:
            for rep in range(5):
                pair = [r for r in rows if r['phase'] == phase and r['replicate'] == rep]
                self.assertEqual(pair[0]['source_sha256'], pair[1]['source_sha256'])
                self.assertEqual(pair[0]['nonce'], pair[1]['nonce'])
                self.assertEqual(pair[0]['mode'], tier.MODES[rep % 2])
        self.assertAlmostEqual(files['summary.json']['states']['edit'][tier.MODES[1]]['median_total_seconds'], 0.12)
        self.assertEqual(files['summary.json']['tiers']['full-heap-restore']['status'], 'unsupported')
        self.assertTrue(files['metadata.json']['raw_receipts_unchanged'])
        self.assertTrue(all('raw' in r for r in files['receipts.json']))

    def test_fallback_never_admitted(self):
        result, files = self.run_fixture(MockNative(fallback=True))
        self.assertEqual(result, 1)
        self.assertTrue(files['summary.json']['states']['edit'][tier.MODES[0]]['excluded'])
        self.assertIsNone(files['summary.json']['states']['edit'][tier.MODES[0]]['median_total_seconds'])

    def test_stale_prepare_is_not_seeded(self):
        result, files = self.run_fixture(MockNative(stale=True))
        self.assertEqual(result, 'raised')
        self.assertEqual(files['records.json'], [])
        self.assertIn('private cold prepare failed', files['metadata.json']['error'])

    def test_interruption_restores_and_retains_failure(self):
        result, files = self.run_fixture(MockNative(crash=True))
        self.assertEqual(result, 'raised')
        self.assertIn('synthetic interruption', files['metadata.json']['error'])
        self.assertEqual(files['summary.json']['status'], 'failed/incomplete')

    def test_summary_never_admits_incomplete_or_failed_identity(self):
        row = dict(mode=tier.MODES[0], phase='edit', replicate=0, accepted=True,
                   elapsed_seconds=1, total_seconds=1)
        self.assertTrue(tier.summarize([row], 5, True)['edit'][tier.MODES[0]]['excluded'])
        self.assertTrue(tier.summarize([row], 1, False)['edit'][tier.MODES[0]]['excluded'])


if __name__ == '__main__':
    unittest.main()
