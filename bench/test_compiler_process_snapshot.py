#!/usr/bin/env python3
"""Mocked harness checks only. Never compiler/checkpoint performance evidence."""
import json
from pathlib import Path
import sys
import tempfile
from types import SimpleNamespace
import unittest
from unittest.mock import patch

import compiler_process_snapshot as snapshot


def result(status=0):
    return dict(status=status, timed_out=False, stdout_b64='', stderr_b64='',
                command=['mock-only'], elapsed_seconds=.01)


class SnapshotTests(unittest.TestCase):
    def test_mode_order_requires_exact_permutation(self):
        self.assertEqual(snapshot.mode_order('restored,fresh,warm'), ('restored', 'fresh', 'warm'))
        for value in ('fresh,fresh,restored', 'warm,restored', 'fresh,warm,restored,unknown'):
            with self.assertRaises(ValueError):
                snapshot.mode_order(value)

    def test_provenance_has_host_and_harness_hashes(self):
        info = snapshot.provenance()
        self.assertIn('kernel', info)
        self.assertIn('hostname', info)
        self.assertEqual(len(info['harness_sources']), 3)
        self.assertTrue(all(len(row['sha256']) == 64 for row in info['harness_sources']))

    def test_cli_repeats_use_new_directories_and_preserve_order(self):
        with tempfile.TemporaryDirectory() as temp:
            root = Path(temp) / 'new'
            argv = ['compiler_process_snapshot.py', '--compiler', '/mock/compiler',
                    '--criu', '/mock/criu', '--out', str(root), '--repeats', '2',
                    '--mode-order', 'restored,warm,fresh']
            with patch.object(sys, 'argv', argv), \
                    patch.object(snapshot, 'capture', return_value={'verdict': 'passed'}) as capture, \
                    patch('builtins.print'):
                self.assertEqual(snapshot.main(), 0)
            self.assertEqual(capture.call_count, 2)
            self.assertEqual(capture.call_args_list[0].args[1], root / 'repeat-000')
            self.assertEqual(capture.call_args_list[1].args[1], root / 'repeat-001')
            self.assertEqual(capture.call_args.args[4], ('restored', 'warm', 'fresh'))
            report = json.loads((root / 'receipt.json').read_text())
            self.assertEqual(report['verdict'], 'passed')
            self.assertEqual(report['repeats_requested'], 2)

    def test_cli_repeats_stop_after_failure(self):
        with tempfile.TemporaryDirectory() as temp:
            root = Path(temp) / 'new'
            argv = ['snapshot.py', '--compiler', '/mock/compiler', '--criu', '/mock/criu',
                    '--out', str(root), '--repeats', '3']
            with patch.object(sys, 'argv', argv), \
                    patch.object(snapshot, 'capture', return_value={'verdict': 'failed'}) as capture, \
                    patch('builtins.print'):
                self.assertEqual(snapshot.main(), 1)
            self.assertEqual(capture.call_count, 1)
            self.assertEqual(json.loads((root / 'receipt.json').read_text())['verdict'], 'failed')

    def test_requires_memory_images_not_just_metadata(self):
        with tempfile.TemporaryDirectory() as temp:
            root = Path(temp)
            (root / 'inventory.img').write_bytes(b'mock inventory')
            with self.assertRaisesRegex(AssertionError, 'memory pages'):
                snapshot.image_inventory(root)
            (root / 'pages-1.img').write_bytes(b'mock pages')
            rows = snapshot.image_inventory(root)
            self.assertEqual(len(rows), 2)
            self.assertTrue(all(len(row['sha256']) == 64 for row in rows))

    def test_dump_failure_preserves_original_command_receipt(self):
        with tempfile.TemporaryDirectory() as temp:
            worker = SimpleNamespace(pid=123, timeout=5, alive=True)
            evidence = {}
            with patch.object(snapshot.shared, 'run', return_value=result(1)) as run:
                with self.assertRaisesRegex(RuntimeError, 'dump unsupported'):
                    snapshot.checkpoint(worker, Path('/mock/criu'), Path(temp) / 'images', evidence)
            self.assertEqual(evidence['dump']['status'], 1)
            command = run.call_args.args[0]
            self.assertNotIn('--leave-running', command)
            self.assertNotIn('--lazy-pages', command)
            self.assertEqual(command[1], 'dump')
            self.assertTrue(worker.alive)

    def test_unsupported_check_retains_receipt_and_sources(self):
        with tempfile.TemporaryDirectory() as temp:
            root = Path(temp)
            source = root / 'fixture.rs'
            source.write_bytes(b'original source')
            with patch.object(snapshot.shared, 'run', side_effect=[result(), result(1)]), \
                    patch.object(snapshot, 'DiskWorker') as worker:
                report = snapshot.capture(Path('/mock/compiler'), root, Path('/mock/criu'), 1)
            worker.assert_not_called()
            self.assertEqual(report['verdict'], 'failed')
            self.assertIn('CRIU check', report['error'])
            self.assertEqual(source.read_bytes(), b'original source')
            self.assertFalse((root / 'helper.rs').exists())
            self.assertEqual(json.loads((root / 'receipt.json').read_text()), report)

    def test_missing_criu_is_failure_receipt(self):
        with tempfile.TemporaryDirectory() as temp:
            root = Path(temp)
            with patch.object(snapshot.shared, 'run', side_effect=FileNotFoundError('original missing binary')):
                report = snapshot.capture(Path('/mock/compiler'), root, Path('/missing/criu'), 1)
            self.assertEqual(report['verdict'], 'failed')
            self.assertIn('original missing binary', report['error'])
            self.assertTrue((root / 'receipt.json').exists())

    def test_restore_failure_never_claims_running(self):
        with tempfile.TemporaryDirectory() as temp:
            worker = SimpleNamespace(pid=123, timeout=1, alive=False)
            evidence = {}
            with patch.object(snapshot.shared, 'run', return_value=result(1)) as run:
                with self.assertRaisesRegex(RuntimeError, 'disk restore'):
                    snapshot.restore(worker, Path('/mock/criu'), Path(temp), evidence)
            self.assertFalse(worker.alive)
            self.assertIn('--restore-detached', run.call_args.args[0])
            self.assertEqual(evidence['restore']['status'], 1)

    def test_surviving_original_rejected(self):
        with tempfile.TemporaryDirectory() as temp:
            worker = SimpleNamespace(pid=123, timeout=1, alive=True,
                                     process=SimpleNamespace(returncode=0))
            with patch.object(snapshot.shared, 'run', return_value=result()), \
                    patch.object(Path, 'exists', return_value=True):
                with self.assertRaisesRegex(AssertionError, 'survived dump'):
                    snapshot.checkpoint(worker, Path('/mock/criu'), Path(temp) / 'images', {})

    def test_full_mocked_workflow_snapshots_before_edit_and_restores_sources(self):
        with tempfile.TemporaryDirectory() as temp:
            root = Path(temp)
            workers = []

            class FakeWorker:
                def __init__(self, compiler, directory, channel, timeout):
                    self.pid = 100 + len(workers)
                    self.command = ['mock-worker', channel]
                    self.sequence = 0
                    workers.append(self)

                def compile(self, argv):
                    self.sequence += 1
                    row = result(1 if 'u32' in (root / 'helper.rs').read_text() else 0)
                    row['pid'] = self.pid
                    if row['status'] == 0:
                        (root / 'fixture').write_bytes(b'identical mock artifact')
                    return row

                def close(self):
                    pass

            def run(command, *args, **kwargs):
                if str(command[0]) == '/mock/compiler':
                    return workers[0].compile([]) | {'command': list(map(str, command))}
                return result()

            def runtime(binary, timeout, directory):
                source = (root / 'fixture.rs').read_text()
                helper = (root / 'helper.rs').read_text()
                values = ':'.join('false' if '{ false }' in text else 'true' for text in (source, helper))
                return dict(nonce='mock-' + str(time_counter[0]), execution=dict(
                    status=0, stdout_b64=snapshot.shared.encoded(
                        ('mock-' + str(time_counter[0]) + ':' + values + '\n').encode()), stderr_b64=''))

            time_counter = [0]

            def next_runtime(*args):
                time_counter[0] += 1
                return runtime(*args)

            previous = [snapshot.owned.cases()[0][1:3]]

            def checkpoint(worker, criu, directory, evidence):
                self.assertEqual(tuple(p.read_text() for p in (root / 'fixture.rs', root / 'helper.rs')), tuple(previous[0]))
                evidence['dump'] = result()

            def restore(worker, criu, directory, evidence):
                previous[0] = tuple(p.read_text() for p in (root / 'fixture.rs', root / 'helper.rs'))
                evidence['restore'] = result()

            with patch.object(snapshot, 'DiskWorker', FakeWorker), \
                    patch.object(snapshot.shared, 'identity', return_value={'mock': True}), \
                    patch.object(snapshot.shared, 'run', side_effect=run), \
                    patch.object(snapshot.owned, 'runtime', side_effect=next_runtime), \
                    patch.object(snapshot, 'checkpoint', side_effect=checkpoint), \
                    patch.object(snapshot, 'restore', side_effect=restore):
                report = snapshot.capture(Path('/mock/compiler'), root, Path('/mock/criu'), 1,
                                          order=('restored', 'warm', 'fresh'))
            self.assertEqual(report['verdict'], 'passed', report.get('error'))
            self.assertEqual(report['mode_order'], ['restored', 'warm', 'fresh'])
            self.assertEqual(list(report['observations'][0]['modes']), ['restored', 'warm', 'fresh'])
            self.assertEqual(len(report['observations']), 7)
            self.assertFalse((root / 'fixture.rs').exists())
            for row in report['observations']:
                self.assertIn('restore_plus_compile_seconds', row['modes']['restored'])

    def test_source_restored_after_worker_start_failure(self):
        with tempfile.TemporaryDirectory() as temp:
            root = Path(temp)
            originals = {'fixture.rs': b'prior root', 'helper.rs': b'prior helper'}
            for name, content in originals.items():
                (root / name).write_bytes(content)
            with patch.object(snapshot.shared, 'run', return_value=result()), \
                    patch.object(snapshot.shared, 'identity', return_value={}), \
                    patch.object(snapshot, 'DiskWorker', side_effect=RuntimeError('unsupported worker')):
                report = snapshot.capture(Path('/mock/compiler'), root, Path('/mock/criu'), 1)
            self.assertEqual(report['verdict'], 'failed')
            for name, content in originals.items():
                self.assertEqual((root / name).read_bytes(), content)


if __name__ == '__main__':
    unittest.main()
