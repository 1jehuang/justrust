#!/usr/bin/env python3
"""Offline mutation tests. Never loads a compiler, client, or daemon."""
import contextlib
import importlib.util
import io
import json
import os
from pathlib import Path
import shutil
import subprocess
import sys
import tempfile
import unittest
from unittest.mock import patch

ROOT = Path(__file__).resolve().parent
spec = importlib.util.spec_from_file_location('br45_validate', ROOT / 'validate.py')
v = importlib.util.module_from_spec(spec)
spec.loader.exec_module(v)


class EvidenceTests(unittest.TestCase):
    def copied(self):
        temporary = tempfile.TemporaryDirectory(dir=os.environ.get('JCODE_SCRATCH_DIR'))
        self.addCleanup(temporary.cleanup)
        destination = Path(temporary.name) / 'standalone'
        shutil.copytree(ROOT, destination, ignore=shutil.ignore_patterns('__pycache__'))
        return destination

    def replay(self, root, inventory=False):
        with patch.object(v, 'ROOT', root), contextlib.redirect_stdout(io.StringIO()):
            v.main(check_inventory=inventory)

    def change_json(self, root, name, mutate):
        path = root / name
        value = json.loads(path.read_text())
        mutate(value)
        path.write_text(json.dumps(value))

    def row(self, rows):
        return next(r for r in rows if r['stack'] == 'owned-jrdaemon' and r['phase'] == 'edit')

    def test_standalone_golden(self):
        root = self.copied()
        result = subprocess.run([sys.executable, str(root / 'validate.py')],
                                cwd=root.parent, capture_output=True, text=True)
        self.assertEqual(result.returncode, 0, result.stdout + result.stderr)
        self.assertIn('10 raw parked proofs', result.stdout)

    def test_semantic_receipt_mutations(self):
        cases = [
            ('records.json', lambda rows: self.row(rows).__setitem__('accepted', False)),
            ('records.json', lambda rows: self.row(rows).__setitem__('timed_out', True)),
            ('records.json', lambda rows: self.row(rows).__setitem__('status', 101)),
            ('records.json', lambda rows: self.row(rows).__setitem__('daemon_served', False)),
            ('records.json', lambda rows: self.row(rows)['environment'].__setitem__('JUSTRUST_CARGO_DAEMON', '0')),
            ('records.json', lambda rows: self.row(rows)['environment'].__setitem__('CARGO', '/stock')),
            ('records.json', lambda rows: self.row(rows)['environment'].__setitem__('JUSTRUST_REAL_CARGO', '/stock')),
            ('records.json', lambda rows: self.row(rows)['environment'].__setitem__('JUSTRUST_CACHE_PROC_MACROS', '1')),
            ('records.json', lambda rows: self.row(rows)['environment'].__setitem__('RUSTFLAGS', '-Zthreads=4')),
            ('records.json', lambda rows: self.row(rows)['environment'].__setitem__('HOME', '/shared')),
            ('records.json', lambda rows: self.row(rows).__setitem__('source_sha256', 'bad')),
            ('records.json', lambda rows: rows.pop()),
            ('metadata.json', lambda m: m.__setitem__('source_restored', False)),
            ('metadata.json', lambda m: m['identities_after']['native_cargo'].__setitem__('sha256', 'changed')),
            ('metadata.json', lambda m: m['seed_source_after'].__setitem__('bad', 'changed')),
            ('metadata.json', lambda m: m['native_daemon_cleanup'].pop()),
            ('summary.json', lambda m: m['medians_seconds'].__setitem__('owned-jrdaemon', .001)),
            ('native-cargo-version.json', lambda m: m.__setitem__('sha256_after', 'changed')),
        ]
        for name, mutation in cases:
            with self.subTest(name=name, mutation=cases.index((name, mutation))):
                root = self.copied()
                self.change_json(root, name, mutation)
                with self.assertRaises(AssertionError):
                    self.replay(root)

    def change_raw(self, root, stack, phase, old, new):
        rows = json.loads((root / 'records.json').read_text())
        row = next(r for r in rows if r['stack'] == stack and r['phase'] == phase)
        raw = row['raw_output']
        path = root / raw['artifact']
        data = path.read_bytes()
        assert old in data
        data = data.replace(old, new)
        path.write_bytes(data)
        # Update both integrity references so semantic checks, not checksum
        # failures, must catch the forged runtime/daemon evidence.
        raw['sha256'] = v.digest(data)
        (root / 'state' / stack / 'runs' / raw['run_id'] / 'output.jsonl').write_bytes(data)
        (root / 'records.json').write_text(json.dumps(rows))

    def test_raw_semantic_mutations_even_with_updated_hashes(self):
        cases = [
            ('edit', b'jr-cargo daemon: served by a parked server', b'jr-cargo daemon: not parked'),
            ('edit', b'JR_RUNTIME_EXECUTED_1', b'JR_RUNTIME_EXECUTED_2'),
            ('edit', b'JR_RUNTIME_EXECUTED_1', b'JR_RUNTIME_EXECUTED_1 JR_RUNTIME_EXECUTED_1'),
            ('type-error', b'E0308', b'E9999'),
            ('type-error', b'JR_RUNTIME_TYPE_ERROR', b'NO_TYPE_ERROR'),
            ('test-failure', b'JR_RUNTIME_FAILURE', b'NO_RUNTIME_FAILURE'),
            ('test-recovery', b'4 passed', b'3 passed'),
        ]
        for phase, old, new in cases:
            with self.subTest(phase=phase, old=old):
                root = self.copied()
                self.change_raw(root, 'owned-jrdaemon', phase, old, new)
                with self.assertRaises(AssertionError):
                    self.replay(root)

    def test_inventory_missing_extra_and_changed(self):
        for mutation in ('missing', 'extra', 'changed'):
            with self.subTest(mutation=mutation):
                root = self.copied()
                if mutation == 'missing':
                    (root / 'source.original').unlink()
                elif mutation == 'extra':
                    (root / 'extra').write_text('unexpected')
                else:
                    (root / 'runtime45.stdout').write_text('tampered')
                with self.assertRaises(AssertionError):
                    self.replay(root, inventory=True)

    def test_export_run_id_and_state_status(self):
        root = self.copied()
        path = next((root / 'exports').rglob('*.jsonl'))
        lines = path.read_text().splitlines()
        entry = json.loads(lines[0])
        entry['id'] = 'not-this-run'
        lines[0] = json.dumps(entry)
        path.write_text('\n'.join(lines) + '\n')
        with self.assertRaises(AssertionError):
            self.replay(root)
        root = self.copied()
        path = next((root / 'state').rglob('summary.json'))
        self.change_json(root, str(path.relative_to(root)), lambda m: m.__setitem__('exit', -9))
        with self.assertRaises(AssertionError):
            self.replay(root)


if __name__ == '__main__':
    unittest.main()
