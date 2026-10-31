#!/usr/bin/env python3
"""Positive, coherently re-signed semantic controls and detached offline replay."""
import json
import os
from pathlib import Path
import shutil
import subprocess
import sys
import tempfile
import unittest
import validate as v

ROOT = Path(__file__).resolve().parent


def signed(changes):
    inventory = json.loads((ROOT / 'manifest.json').read_bytes())
    for name, raw in changes.items():
        inventory[name] = v.sha(raw)
    return v.Evidence(ROOT, {**changes, 'manifest.json': json.dumps(inventory).encode()})


def state_change(edit):
    state = json.loads((ROOT / 'raw/state.json').read_bytes())
    edit(state)
    return signed({'raw/state.json': json.dumps(state).encode()})


class ColdProfileControls(unittest.TestCase):
    def test_positive_replay(self):
        result = v.audit()
        self.assertEqual(result['actual_cold_calls'], 2)
        self.assertFalse(result['comparison'])
        self.assertFalse(result['cpu_phases_additive'])
        self.assertFalse(result['v31_validation_claimed'])

    def test_plain_corruption(self):
        with self.assertRaisesRegex(ValueError, 'checksum'):
            v.audit(v.Evidence(ROOT, {'raw/engine.stdout': b'corrupt'}))

    def test_resigned_renamed_test_same_count(self):
        raw = (ROOT / 'raw/engine.stdout').read_bytes().replace(b'parses_libtest_results', b'fake_libtest_results')
        with self.assertRaisesRegex(ValueError, 'four named tests'):
            v.audit(signed({'raw/engine.stdout': raw}))

    def test_resigned_test_count(self):
        raw = (ROOT / 'raw/reference.stdout').read_bytes().replace(b'4 passed;', b'0 passed;')
        with self.assertRaisesRegex(ValueError, 'test result count'):
            v.audit(signed({'raw/reference.stdout': raw}))

    def test_resigned_binary_both_copies(self):
        def edit(s):
            for key in ('tools_before', 'tools_after'):
                s[key]['engine'] = '0' * 64
        with self.assertRaisesRegex(ValueError, 'immutable binary identity'):
            v.audit(state_change(edit))

    def test_resigned_toolchain_both_copies(self):
        def edit(s):
            for key in ('toolchain_before', 'toolchain_after'):
                s[key]['path'] = '/fake-toolchain'
        with self.assertRaisesRegex(ValueError, 'immutable toolchain identity'):
            v.audit(state_change(edit))

    def test_resigned_source_both_copies(self):
        def edit(s):
            for key in ('source_before', 'source_after'):
                s[key]['Cargo.toml'] = '0' * 64
        with self.assertRaisesRegex(ValueError, 'immutable source identity'):
            v.audit(state_change(edit))

    def test_resigned_failed_native_status(self):
        with self.assertRaisesRegex(ValueError, 'native call status'):
            v.audit(state_change(lambda s: s['rows'][1].update(status=101)))

    def test_resigned_empty_filter(self):
        with self.assertRaisesRegex(ValueError, 'exact test command'):
            v.audit(state_change(lambda s: s['rows'][1]['command'].__setitem__(4, 'fake::')))

    def test_resigned_quiet_machine_claim(self):
        with self.assertRaisesRegex(ValueError, 'noise limitations'):
            v.audit(state_change(lambda s: s['rows'][1]['noise'].update(contamination='quiet')))

    def test_resigned_std_root_count(self):
        raw = (ROOT / 'raw/engine-worker-0.log').read_bytes().replace(b'compiled 25924 of 25960', b'compiled 1 of 25960')
        with self.assertRaisesRegex(ValueError, 'standard-library counts'):
            v.audit(signed({'raw/engine-worker-0.log': raw}))

    def test_resigned_cpu_phase_time(self):
        raw = (ROOT / 'raw/engine-worker-0.log').read_bytes().replace(b'translate 80.4s', b'translate 0.0s')
        with self.assertRaisesRegex(ValueError, 'CPU phase receipt'):
            v.audit(signed({'raw/engine-worker-0.log': raw}))

    def test_resigned_client_time(self):
        raw = (ROOT / 'raw/engine.stderr').read_bytes().replace(b'26338.5', b'1.0')
        with self.assertRaisesRegex(ValueError, 'client timing receipt'):
            v.audit(signed({'raw/engine.stderr': raw}))

    def test_detached_relocation(self):
        scratch = os.environ.get('JCODE_SCRATCH_DIR')
        with tempfile.TemporaryDirectory(prefix='cold-profile-offline-', dir=scratch) as temporary:
            base = Path(temporary)
            copied = base / 'evidence'
            shutil.copytree(ROOT, copied, ignore=shutil.ignore_patterns('__pycache__'))
            home = base / 'empty-home'
            home.mkdir()
            result = subprocess.run([sys.executable, '-I', str(copied / 'validate.py')], cwd=home,
                env={'HOME': str(home), 'PATH': '', 'PYTHONDONTWRITEBYTECODE': '1'},
                capture_output=True, text=True, timeout=30)
            self.assertEqual(result.returncode, 0, result.stderr)
            self.assertEqual(json.loads(result.stdout)['actual_tests_per_call'], 4)


if __name__ == '__main__':
    unittest.main(verbosity=2)
