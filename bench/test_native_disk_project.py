#!/usr/bin/env python3
"""Mock-only native real-project runner checks. Never executes Rust."""
import argparse
import json
import os
from pathlib import Path
import tempfile
import unittest
from unittest.mock import patch

import native_disk_project as h


class ProjectTests(unittest.TestCase):
    def test_host_guard_before_any_native_execution(self):
        with patch.object(h.snapshot, 'host_guard', side_effect=ValueError('hosted-only')), \
                patch.object(h.runtime, 'run_process') as run:
            with self.assertRaisesRegex(ValueError, 'hosted-only'):
                h.execute(argparse.Namespace(execute_host_workloads=False))
            run.assert_not_called()

    def test_private_environment_no_credentials_or_inherited_flags(self):
        with tempfile.TemporaryDirectory() as temp:
            with patch.dict(os.environ, {'JUSTRUST_API_KEY': 'secret', 'RUSTFLAGS': 'poison',
                                         'CARGO_ENCODED_RUSTFLAGS': 'poison',
                                         'RUSTC_WRAPPER': '/bad', 'AWS_SECRET_ACCESS_KEY': 'secret'}):
                env = h.environment(Path(temp), Path('/frozen/rustc'), 'off')
            self.assertEqual(env['JUSTRUST_TOOLCHAIN'], 'system')
            self.assertEqual(env['RUSTC'], '/frozen/rustc')
            self.assertEqual(env['RUSTFLAGS'], '')
            self.assertNotIn('RUSTC_WRAPPER', env)
            self.assertNotIn('CARGO_ENCODED_RUSTFLAGS', env)
            self.assertNotIn('JUSTRUST_API_KEY', env)
            self.assertNotIn('AWS_SECRET_ACCESS_KEY', env)
            self.assertNotIn('JUSTRUST_RUSTC_DISK_AST', env)
            on = h.environment(Path(temp), Path('/frozen/rustc'), 'on', True)
            self.assertEqual(on['RUSTFLAGS'], '-Ztime-passes')
            self.assertEqual(on['JUSTRUST_RUSTC_DISK_AST'], temp + '/disk')

    def test_telemetry_requires_pid_and_preserves_source(self):
        with tempfile.TemporaryDirectory() as temp:
            path = Path(temp) / 'stats'
            event = dict(event='reject', pid=42, source='src/lib.rs', reason='module')
            path.write_text(json.dumps(event) + '\n')
            events, offset = h.telemetry(path, 0)
            self.assertEqual(events, [event])
            self.assertEqual(h.telemetry(path, offset)[0], [])
            path.write_text('')
            with self.assertRaisesRegex(ValueError, 'truncated'):
                h.telemetry(path, offset)
            path.write_text('{"event":"hit"}\n')
            with self.assertRaisesRegex(ValueError, 'PID'):
                h.telemetry(path, 0)

    def test_coverage_never_infers_module_hits(self):
        with tempfile.TemporaryDirectory() as temp:
            source = Path(temp)
            (source / 'src').mkdir()
            (source / 'src/lib.rs').write_text('mod summary;')
            data = h.coverage([dict(pid=12, event='reject', source='src/lib.rs'),
                               dict(pid=13, event='hit')], source)
            self.assertEqual(list(data['files']), [str(source / 'src/lib.rs')])
            self.assertEqual(len(data['unattributed_events']), 1)
            self.assertNotIn(str(source / 'src/summary.rs'), data['files'])

    def test_no_medians_until_all_gates_and_never_zero_hit_acceptance(self):
        rows = [dict(mode=mode, phase=phase, parsed={'tests': ['a']},
                     accepted=True, elapsed_seconds=1, disk_events=[])
                for mode in ('off', 'on') for phase in h.PHASES]
        summary = h.summarize(rows, True, 1)
        self.assertTrue(summary['runtime_identity_gates_passed'])
        self.assertFalse(summary['cache_speedup_accepted'])
        self.assertIn('Zero disk hits', summary['cache_policy'])
        self.assertTrue(summary['medians_seconds'])
        rows[0]['accepted'] = False
        self.assertEqual(h.summarize(rows, True, 1)['medians_seconds'], {})
        self.assertEqual(h.summarize(rows[:-1], True, 1)['medians_seconds'], {})

    def test_foreground_identity_rejects_wrong_compiler(self):
        with tempfile.TemporaryDirectory() as temp:
            root = Path(temp)
            source = root / 'source'
            (source / 'src').mkdir(parents=True)
            (source / 'src/summary.rs').write_text('original')
            env = h.environment(root, Path('/frozen/rustc'), 'off')
            run_id = '20261118-123456789-42'
            meta = Path(env['JUSTRUST_HOME']) / 'runs' / run_id / 'meta.json'
            meta.parent.mkdir(parents=True)
            meta.write_text(json.dumps(dict(id=run_id, cwd=str(source),
                args=['test', *h.shared.FILTER], real_rustc='/wrong/rustc')))
            calls = []

            def run(command, cwd, environ, prefix, timeout):
                calls.append(command)
                prefix.with_suffix('.stdout').write_text('justrust log ' + run_id)
                prefix.with_suffix('.stderr').write_text('')
                return dict(status=0, error=None, timed_out=False, elapsed_seconds=0.1)

            args = argparse.Namespace(client=Path('/client'), compiler=Path('/frozen/rustc'), timeout=1)
            with patch.object(h.runtime, 'run_process', side_effect=run):
                row = h.invoke(args, source, env, root / 'edit', 'edit', 'JR_RUNTIME_EXECUTED_3')
            self.assertFalse(row['identity_verified'])
            self.assertFalse(row['accepted'])
            self.assertEqual(calls[0][2], '--local=' + h.REASON)
            self.assertEqual(calls[1], ['/client', 'log', run_id])
            self.assertTrue((root / 'edit.run-meta.json').exists())

    def test_mock_full_execution_restores_sources_and_separates_attribution(self):
        from test_ultrafast_hosted_compare import output
        with tempfile.TemporaryDirectory() as temp:
            root = Path(temp)
            source = root / 'fixture'
            (source / 'src').mkdir(parents=True)
            original = b'fn shares_overlapping_time() {\n}\n'
            (source / 'src/summary.rs').write_bytes(original)
            for name in ('compiler', 'client'):
                (root / name).write_bytes(b'\x7fELFmock')
                (root / name).chmod(0o700)
            args = argparse.Namespace(source=source, output=root / 'out',
                compiler=root / 'compiler', client=root / 'client', samples=1,
                timeout=1, raw_rustc_phases=True, execute_host_workloads=True)
            count = 0
            logs = {}

            def run(command, cwd, env, prefix, timeout):
                nonlocal count
                count += 1
                text, status = '', 0
                if command[1] == 'test':
                    phase = prefix.name
                    run_id = f'20261118-123456789-{count}'
                    meta = Path(env['JUSTRUST_HOME']) / 'runs' / run_id / 'meta.json'
                    meta.parent.mkdir(parents=True)
                    arguments = ['--lib', '--', '--test-threads=1', '--nocapture'] if phase == 'full-library' else h.shared.FILTER
                    meta.write_text(json.dumps(dict(id=run_id, cwd=str(cwd),
                        args=['test', *arguments], real_rustc=str(args.compiler))))
                    content = (cwd / 'src/summary.rs').read_text()
                    marker = __import__('re').search(r'JR_RUNTIME_EXECUTED_\d+', content)
                    if phase == 'type-error':
                        status, logs[run_id] = 101, 'E0308 JR_RUNTIME_TYPE_ERROR'
                    else:
                        status = 101 if phase == 'assert-failure' else 0
                        logs[run_id] = output(marker.group(), failing=phase == 'assert-failure',
                                              full=phase == 'full-library')
                    text = 'justrust log ' + run_id
                elif command[1] == 'log':
                    text = logs[command[2]]
                prefix.with_suffix('.stdout').write_text(text)
                prefix.with_suffix('.stderr').write_text('')
                return dict(status=status, error=None, timed_out=False, elapsed_seconds=0.1)

            with patch.object(h.snapshot, 'host_guard', return_value={'mock': True}), \
                    patch.object(h.shared, 'pinned_source'), \
                    patch.object(h.runtime, 'run_process', side_effect=run):
                self.assertEqual(h.execute(args), 0)
            receipt = json.loads((args.output / 'receipt.json').read_text())
            self.assertEqual(len(receipt['rows']), 16)
            self.assertEqual(len(receipt['attribution_rows']), 16)
            self.assertTrue(receipt['summary']['runtime_identity_gates_passed'])
            self.assertFalse(receipt['summary']['cache_speedup_accepted'])
            self.assertTrue(all(row['rustflags'] == '' for row in receipt['rows']))
            self.assertTrue(all(row['rustflags'] == '-Ztime-passes' for row in receipt['attribution_rows']))
            self.assertEqual((source / 'src/summary.rs').read_bytes(), original)
            for copy in args.output.glob('*/*/*/source/src/summary.rs'):
                self.assertEqual(copy.read_bytes(), original)

    def test_oracle_not_weakened(self):
        self.assertIs(h.oracle.accepted, __import__('ultrafast_hosted_compare').accepted)
        self.assertIn('type-error', h.PHASES)
        self.assertIn('assert-failure', h.PHASES)
        self.assertIn('full-library', h.PHASES)


if __name__ == '__main__':
    unittest.main()
