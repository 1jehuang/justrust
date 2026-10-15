#!/usr/bin/env python3
"""Foreign rustc liveness gates. No real compiler or process signaling."""
import contextlib
import io
import json
from pathlib import Path
import subprocess
import tempfile
from types import SimpleNamespace
import unittest
from unittest.mock import patch

import suite


class ForeignLoadTests(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory()
        self.addCleanup(self.temp.cleanup)
        self.proc = Path(self.temp.name)
        self.original_live = suite.rustc_process_is_live

    def process(self, pid, state='S', record=None):
        directory = self.proc / str(pid)
        directory.mkdir(exist_ok=True)
        (directory / 'stat').write_text(record if record is not None else f'{pid} (rustc) {state} 1 0 0\n')

    def load(self, stdout='', code=0, stderr='', failure=None):
        result = subprocess.CompletedProcess(['pgrep'], code, stdout, stderr)
        with patch.object(suite.Path, 'read_text', autospec=True) as read, \
                patch.object(suite.subprocess, 'run', side_effect=failure, return_value=result) as discover, \
                patch.object(suite, 'rustc_process_is_live') as live:
            real_read = self.real_read
            read.side_effect = lambda path, *a, **k: '0.25 0 0 1/1 1\n' if str(path) == '/proc/loadavg' else real_read(path, *a, **k)
            live.side_effect = lambda pid: self.original_live(pid, self.proc)
            value = suite.foreign_load()
            discover.assert_called_once_with(['pgrep', '-x', 'rustc'], capture_output=True, text=True)
            return value

    @property
    def real_read(self):
        return _REAL_READ

    def test_live_and_zombie_mix_counts_only_live(self):
        for pid, state in [(11, 'S'), (12, 'Z'), (13, 'D'), (14, 'X'), (15, 'x')]:
            self.process(pid, state)
        self.assertEqual(self.load('11\n12\n13\n14\n15\n'), (0.25, 2))

    def test_only_proven_zombies_or_dead_are_zero(self):
        for pid, state in [(11, 'Z'), (12, 'X'), (13, 'x')]:
            self.process(pid, state)
        self.assertEqual(self.load('11\n12\n13\n'), (0.25, 0))

    def test_no_processes_requires_explicit_no_match_status(self):
        self.assertEqual(self.load(code=1), (0.25, 0))
        for code, stdout in [(0, ''), (1, '11\n'), (2, '')]:
            with self.assertRaisesRegex(RuntimeError, 'discovery unknown'):
                self.load(stdout, code)

    def test_sleeping_blocked_stopped_and_runnable_are_all_counted(self):
        states = 'RSDTtKWPI'
        for pid, state in enumerate(states, 20):
            self.process(pid, state)
        self.assertEqual(self.load(''.join(f'{pid}\n' for pid in range(20, 20 + len(states)))), (0.25, len(states)))

    def test_discovery_command_failure_or_malformed_identifiers_aborts(self):
        with self.assertRaisesRegex(RuntimeError, 'discovery unknown'):
            self.load(failure=OSError('pgrep unavailable'))
        for output in ('abc\n', '0\n', '-1\n', '11\n11\n', ' 11\n'):
            with self.assertRaisesRegex(RuntimeError, 'discovery unknown'):
                self.load(output)
        with self.assertRaisesRegex(RuntimeError, 'discovery unknown'):
            self.load('11\n', stderr='discovery warning')

    def test_missing_unreadable_malformed_or_reused_process_aborts(self):
        with self.assertRaisesRegex(RuntimeError, 'state unknown'):
            self.load('11\n')
        for record in ('11 rustc S 1', '11 (rustc) Q 1', '11 (rustc) S',
                       '12 (rustc) S 1', '11 (other) S 1', '11 (rustc) S bad'):
            self.process(11, record=record)
            with self.assertRaisesRegex(RuntimeError, 'state unknown'):
                self.load('11\n')
        with patch.object(suite.Path, 'read_text', side_effect=PermissionError('denied')):
            with self.assertRaisesRegex(RuntimeError, 'state unknown'):
                self.original_live(11, self.proc)

    def test_mixed_measurement_policies_block_named_and_synthetic_comparison(self):
        env = {'snapshots': {'desktop': 'pin'}, 'host': 'host', 'cpu': 'cpu', 'ncpu': 16,
               'justrust_rev': 'rev', 'execution': 'local'}
        for synthetic in (False, True):
            a = {'env': env, **({'synth': {}} if synthetic else {'scenarios': {}})}
            b = {**a, 'env': {**env, 'foreign_rustc_policy': suite.FOREIGN_RUSTC_POLICY}}
            first, second = self.proc / 'a.json', self.proc / 'b.json'
            first.write_text(json.dumps(a))
            second.write_text(json.dumps(b))
            args = SimpleNamespace(a=first, b=second, floor=9)
            if synthetic:
                output = io.StringIO()
                with contextlib.redirect_stdout(output):
                    suite.cmd_compare(args)
                self.assertIn('different foreign_rustc_policy', output.getvalue())
                self.assertNotIn('delta', output.getvalue())
            else:
                with self.assertRaisesRegex(SystemExit, 'different foreign_rustc_policy'):
                    suite.cmd_compare(args)


_REAL_READ = Path.read_text

if __name__ == '__main__':
    unittest.main()
