#!/usr/bin/env python3
"""Pure mock tests. Never launch a compiler, consumer, or native build."""
import copy
import hashlib
import itertools
import json
import os
from pathlib import Path
from types import SimpleNamespace
import tempfile
import unittest
from unittest import mock

import native_disk_snapshot as subject


def observation(status=0, pid=100):
    return dict(status=status, pid=pid, timed_out=False, elapsed_seconds=0.125,
                stdout_b64='', stderr_b64='' if status == 0 else 'ZXJyb3IK')


class Model:
    """Deliberately small process/cache model, not native acceptance evidence."""
    def __init__(self, missing_hits=False, malformed=False):
        self.pids = itertools.count(1000)
        self.nonces = itertools.count()
        self.calls = []
        self.missing_hits, self.malformed = missing_hits, malformed
        self.workers = []

    def compile(self, root, pid):
        source = (root / 'fixture.rs').read_text()
        failed = '->bool' in source or '& 255 ) }' in source
        if not failed:
            (root / 'libast_fixture.rlib').write_bytes(source.encode())
        return observation(int(failed), pid)

    def run(self, command, timeout, cwd, env):
        self.calls.append((list(command), dict(env)))
        pid = next(self.pids)
        result = self.compile(cwd, pid)
        result['command'] = list(map(str, command))
        if subject.DISK_ENV[0] in env:
            source = (cwd / 'fixture.rs').read_bytes()
            cache = Path(env[subject.DISK_ENV[0]])
            entry = cache / hashlib.sha256(source).hexdigest()
            stats = Path(env[subject.DISK_ENV[1]])
            if entry.exists() and entry.read_bytes() == source:
                events = ['miss'] if self.missing_hits else ['hit']
            else:
                events = ['reject', 'miss'] if entry.exists() else ['miss']
                if '& 255 ) }' not in source.decode():
                    entry.write_bytes(source)
                    events.append('store')
            with stats.open('a') as stream:
                if self.malformed:
                    stream.write('not JSON\n')
                else:
                    for event in events:
                        stream.write(json.dumps(dict(event=event, pid=pid)) + '\n')
        return result

    def consumer(self, compiler, root, timeout, a, b):
        nonce = f'nonce-{next(self.nonces)}'
        expected = ''.join(f'{nonce}:{x}:{(x ^ a) & 255}:{(x ^ b) & 255}\n'
                           for x in subject.ast.EDGES)
        execution = observation()
        execution['stdout_b64'] = subject.shared.encoded(expected.encode())
        return dict(nonce=nonce, compile=observation(), execution=execution)

    def worker(self, compiler, root, mode, timeout):
        model = self
        class Worker:
            def __init__(self):
                self.process = SimpleNamespace(pid=next(model.pids))
                self.index = 0
                self.stats = dict.fromkeys(subject.ast.COUNTERS, 0)
                self.closed = False

            def compile(self, argv):
                expected = subject.cases()[self.index]
                self.index += 1
                kind = expected[-1]
                key = 'hits' if kind == 'hit' else 'rejected' if kind == 'reject' else 'misses'
                self.stats[key] += 1
                if kind != 'reject':
                    self.stats.update(retained_entries=1, retained_estimated_bytes=64)
                result = model.compile(root, self.process.pid)
                result.update(ast=dict(self.stats), executed=True,
                    owned=dict(retirement_reason=None),
                    lexer=dict.fromkeys(subject.ast.COUNTERS, 0),
                    live=dict.fromkeys(subject.ast.LIVE_COUNTERS, 0))
                return result

            def close(self):
                self.closed = True
        worker = Worker()
        self.workers.append(worker)
        return worker


