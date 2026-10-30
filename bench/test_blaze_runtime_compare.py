#!/usr/bin/env python3
"""Synthetic harness tests. Never load Blaze or a native compiler."""
import argparse
import json
import os
from pathlib import Path
import signal
import sys
import tempfile
import unittest
from unittest.mock import patch

import blaze_runtime_compare as h

NAME = 'summary::tests::shares_overlapping_time'
ORIGINAL = b'fn shares_overlapping_time() {\n}\n'


def output(names=(NAME,), marker='', failed=False):
    lines = [f'test {name} ... ' + (marker + '\n' if marker else '') +
             ('FAILED' if failed and name == NAME else 'ok') for name in names]
    return '\n'.join(lines) + (f'\ntest result: {"FAILED" if failed else "ok"}. '
        f'{len(names) - int(failed)} passed; {int(failed)} failed; 0 ignored; 0 measured; 2 filtered out\n')


class HarnessTests(unittest.TestCase):
    def test_false_success_and_counts(self):
        row = dict(status=0, timed_out=False, error=None)
        for text in ('Finished compilation', '', output().replace('1 passed', '2 passed')):
            self.assertFalse(h.accepted(row, text))
        self.assertTrue(h.accepted(row, output()))
        self.assertFalse(h.accepted(dict(row, status=-9), output()))
        self.assertFalse(h.accepted(dict(row, timed_out=True), output()))

    def test_missing_extra_and_stale(self):
        row = dict(status=0, timed_out=False, error=None)
        expected = h.test_results(output())
        for names in (('different',), (NAME, 'extra')):
            self.assertFalse(h.accepted(row, output(names), expected))
        self.assertFalse(h.accepted(row, output(), expected, 'JR_RUNTIME_EXECUTED_2'))
        self.assertFalse(h.accepted(row, output(marker='JR_RUNTIME_EXECUTED_20'), expected,
                                    'JR_RUNTIME_EXECUTED_2'))
        self.assertTrue(h.accepted(row, output(marker='JR_RUNTIME_EXECUTED_2'), expected,
                                   'JR_RUNTIME_EXECUTED_2'))
        for marker in ('JR_RUNTIME_EXECUTED_2 JR_RUNTIME_EXECUTED_2',
                       'JR_RUNTIME_EXECUTED_1 JR_RUNTIME_EXECUTED_2'):
            self.assertFalse(h.accepted(row, output(marker=marker), expected,
                                       'JR_RUNTIME_EXECUTED_2'))

    def test_failure_output(self):
        parsed = h.test_results(output(marker='panic: JR_RUNTIME_FAILURE', failed=True))
        self.assertEqual(parsed['failed'], 1)
        self.assertEqual(parsed['tests'], [(NAME, 'FAILED')])

    def test_same_edits_and_order(self):
        stacks = ['owned-off', 'all3on', 'blaze']
        for rep in range(5):
            self.assertEqual(len({h.edit_bytes(ORIGINAL, rep) for _ in stacks}), 1)
            self.assertEqual(h.order(stacks, rep)[0], stacks[rep % 3])
        self.assertNotEqual(h.edit_bytes(ORIGINAL, 0), h.edit_bytes(ORIGINAL, 1))
        with self.assertRaises(ValueError):
            h.edit_bytes(ORIGINAL + ORIGINAL)

    def test_environment_privacy(self):
        env = dict(h.PROFILES, SECRET_TOKEN='do not log', AWS_SECRET_ACCESS_KEY='secret',
                   BLAZE_BACKEND='llvm')
        logged = h.logged_environment(env)
        self.assertNotIn('SECRET_TOKEN', logged)
        self.assertNotIn('AWS_SECRET_ACCESS_KEY', logged)
        self.assertEqual(logged['CARGO_PROFILE_TEST_OPT_LEVEL'], '0')
        self.assertEqual(logged['BLAZE_BACKEND'], 'llvm')

    def test_timeout_signal_exact_output(self):
        with tempfile.TemporaryDirectory() as temp:
            root = Path(temp)
            row = h.run_process([sys.executable, '-c',
                'import time; print("exact", flush=True); time.sleep(10)'], root,
                os.environ.copy(), root / 'timeout', .1)
            self.assertTrue(row['timed_out'])
            self.assertEqual(row['status'], -signal.SIGKILL)
            self.assertEqual((root / 'timeout.stdout').read_bytes(), b'exact\n')
            row = h.run_process([sys.executable, '-c',
                'import os,signal; os.kill(os.getpid(),signal.SIGTERM)'], root,
                os.environ.copy(), root / 'signal', 5)
            self.assertFalse(row['timed_out'])
            self.assertEqual(row['status'], -signal.SIGTERM)

    def exercise(self, root, crash=False, stale=False, raw_mode='valid'):
        source = root / 'source'
        (source / 'src').mkdir(parents=True)
        pinned = source / 'src/summary.rs'
        pinned.write_bytes(ORIGINAL)
        args = argparse.Namespace(source=source, output=root / 'out', samples=2,
            edit_file='src/summary.rs', edit_test=NAME, edit_anchor='fn shares_overlapping_time() {',
            blaze_llvm=True, command='test', test_filter='summary::tests::', timeout=1,
            binary='justrust', blaze=Path('/fake/blaze'), justrust=Path('/fake/justrust'))
        observed = []
        def fake(command, cwd, env, prefix, timeout):
            contents = pinned.read_bytes()
            observed.append((prefix.name, contents))
            if crash:
                raise RuntimeError('synthetic interruption')
            text, status = output(), 0
            if b'JR_RUNTIME_TYPE_ERROR' in contents:
                text, status = 'JR_RUNTIME_TYPE_ERROR error[E0308]', 101
            elif b'JR_RUNTIME_FAILURE' in contents:
                text, status = output(marker='JR_RUNTIME_FAILURE', failed=True), 101
            elif b'JR_RUNTIME_EXECUTED_' in contents and not stale:
                import re
                marker = re.search(rb'JR_RUNTIME_EXECUTED_\d+', contents).group().decode()
                text = output(marker=marker)
            if command[0] == '/fake/justrust':
                recording = Path(env['JUSTRUST_HOME']) / 'runs' / f'run-{len(observed)}'
                if raw_mode != 'missing':
                    recording.mkdir(parents=True)
                    raw = ''.join(json.dumps({'t': i, 's': 'o', 'l': line}) + '\n'
                                  for i, line in enumerate(text.splitlines()))
                    (recording / 'output.jsonl').write_text(raw if raw_mode == 'valid' else '{bad json')
                text = 'justrust: condensed UI, no test names or diagnostics'
            prefix.with_suffix('.stdout').write_text(text)
            prefix.with_suffix('.stderr').write_text('')
            return dict(status=status, error=None, timed_out=False, elapsed_seconds=.001)
        with patch.object(h.base, 'identities', return_value={'fake': 'identity'}), \
             patch.object(h, 'environment', side_effect=lambda a, s: {
                 'HOME': str(a.output / s), 'JUSTRUST_HOME': str(a.output / s / '.justrust')}), \
             patch.object(h, 'run_process', side_effect=fake):
            if crash:
                with self.assertRaises(RuntimeError):
                    h.execute(args)
            else:
                self.assertEqual(h.execute(args), 1 if stale or raw_mode != 'valid' else 0)
        self.assertEqual(pinned.read_bytes(), ORIGINAL)
        self.assertEqual((args.output / 'source.original').read_bytes(), ORIGINAL)
        return observed

    def test_end_to_end_gates_same_bytes_rotation(self):
        with tempfile.TemporaryDirectory() as temp:
            rows = self.exercise(Path(temp))
            for rep in range(2):
                edits = [(name, data) for name, data in rows if name.startswith(f'edit-{rep:03d}')]
                self.assertEqual(len(edits), 4)
                self.assertEqual(len({data for _, data in edits}), 1)
                self.assertIn('owned-off' if rep == 0 else 'all3on', edits[0][0])

    def test_missing_or_malformed_raw_logs_excluded(self):
        for mode in ('missing', 'malformed'):
            with self.subTest(mode=mode), tempfile.TemporaryDirectory() as temp:
                self.exercise(Path(temp), raw_mode=mode)
                summary = json.loads((Path(temp) / 'out/summary.json').read_text())
                self.assertNotIn('owned-off', summary['medians_seconds'])
                self.assertNotIn('all3on', summary['medians_seconds'])
                records = json.loads((Path(temp) / 'out/records.json').read_text())
                self.assertTrue(all(r.get('raw_output_error') for r in records
                                    if r['stack'] in ('owned-off', 'all3on')))

    def test_raw_logs_reject_stale_and_ambiguous_runs(self):
        with tempfile.TemporaryDirectory() as temp:
            root = Path(temp)
            env = {'JUSTRUST_HOME': str(root / 'private')}
            runs = root / 'private/runs'
            old = runs / 'old'
            old.mkdir(parents=True)
            (old / 'output.jsonl').write_text(json.dumps({'s': 'o', 'l': output()}) + '\n')
            before = h.recorded_runs(env)
            with self.assertRaises(ValueError):
                h.recorded_output(env, before, root / 'stale')
            for name in ('new-a', 'new-b'):
                (runs / name).mkdir()
            with self.assertRaises(ValueError):
                h.recorded_output(env, before, root / 'ambiguous')

    def test_raw_log_artifact_and_no_console_duplication(self):
        with tempfile.TemporaryDirectory() as temp:
            self.exercise(Path(temp))
            root = Path(temp) / 'out'
            records = json.loads((root / 'records.json').read_text())
            row = next(r for r in records if r['stack'] == 'owned-off' and r['phase'] == 'edit')
            self.assertEqual(row['elapsed_seconds'], .001)
            artifact = root / row['raw_output']['artifact']
            self.assertTrue(artifact.is_file())
            self.assertEqual(h.base.digest(artifact), row['raw_output']['sha256'])
            self.assertIn('JR_RUNTIME_EXECUTED_1', artifact.read_text())
            self.assertNotIn('JR_RUNTIME_EXECUTED', (root / 'edit-000-owned-off.stdout').read_text())
            self.assertTrue(row['accepted'])

    def test_restore_on_exception(self):
        with tempfile.TemporaryDirectory() as temp:
            self.exercise(Path(temp), crash=True)

    def test_stale_runtime_excluded(self):
        with tempfile.TemporaryDirectory() as temp:
            self.exercise(Path(temp), stale=True)


if __name__ == '__main__':
    unittest.main()
