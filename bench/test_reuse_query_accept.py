import importlib.util
import json
from pathlib import Path
import subprocess
import tempfile
import unittest
from unittest.mock import patch

spec = importlib.util.spec_from_file_location('query_accept', Path(__file__).with_name('reuse-query-accept.py'))
accept = importlib.util.module_from_spec(spec)
spec.loader.exec_module(accept)


class AcceptanceTests(unittest.TestCase):
    def test_only_source_directory_is_normalized(self):
        off = dict(status=1, stdout='', stderr='/a/main.rs:2: error E123 hash abc\n')
        on = dict(status=1, stdout='', stderr='/b/main.rs:2: error E123 hash abc\n')
        accept.compare(off, on, Path('/a'), Path('/b'), 'same')
        for field, value in [('status', 2), ('stderr', '/b/main.rs:2: error E123 hash def\n'), ('stdout', 'extra')]:
            changed = dict(on, **{field: value})
            with self.assertRaises(AssertionError):
                accept.compare(off, changed, Path('/a'), Path('/b'), 'changed')

    def test_existing_output_is_untouched(self):
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            sentinel = root / 'acceptance.json'
            sentinel.write_text('keep')
            with self.assertRaises(FileExistsError):
                accept.capture(root / 'missing', root)
            self.assertEqual(sentinel.read_text(), 'keep')

    def test_failure_report_survives(self):
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp) / 'capture'
            compiler = Path(tmp) / 'rustc'
            compiler.write_text('identity')
            failed = dict(command=['rustc'], status=1, stdout='', stderr='complete compiler error\n')
            with patch.object(accept, 'run', return_value=failed):
                with self.assertRaisesRegex(AssertionError, 'complete compiler error'):
                    accept.capture(compiler, root)
            report = json.loads((root / 'acceptance.json').read_text())
            self.assertEqual(report['verdict'], 'failed')
            self.assertEqual(report['completed_cases'], 0)
            self.assertEqual(report['compiler']['version']['stderr'], failed['stderr'])
            self.assertEqual(root.stat().st_mode & 0o777, 0o700)

    def test_timeout_keeps_captured_errors(self):
        failure = subprocess.TimeoutExpired(['rustc'], 180, output=b'partial output', stderr=b'full error so far')
        with patch.object(accept.subprocess, 'run', side_effect=failure):
            with self.assertRaisesRegex(RuntimeError, 'full error so far'):
                accept.run(['rustc'])

    def test_case_inventory(self):
        cases = accept.cases()
        self.assertEqual(len(cases), 22)
        self.assertEqual(len({c[0] for c in cases}), len(cases))
        self.assertEqual(sum(c[3] is None for c in cases), 3)
        self.assertEqual(cases[0][1:], cases[1][1:])
        self.assertEqual(cases[0][1:], cases[-1][1:])

    def test_synthetic_capture_protocol(self):
        with tempfile.TemporaryDirectory() as tmp:
            tmp = Path(tmp)
            compiler = tmp / 'rustc'
            compiler.write_text('compiler')
            lib = tmp / 'lib'
            lib.mkdir()
            (lib / 'librustc_driver-test.so').write_text('driver')
            calls = []

            def fake_run(command):
                command = [str(x) for x in command]
                calls.append(command)
                result = dict(command=command, status=0, stdout='', stderr='')
                if command[1:] == ['-vV']:
                    result['stdout'] = 'test compiler\n'
                elif command[1:] == ['--print', 'sysroot']:
                    result['stdout'] = str(tmp) + '\n'
                elif len(command) == 1:
                    result['stdout'] = '15\n'
                else:
                    self.assertIn('-Zincremental-verify-ich', command)
                    self.assertEqual(accept.FLAG in command, '/on/' in command[1])
                    Path(command[command.index('-o') + 1]).write_text('artifact')
                return result

            with patch.object(accept, 'run', side_effect=fake_run), patch.object(accept, 'cases', return_value=[accept.cases()[0]]):
                report = accept.capture(compiler, tmp / 'capture')
            self.assertEqual(report['completed_cases'], 1)
            self.assertEqual(report['verdict'], 'passed')
            self.assertEqual(len(report['drivers'][0]['sha256']), 64)
            increments = [c[c.index('-C') + 1] for c in calls if '-C' in c]
            self.assertEqual(len(set(increments)), 4)


if __name__ == '__main__':
    unittest.main()