class HarnessTests(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory()
        self.addCleanup(self.temp.cleanup)
        self.root = Path(self.temp.name)
        self.compiler = self.root / 'compiler'
        self.compiler.write_bytes(b'mock only, never executed')
        self.compiler.chmod(0o700)

    def patches(self, model):
        stack = self.enterContext(__import__('contextlib').ExitStack())
        stack.enter_context(mock.patch.object(subject, 'run', side_effect=model.run))
        stack.enter_context(mock.patch.object(subject, 'verify_consumer', side_effect=model.consumer))
        stack.enter_context(mock.patch.object(subject.ast, 'AstResident', side_effect=model.worker))
        return stack

    def capture(self, model, **kwargs):
        self.patches(model)
        self.enterContext(mock.patch.object(subject, 'host_guard', return_value={'host': 'mock'}))
        self.enterContext(mock.patch.object(subject.shared, 'identity', return_value={'compiler': 'frozen'}))
        return subject.capture(self.compiler, self.root / 'evidence', samples=2,
                               execute_host_workloads=True, **kwargs)

    def test_full_mock_workflow(self):
        model = Model()
        report = self.capture(model)
        self.assertEqual(report['verdict'], 'passed', report)
        self.assertEqual(len(report['samples']), 2)
        self.assertNotIn('initial', report['median_compiler_seconds'])
        self.assertEqual(len(report['cold_compiler_seconds']), 2)
        self.assertTrue(all(worker.closed for worker in model.workers))
        for sample in report['samples']:
            self.assertEqual([c['verdict'] for c in sample['controls']], ['passed', 'passed'])
            for row in sample['observations']:
                self.assertEqual(set(row['records']), set(subject.MODES))
                inc = row['records']['incremental']['argv']
                disk = row['records']['disk-ast']['argv']
                self.assertTrue(any(a.startswith('-Cincremental=') for a in inc))
                self.assertTrue(any(a.startswith('-Cincremental=') for a in disk))
                self.assertFalse(any(a.startswith('-Cincremental=') for a in row['records']['fresh']['argv']))
        for command, env in model.calls:
            self.assertEqual(command[0], self.compiler)
            if subject.DISK_ENV[0] not in env:
                self.assertNotIn(subject.DISK_ENV[1], env)

    def test_missing_hit_suppresses_every_median(self):
        report = self.capture(Model(missing_hits=True))
        self.assertEqual(report['verdict'], 'failed')
        self.assertNotIn('median_compiler_seconds', report)
        self.assertTrue(any('genuine' in f['error'] for f in report['samples'][0]['failures']))

    def test_malformed_telemetry_retains_raw_compile(self):
        report = self.capture(Model(malformed=True))
        self.assertEqual(report['verdict'], 'failed')
        receipt = json.loads((self.root / 'evidence/sample-000/receipt.json').read_text())
        rows = receipt['observations']
        self.assertIn('stdout_b64', rows[0]['records']['disk-ast']['compile'])
        self.assertNotIn('median_compiler_seconds', report)

    def test_identity_drift_blocks_medians(self):
        self.patches(Model())
        self.enterContext(mock.patch.object(subject, 'host_guard', return_value={}))
        self.enterContext(mock.patch.object(subject.shared, 'identity', side_effect=[{'sha': 'a'}, {'sha': 'b'}]))
        report = subject.capture(self.compiler, self.root / 'evidence', samples=1,
                                 execute_host_workloads=True)
        self.assertEqual(report['verdict'], 'failed')
        self.assertIn('frozen compiler changed', report['error'])
        self.assertNotIn('median_compiler_seconds', report)

    def test_no_overwrite_existing_evidence(self):
        output = self.root / 'evidence'
        output.mkdir()
        (output / 'receipt.json').write_text('untouched')
        with mock.patch.object(subject, 'host_guard', return_value={}):
            with self.assertRaises(FileExistsError):
                subject.capture(self.compiler, output, execute_host_workloads=True)
        self.assertEqual((output / 'receipt.json').read_text(), 'untouched')

    def test_ack_guard_before_any_compiler_call(self):
        with mock.patch.object(subject.subprocess, 'Popen') as popen:
            with self.assertRaisesRegex(ValueError, 'explicit'):
                subject.capture(self.compiler, self.root / 'never')
            popen.assert_not_called()
        self.assertFalse((self.root / 'never').exists())

    def test_missing_host_marker_fails_closed(self):
        with mock.patch.object(subject, 'HOST_MARKER', self.root / 'absent'):
            with self.assertRaisesRegex(ValueError, 'hosted-only'):
                subject.host_guard(True)

    def test_host_marker_permissions(self):
        marker = mock.Mock()
        marker.is_file.return_value = True
        for uid, mode in ((1000, 0o100644), (0, 0o100666)):
            marker.stat.return_value = SimpleNamespace(st_uid=uid, st_mode=mode)
            with mock.patch.object(subject, 'HOST_MARKER', marker):
                with self.assertRaisesRegex(ValueError, 'root-owned'):
                    subject.host_guard(True)

    def test_input_validation(self):
        with mock.patch.object(subject, 'host_guard', return_value={}):
            for count, timeout in ((0, 1), (True, 1), (1, float('nan')), (1, float('inf')), (1, 0)):
                with self.assertRaises(ValueError):
                    subject.capture(self.compiler, self.root / 'never', samples=count, timeout=timeout)
            with self.assertRaisesRegex(ValueError, 'absolute'):
                subject.capture(Path('relative'), self.root / 'never')
        self.assertFalse((self.root / 'never').exists())

    def test_counterbalance(self):
        positions = [subject.ordered_modes(s, 0) for s in range(4)]
        for index in range(4):
            self.assertEqual({order[index] for order in positions}, set(subject.MODES))
        self.assertEqual(subject.ordered_modes(4, 0), tuple(reversed(subject.MODES)))

    def test_fixture_clean_and_helper_edit_preserves_root(self):
        cases = subject.cases()
        self.assertEqual(cases[0][1], cases[2][1])
        self.assertNotEqual(cases[0][2], cases[2][2])
        self.assertNotIn('#', cases[0][1])
        self.assertIn('repeat-type-error', [case[0] for case in cases])
        self.assertEqual(cases[-1][0], 'type-recovery')

    def test_environment_does_not_leak_disk_gates(self):
        with mock.patch.dict(os.environ, {subject.DISK_ENV[0]: '/foreign', subject.DISK_ENV[1]: '/foreign-stats'}):
            env = subject.environment()
            self.assertTrue(all(key not in env for key in subject.DISK_ENV))
            with subject.no_inherited_disk():
                self.assertTrue(all(key not in os.environ for key in subject.DISK_ENV))
            self.assertEqual(os.environ[subject.DISK_ENV[0]], '/foreign')

    def test_telemetry_pid_and_append_contract(self):
        path = self.root / 'stats'
        path.write_text(json.dumps(dict(event='hit', pid=12)) + '\n')
        events, offset = subject.events_since(path, 0, 12)
        self.assertEqual(events[0]['event'], 'hit')
        with self.assertRaisesRegex(AssertionError, 'PID'):
            subject.events_since(path, 0, 13)
        path.write_text('')
        with self.assertRaisesRegex(AssertionError, 'truncated'):
            subject.events_since(path, offset, 12)

    def test_invalid_telemetry_event(self):
        path = self.root / 'stats'
        for value in ({'event': 'restore', 'pid': 1}, {'event': 'hit', 'pid': True}, []):
            path.write_text(json.dumps(value) + '\n')
            with self.assertRaises(AssertionError):
                subject.events_since(path, 0, 1)

    def test_no_medians_for_failed_or_empty_results(self):
        for samples in ([], [{'verdict': 'failed'}], [{'verdict': 'incomplete'}]):
            with self.assertRaisesRegex(AssertionError, 'no medians'):
                subject.summaries(samples)

    def test_mock_process_timeout_is_reaped(self):
        process = mock.Mock(pid=42, returncode=-9)
        process.communicate.side_effect = [subject.subprocess.TimeoutExpired(['mock'], 1), (b'full out', b'full error')]
        with mock.patch.object(subject.subprocess, 'Popen', return_value=process):
            result = subject.run(['mock'], 1, self.root, {})
        self.assertTrue(result['timed_out'])
        self.assertEqual(result['pid'], 42)
        self.assertEqual(result['stderr_b64'], subject.shared.encoded(b'full error'))
        process.kill.assert_called_once()
        self.assertEqual(process.communicate.call_count, 2)

    def test_cli_defaults_and_explicit_ack(self):
        with mock.patch.object(subject, 'capture', return_value={'verdict': 'passed'}) as capture:
            with mock.patch('builtins.print'):
                self.assertEqual(subject.main(['--compiler', '/frozen', '--output', '/new', '--execute-host-workloads']), 0)
            self.assertEqual(capture.call_args.kwargs['samples'], 5)
            self.assertTrue(capture.call_args.kwargs['execute_host_workloads'])

    def test_runtime_nonce_and_diagnostics_gates(self):
        model = Model()
        report = self.capture(model)
        sample = report['samples'][0]
        row = copy.deepcopy(sample['observations'][1])
        row['records']['disk-ast']['compile']['stderr_b64'] = 'changed'
        with self.assertRaisesRegex(AssertionError, 'parity'):
            subject.gate_row(row, None, sample['live_pid'], set(), set())
        row = copy.deepcopy(sample['observations'][1])
        fresh_pid = row['records']['fresh']['compile']['pid']
        with self.assertRaisesRegex(AssertionError, 'PID reused'):
            subject.gate_row(row, None, sample['live_pid'], {fresh_pid}, set())
        row = copy.deepcopy(sample['observations'][1])
        nonce = row['records']['fresh']['runtime']['nonce']
        with self.assertRaisesRegex(AssertionError, 'nonce reused'):
            subject.gate_row(row, None, sample['live_pid'], set(), {nonce})


if __name__ == '__main__':
    unittest.main()
