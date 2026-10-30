import importlib.util
import json
from pathlib import Path
import subprocess
import tempfile
import unittest
from unittest.mock import patch

SPEC = importlib.util.spec_from_file_location('entry_accept', Path(__file__).with_name('resident_entry_accept.py'))
h = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(h)


class EntryTests(unittest.TestCase):
    def fixture(self, change=None):
        temp = tempfile.TemporaryDirectory()
        self.addCleanup(temp.cleanup)
        root = Path(temp.name)
        engine = root / 'engine'
        engine.write_bytes(b'fake-native-identity')
        output = root / 'out'

        def run(command, env, **kwargs):
            argument = command[1]
            stdout, stderr, code = b'', b'', 1
            home = Path(env['JR_ENGINE_HOME']) if 'JR_ENGINE_HOME' in env else None
            if argument == '--version':
                code = 0
                stdout = f'jr-engine experimental (Blaze-derived origin {h.ORIGIN}; build resident-v1-{"a" * 64})\n'.encode()
            elif argument == 'enable':
                stderr = b'does not support management'
            elif home and home.name == 'private':
                code = 0
                for part in ('resident-v1', 'resident-v1/runtime', 'resident-v1/store', 'resident-v1/functions', 'resident-v1/store/build'):
                    (home / part).mkdir(mode=0o700, exist_ok=True)
            result = subprocess.CompletedProcess(command, code, stdout, stderr)
            if change:
                change(command, env, result, engine, output)
            return result
        return engine, output, run

    def test_all_cases_and_receipt(self):
        engine, out, fake = self.fixture()
        with patch.object(h.subprocess, 'run', side_effect=fake):
            report = h.execute(engine, out)
        self.assertEqual(report['status'], 'passed')
        self.assertEqual(len(report['rows']), 8)
        self.assertEqual(report['sha256_before'], report['sha256_after'])
        self.assertEqual(json.loads((out/'acceptance.json').read_text()), report)

    def rejected(self, change, message):
        engine, out, fake = self.fixture(change)
        with patch.object(h.subprocess, 'run', side_effect=fake):
            with self.assertRaisesRegex(ValueError, message):
                h.execute(engine, out)
        self.assertEqual(json.loads((out/'acceptance.json').read_text())['status'], 'failed')

    def test_missing_home_must_fail(self):
        def change(command, env, result, *args):
            if command[1] == 'status' and 'JR_ENGINE_HOME' not in env:
                result.returncode = 0
        self.rejected(change, 'missing home accepted')

    def test_unknown_identity_fails_closed(self):
        def change(command, env, result, *args):
            if command[1] == '--version':
                result.stdout = b'jr-engine experimental (build unknown)'
        self.rejected(change, 'missing exact origin')

    def test_hostile_alias_must_not_write(self):
        def change(command, env, result, engine, out):
            if env.get('BLAZE_REMOTE'):
                (out/'must-not-exist').mkdir(exist_ok=True)
        self.rejected(change, 'inherited aliases redirected')

    def test_private_permissions_audited(self):
        def change(command, env, result, engine, out):
            if env.get('BLAZE_REMOTE'):
                (out/'private/resident-v1/functions').chmod(0o755)
        self.rejected(change, 'group/other accessible')

    def test_executable_mutation_rejected(self):
        def change(command, env, result, engine, out):
            if command[1] == 'enable':
                engine.write_bytes(b'changed')
        self.rejected(change, 'engine changed')


if __name__ == '__main__':
    unittest.main()
