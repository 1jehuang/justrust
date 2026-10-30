#!/usr/bin/env python3
"""Offline mutation controls on private scratch copies. No native workloads."""
import hashlib
import json
import os
import shutil
import tempfile
import unittest
from pathlib import Path

from validate import validate

HERE = Path(__file__).resolve().parent


class AdmissionControls(unittest.TestCase):
    def setUp(self):
        scratch = Path(os.environ.get('JCODE_SCRATCH_DIR', str(Path.home() / '.jcode/scratch')))
        scratch.mkdir(parents=True, exist_ok=True)
        self.temp = tempfile.TemporaryDirectory(prefix='collector-v26-audit-', dir=scratch)
        self.root = Path(self.temp.name) / 'bundle'
        shutil.copytree(HERE, self.root, ignore=shutil.ignore_patterns('__pycache__'))

    def tearDown(self):
        self.temp.cleanup()

    def rehash(self):
        rows = []
        for path in sorted(self.root.rglob('*')):
            if path.is_file() and path.name not in ('SHA256SUMS', 'validation.json'):
                rows.append(f'{hashlib.sha256(path.read_bytes()).hexdigest()}  {path.relative_to(self.root)}\n')
        (self.root / 'SHA256SUMS').write_text(''.join(rows))

    def change(self, relative, modify):
        path = self.root / relative
        value = json.loads(path.read_text())
        modify(value)
        path.write_text(json.dumps(value) + '\n')
        self.rehash()

    def test_accepts_complete_copied_bundle(self):
        self.assertEqual(validate(self.root)['native_tests'], 351)

    def test_rejects_payload_corruption(self):
        (self.root / 'native/full-native.exit').write_text('1\n')
        with self.assertRaisesRegex(ValueError, 'hash mismatch'):
            validate(self.root)

    def test_rejects_failed_gate_even_after_rehash(self):
        (self.root / 'native/full-native.exit').write_text('1\n')
        self.rehash()
        with self.assertRaisesRegex(ValueError, 'failed gate'):
            validate(self.root)

    def test_rejects_changed_private_snapshot_even_after_rehash(self):
        self.change('native/source-copy-after.json', lambda s: s['selected'].update({'Cargo.lock': '0' * 64}))
        with self.assertRaisesRegex(ValueError, 'private snapshot changed'):
            validate(self.root)

    def test_rejects_nonlock_source_delta_even_after_rehash(self):
        for name in ('before', 'after'):
            self.change(f'native/source-copy-{name}.json', lambda s: s['selected'].update({'Cargo.toml': '0' * 64}))
        with self.assertRaisesRegex(ValueError, 'non-lock private source delta'):
            validate(self.root)

    def test_rejects_invented_inventory_even_after_rehash(self):
        self.change('profile/state.json', lambda s: s['rows'][0]['collector_inventory'][0].update({'calls': 999999}))
        with self.assertRaisesRegex(ValueError, 'raw inventory mismatch'):
            validate(self.root)

    def test_rejects_drops_even_after_rehash(self):
        self.change('profile/state.json', lambda s: s['rows'][0].update({'transport': [[17, 0, 1]]}))
        with self.assertRaisesRegex(ValueError, 'raw transport accounting mismatch'):
            validate(self.root)

    def test_rejects_changed_native_count_even_after_rehash(self):
        self.change('native/positive-test-admission.json', lambda s: s['native_test_rows'][0].update({'passed': 0}))
        with self.assertRaisesRegex(ValueError, 'native gate count mismatch'):
            validate(self.root)

    def test_rejects_disabled_runtime_with_telemetry_even_after_rehash(self):
        self.change('disabled-runtime/rsa23/metadata.json', lambda s: s['environments']['engine'].update({'RA_JIT_DIAGNOSTICS_STATS': '1'}))
        with self.assertRaisesRegex(ValueError, 'telemetry enabled'):
            validate(self.root)


if __name__ == '__main__':
    unittest.main(verbosity=2)
