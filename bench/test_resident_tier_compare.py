#!/usr/bin/env python3
"""Synthetic harness tests only. Never invoke an engine, compiler or Rust build."""
import argparse
from contextlib import ExitStack
import json
import re
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


class MockBudgetNative:
    def __init__(self, evict=True):
        self.states = {}
        self.next_pid = 2000
        self.evict = evict

    def identity(self, pid):
        live = any(pid == state['daemon'] or pid in state['projects'].values()
                   for state in self.states.values())
        return dict(pid=pid, start_ticks=pid * 10, state='S') if live else None

    def run(self, command, cwd, env, prefix, timeout):
        state = self.states.setdefault(env['JR_ENGINE_HOME'], dict(daemon=None, projects={}))
        operation = command[1]
        status = 0
        if operation == '--version':
            text = 'jr-engine synthetic-budget\n'
        elif operation == 'stop':
            state.update(daemon=None, projects={})
            text = 'daemon: stopped\n'
        elif operation == 'status':
            text = f"daemon: pid {state['daemon']}, version mock, exits never, no limit\n" if state['daemon'] else 'daemon: not running\n'
            for project, pid in state['projects'].items():
                log = Path(env['JR_ENGINE_HOME']) / (project.name + '.log')
                text += f'  worker: pid {pid}, {project}, holds 100 MB, log {log}\n    {project}: ready\n'
        else:
            assert env['RA_JIT_SPARE'] == '0'
            assert env['RA_JIT_MEMORY_MAX'] in ('off', '1M')
            if not state['daemon']:
                self.next_pid += 1
                state['daemon'] = self.next_pid
                (Path(env['JR_ENGINE_HOME']) / 'resident-v1/runtime/daemon.log').write_text('daemon starts\n')
            cold = cwd not in state['projects']
            log = Path(env['JR_ENGINE_HOME']) / (cwd.name + '.log')
            if cold:
                self.next_pid += 1
                state['projects'][cwd] = self.next_pid
                log.write_text('native worker loading\n')
            else:
                with log.open('a') as stream:
                    stream.write('edit-to-binary: 10ms\n')
            if self.evict and env['RA_JIT_MEMORY_MAX'] != 'off':
                with (Path(env['JR_ENGINE_HOME']) / 'resident-v1/runtime/daemon.log').open('a') as stream:
                    for project in state['projects']:
                        if project != cwd:
                            stream.write(f'worker abc of {project} stopped: the workers hold 200 MB, '
                                         'over the 1 MB of BLAZE_MEMORY_MAX, and it was asked longest ago\n')
                state['projects'] = {cwd: state['projects'][cwd]}
            data = (cwd / 'src/summary.rs').read_text()
            nonce = env['JR_MATRIX_NONCE']
            text = "blaze: worker not loaded; `fixture` is cargo's to build\n" if cold else ''
            if 'JR_MATRIX_TYPE_' in data:
                status = 101
                text += 'error[E0308]: JR_MATRIX_TYPE_' + nonce
            else:
                failing = 'JR_MATRIX_FAILURE_' in data
                status = 101 if failing else 0
                text += test_output(failing) + 'JR_MATRIX_RUNTIME_' + nonce + '\n'
                text += ('JR_MATRIX_FAILURE_' + nonce if failing else
                         re.search(r'JR_RUNTIME_EXECUTED_\d+', data).group()) + '\n'
        prefix.with_suffix('.stdout').write_text(text)
        prefix.with_suffix('.stderr').write_text('')
        return dict(command=command, status=status, timed_out=False, error=None, elapsed_seconds=0.1)


