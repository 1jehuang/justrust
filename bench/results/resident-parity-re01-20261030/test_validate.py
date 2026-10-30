#!/usr/bin/env python3
"""Offline mutations exercise semantic gates with the integrity seal disabled."""
import json
import os
from pathlib import Path
import shutil
import tempfile
import unittest
import validate as audit

class MutationTests(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        # Scratch is optional, no particular checkout or private path is required.
        cls.temp = tempfile.TemporaryDirectory(dir=os.environ.get('JCODE_SCRATCH_DIR'))
        cls.root = Path(cls.temp.name) / 'portable-copy'
        shutil.copytree(audit.ROOT, cls.root, ignore=shutil.ignore_patterns('__pycache__'))

    @classmethod
    def tearDownClass(cls):
        cls.temp.cleanup()

    def mutate(self, name, change, message):
        path = self.root / name
        original = path.read_bytes()
        try:
            path.write_bytes(change(original))
            with self.assertRaisesRegex(ValueError, message):
                audit.validate(self.root, integrity=False)
        finally:
            path.write_bytes(original)

    def json_mutate(self, name, change, message):
        def apply(data):
            obj = json.loads(data)
            change(obj)
            return json.dumps(obj).encode()
        self.mutate(name, apply, message)

    def test_portable_copy(self):
        self.assertEqual(audit.validate(self.root)['accepted_runtime_edits'], 10)

    def test_all_ten_markers(self):
        for stack in ('reference', 'engine'):
            for rep in range(5):
                name = f'runtime/{stack}-edit-{rep}.stdout'
                for replacement in (b'', b'JR_RUNTIME_EXECUTED_99', f'JR_RUNTIME_EXECUTED_{rep+1} JR_RUNTIME_EXECUTED_{rep+1}'.encode()):
                    with self.subTest(stack=stack, rep=rep, replacement=replacement):
                        self.mutate(name, lambda d, v=replacement: d.replace(f'JR_RUNTIME_EXECUTED_{rep+1}'.encode(), v), 'marker')

    def test_actual_names_and_counts(self):
        for stack in ('reference', 'engine'):
            name = f'runtime/{stack}-edit-0.stdout'
            self.mutate(name, lambda d: d.replace(b'parses_libtest_results', b'wrong_test'), 'names')
            self.mutate(name, lambda d: d.replace(b'4 passed', b'5 passed'), 'names')

    def test_failure_recovery_both(self):
        for stack in ('reference', 'engine'):
            self.mutate(f'runtime/{stack}-type-error-0.stderr', lambda d: d.replace(b'E0308', b'E9999'), 'type failure')
            self.mutate(f'runtime/{stack}-assert-failure-0.stderr', lambda d: d.replace(b'JR_RUNTIME_FAILURE', b'no marker'), 'assertion failure')
            for phase in ('type-recovery', 'assert-recovery'):
                self.mutate(f'runtime/{stack}-{phase}-0.stdout', lambda d: d.replace(b'4 passed', b'0 passed'), 'names')

    def test_identities_policy_and_build(self):
        self.mutate('source/src/summary.rs', lambda d: d + b'\n', 'pristine')
        self.json_mutate('runtime/metadata.json', lambda d: d['tools_after']['engine'].update(sha256='0'*64), 'identity changed')
        self.json_mutate('runtime/metadata.json', lambda d: d['toolchain_after']['files']['bin/rustc'].update(sha256='0'*64), 'identity changed')
        self.json_mutate('runtime/metadata.json', lambda d: d.update(store_scope='seeded'), 'no seed')
        self.json_mutate('runtime/records.json', lambda d: d[0]['command'].append('seed'), 'seed command')
        self.mutate('build/candidate-build-v2/compile-exit', lambda d: b'1\n', 'compile/runtime')
        self.mutate('build/baseline-build/exit', lambda d: b'1\n', 'compiler build')
        self.mutate('build/candidate-build-v2/engine-identity.sha256', lambda d: b'wrong\n', 'binary identity')
        self.mutate('provenance/channel-manifest.toml', lambda d: d + b'\n', 'toolchain manifest')
        self.json_mutate('runtime/records.json', lambda d: d.pop(), 'row count')

    def test_namespace_all_eight(self):
        for i in range(8):
            self.json_mutate('entry/acceptance.json', lambda d, i=i: d['rows'][i].update(status=42), 'namespace status')
        self.json_mutate('entry/acceptance.json', lambda d: d['rows'].pop(), 'eight native')
        self.mutate('entry/no-home.stderr', lambda d: b'accepted\n', 'namespace raw')

    def test_seal_catches_unowned_extra(self):
        path = self.root / 'unexpected.txt'
        try:
            path.write_text('not allowlisted')
            with self.assertRaisesRegex(ValueError, 'integrity seal'):
                audit.validate(self.root)
        finally:
            path.unlink()

if __name__ == '__main__':
    unittest.main()
