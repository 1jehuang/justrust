"""Synthetic runner tests. These do not replace actual QEMU acceptance."""
import importlib.util
from pathlib import Path
import stat
import subprocess
import tempfile
import time
import sys
import unittest

spec = importlib.util.spec_from_file_location('capsule', Path(__file__).with_name('run.py'))
capsule = importlib.util.module_from_spec(spec)
spec.loader.exec_module(capsule)


def transcript():
    return '\n'.join([
        'CAPSULE RESULT check 0', 'CAPSULE RESULT build 0', 'CAPSULE RESULT run 0',
        'CAPSULE RESULT invalid-check 1', 'CAPSULE RESULT invalid-build 1',
        'CAPSULE RESULT missing-source 1', 'CAPSULE RESULT existing-output 1',
        'E0308: mismatched types:', 'E0308: mismatched types:',
        'CAPSULE ELF 7f454c46', 'CAPSULE DONE PASS'])


class RunnerTests(unittest.TestCase):
    def test_archive_order_and_metadata(self):
        a = [('b', stat.S_IFREG | 0o644, b'B'), ('a', stat.S_IFDIR | 0o755, b'')]
        self.assertEqual(capsule.newc(a), capsule.newc(list(reversed(a))))
        archive = capsule.newc(a)
        self.assertEqual(len(archive) % 512, 0)
        self.assertEqual(archive[:6], b'070701')
        self.assertEqual(archive[22:54], b'00000000000000000000000100000000')

    def test_parse_complete(self):
        results, elf = capsule.parse_guest(transcript())
        self.assertEqual(results['build'], 0)
        self.assertEqual(elf, b'\x7fELF')

    def test_false_success_rejected(self):
        for malformed in [
            '', transcript().replace('CAPSULE DONE PASS', 'CAPSULE DONE FAIL'),
            transcript().replace('RESULT invalid-build 1', 'RESULT invalid-build 0'),
            transcript().replace('RESULT build 0', 'RESULT build 1'),
            transcript().replace('RESULT missing-source 1', 'RESULT missing-source 127'),
            transcript().replace('CAPSULE ELF 7f454c46', 'CAPSULE ELF 0000'),
            transcript() + '\nCAPSULE RESULT check 0',
            transcript().replace('E0308: mismatched types:', 'infrastructure failure'),
            transcript().replace('CAPSULE RESULT run 0', ''),
        ]:
            with self.subTest(malformed=malformed):
                with self.assertRaises(ValueError):
                    capsule.parse_guest(malformed)

    def test_opt_in_required(self):
        with tempfile.TemporaryDirectory() as directory:
            output = Path(directory) / 'must-not-exist'
            result = subprocess.run([sys.executable, str(Path(__file__).with_name('run.py')),
                                     '--output', str(output)], capture_output=True, timeout=5)
            self.assertEqual(result.returncode, 2)
            self.assertFalse(output.exists())

    def test_nonzero_preserved(self):
        result = capsule.bounded([sys.executable, '-c', 'raise SystemExit(7)'], 5)
        self.assertEqual(result['returncode'], 7)

    def test_wall_timeout(self):
        with self.assertRaisesRegex(RuntimeError, 'timeout'):
            capsule.bounded([sys.executable, '-c', 'import time; time.sleep(5)'], 0.1)

    def test_parent_death_kills_child(self):
        with tempfile.TemporaryDirectory() as directory:
            pidfile = Path(directory) / 'child.pid'
            child = ('import os,time; from pathlib import Path; '
                     f'Path({str(pidfile)!r}).write_text(str(os.getpid())); time.sleep(30)')
            parent = (f'import sys,os,signal; sys.path.insert(0, {str(Path(__file__).parent)!r}); '
                      'import run; signal.signal(signal.SIGALRM, lambda *_: os._exit(0)); '
                      f'signal.alarm(1); run.bounded([sys.executable,"-c",{child!r}], 10)')
            subprocess.run([sys.executable, '-c', parent], check=True, timeout=5)
            pid = int(pidfile.read_text())
            for _ in range(20):
                proc = Path(f'/proc/{pid}/stat')
                if not proc.exists() or proc.read_text().split()[2] == 'Z':
                    break
                time.sleep(0.05)
            else:
                self.fail('direct child survived parent death')

    def test_log_bound(self):
        with self.assertRaisesRegex(RuntimeError, 'output limit'):
            capsule.bounded([sys.executable, '-c', 'print("x" * 5000000)'], 5)


if __name__ == '__main__':
    unittest.main()