class HarnessTests(unittest.TestCase):
    def test_status_parse_and_not_running(self):
        value = tier.parse_status('daemon: pid 12, version abc\n'
            '  worker: pid 15, /x, log /y\n/x: 2 builds by the daemon, 1 by cargo\n')
        self.assertEqual(value, dict(daemon=12, workers=[15], served=2, counters_available=True,
            worker_details=[dict(pid=15, project='/x', log='/y')], ready_projects=[]))
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

    def test_cold_reload_requires_complete_success_and_restart(self):
        with tempfile.TemporaryDirectory() as directory:
            home = Path(directory)
            log = home / 'log'
            text = ('cold: compiled 3990 of 3990 fns in 3.5s\n'
                    'stats: 10631 of 44576 functions compiled, 33945 reused\n'
                    'link: 227ms (exit status: 0)\nwatching 49 files\n')
            status = dict(worker_details=[dict(pid=43, project='/source', log=str(log))])
            log.write_text(text)
            result = tier.worker_log_evidence({}, status, home, home / 'cold', restart=True)
            self.assertEqual(result['classification'], 'native-worker-cold-reload')
            self.assertEqual(result['failures'], [])
            before = tier.worker_log_snapshot(status, home)
            with log.open('a') as stream:
                stream.write(text)
            self.assertTrue(tier.worker_log_evidence(before, status, home, home / 'hot')['failures'])
            for index, invalid in enumerate((text.replace('exit status: 0', 'exit status: 1'),
                                             text.replace('watching 49 files\n', ''),
                                             text + 'refused: unsupported\n')):
                log.write_text(invalid)
                self.assertTrue(tier.worker_log_evidence({}, status, home,
                    home / str(index), restart=True)['failures'])

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

    def test_native_eviction_log_accepts_checkout_trailing_separator(self):
        with tempfile.TemporaryDirectory() as directory:
            home = Path(directory)
            log = home / 'resident-v1/runtime/daemon.log'
            log.parent.mkdir(parents=True)
            log.write_text('started\n')
            before = tier.worker_log_snapshot(dict(worker_details=[dict(pid=1,
                project='/project/a', log=str(log))]), home)
            with log.open('a') as stream:
                stream.write('worker abc of /project/a/ stopped: the workers hold 4000 MB, '
                    'over the 1 MB of BLAZE_MEMORY_MAX, and it was asked longest ago\n')
            evidence = tier.budget_eviction_receipt(before, home, Path('/project/a'), home / 'probe')
            self.assertGreater(evidence['bytes'], 0)
            with self.assertRaises(ValueError):
                tier.budget_eviction_receipt(before, home, Path('/project/other'), home / 'wrong')

    def test_readiness_latency_is_separate_from_result_latency(self):
        rows = [dict(mode=mode, phase='hot-edit', replicate=rep, accepted=True,
                     elapsed_seconds=0.1, total_seconds=0.1,
                     ready_for_next_edit_seconds=0.5)
                for mode in tier.BUDGET_MODES for rep in range(5)]
        states = tier.summarize(rows, 5, True, modes=tier.BUDGET_MODES, phases=('hot-edit',))
        for state in states['hot-edit'].values():
            self.assertEqual(state['median_workload_seconds'], 0.1)
            self.assertEqual(state['median_ready_for_next_edit_seconds'], 0.5)
        rows.pop()
        self.assertTrue(tier.summarize(rows, 5, True, modes=tier.BUDGET_MODES,
            phases=('hot-edit',))['hot-edit']['bounded']['excluded'])

    def test_summary_never_admits_incomplete_or_failed_identity(self):
        row = dict(mode=tier.MODES[0], phase='edit', replicate=0, accepted=True,
                   elapsed_seconds=1, total_seconds=1)
        self.assertTrue(tier.summarize([row], 5, True)['edit'][tier.MODES[0]]['excluded'])
        self.assertTrue(tier.summarize([row], 1, False)['edit'][tier.MODES[0]]['excluded'])

    def run_budget(self, native):
        with tempfile.TemporaryDirectory(prefix='bt-', dir='/tmp') as directory:
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
            args = argparse.Namespace(engine=engine, source=source, output=root / 'o',
                toolchain=tc, proc_macro_server=None, samples=5, timeout=10,
                settle_timeout=0.02, memory_max='1M')
            with ExitStack() as stack:
                stack.enter_context(patch.object(tier.shared, 'pinned_source'))
                stack.enter_context(patch.object(tier.shared, 'toolchain_identity', return_value={'mock': 'fixed'}))
                stack.enter_context(patch.object(tier.runtime, 'run_process', side_effect=native.run))
                stack.enter_context(patch.object(tier, 'process_identity', side_effect=native.identity))
                stack.enter_context(patch.object(tier, 'memory_snapshot', return_value=[]))
                stack.enter_context(patch.object(tier.time, 'sleep'))
                try:
                    result = tier.execute_budget(args)
                except ValueError:
                    result = 'raised'
            self.assertEqual((source / 'src/summary.rs').read_bytes(), original)
            self.assertTrue(all(not state['daemon'] for state in native.states.values()))
            return result, {name: json.loads((args.output / name).read_text())
                for name in ('metadata.json', 'records.json', 'receipts.json', 'summary.json')}

    def test_budget_real_two_project_hot_and_evicted_paths(self):
        result, files = self.run_budget(MockBudgetNative())
        self.assertEqual(result, 0, files['metadata.json'].get('error'))
        rows = files['records.json']
        self.assertEqual(len(rows), 60)
        self.assertTrue(files['metadata.json']['copy_identities_restored'])
        self.assertTrue(files['metadata.json']['raw_receipts_unchanged'])
        for rep in range(5):
            for phase in tier.BUDGET_PHASES:
                pair = [r for r in rows if r['replicate'] == rep and r['phase'] == phase]
                self.assertEqual(pair[0]['source_sha256'], pair[1]['source_sha256'])
                self.assertEqual(pair[0]['nonce'], pair[1]['nonce'])
                self.assertTrue(all(r['accepted'] for r in pair))
            miss = next(r for r in rows if r['replicate'] == rep and r['phase'] == 'after-peer-edit' and r['mode'] == 'bounded')
            self.assertEqual(miss['native_evidence']['classification'], 'cargo-reload-with-native-worker-ready')
            self.assertIn('eviction_log', miss['transition'])
            self.assertNotEqual(miss['transition']['old_a_identity']['pid'],
                                tier.project_workers(miss['after'], Path(miss['cwd']))[0]['pid'])
            hot = next(r for r in rows if r['replicate'] == rep and r['phase'] == 'hot-edit' and r['mode'] == 'bounded')
            self.assertEqual(hot['native_evidence']['classification'], 'native-worker-edit')
        self.assertNotIn('weighted', files['summary.json'])

    def test_budget_missing_actual_eviction_fails_closed(self):
        result, files = self.run_budget(MockBudgetNative(evict=False))
        self.assertEqual(result, 'raised')
        self.assertIn('readiness/eviction timeout', files['metadata.json']['error'])
        self.assertEqual(files['summary.json']['status'], 'failed/incomplete')
        self.assertTrue(files['summary.json']['states']['after-peer-edit']['bounded']['excluded'])

    def test_budget_eviction_requires_fresh_budget_event(self):
        with tempfile.TemporaryDirectory() as directory:
            home = Path(directory)
            log = home / 'resident-v1/runtime/daemon.log'
            log.parent.mkdir(parents=True)
            log.write_text('worker old of /a stopped: over BLAZE_MEMORY_MAX, asked longest ago\n')
            before = tier.worker_log_snapshot(dict(worker_details=[dict(pid=12,
                project='/a', log=str(log))]), home)
            with log.open('a') as stream:
                stream.write('worker crashed unexpectedly\n')
            with self.assertRaisesRegex(ValueError, 'no fresh native budget eviction'):
                tier.budget_eviction_receipt(before, home, Path('/a'), home / 'crash')
            with log.open('a') as stream:
                stream.write('worker abc of /a stopped: over BLAZE_MEMORY_MAX, asked longest ago\n')
            receipt = tier.budget_eviction_receipt(before, home, Path('/a'), home / 'evicted')
            self.assertEqual(tier.runtime.base.digest(Path(receipt['path'])), receipt['sha256'])

    def test_process_identity_reuse_and_zombie_exit(self):
        old = dict(pid=4, start_ticks=10, state='S')
        for now, expected in ((old, False), (None, True),
                (dict(pid=4, start_ticks=11, state='S'), True),
                (dict(pid=4, start_ticks=10, state='Z'), True)):
            with patch.object(tier, 'process_identity', return_value=now):
                self.assertEqual(tier.process_gone(old), expected)


if __name__ == '__main__':
    unittest.main()
