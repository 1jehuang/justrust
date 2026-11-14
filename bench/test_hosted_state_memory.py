"""Local protocol checks for the hosted read-only byte/codec experiment."""
import ctypes.util
import hashlib
import json
import os
from pathlib import Path
import subprocess
import sys
import tempfile
import unittest

from bench.hosted_state_memory import REMOTE


@unittest.skipUnless(ctypes.util.find_library('zstd') and hasattr(os, 'posix_fadvise'),
                     'requires zstd and POSIX cache advice')
class HostedStateMemoryTests(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory(prefix='disk-state-memory-tests-')
        self.addCleanup(self.temp.cleanup)
        self.source = Path(self.temp.name) / 'state-snapshot'
        self.source.mkdir()
        (self.source / 'query-cache.bin').write_bytes(bytes(range(256)) * 8)
        (self.source / 'dep-graph.bin').write_bytes(b'graph-state' * 300)

    def run_protocol(self, source=None):
        program = ('import sys\nfrom bench import compressed_state_restore as codec\n'
                   "sys.modules['compressed_state_restore']=codec\n"
                   f'SOURCE={str(source or self.source)!r}\n' + REMOTE)
        return subprocess.run([sys.executable, '-c', program], text=True,
                              capture_output=True, timeout=30)

    def test_verified_read_and_codec_cycles_preserve_input(self):
        before = {p.name: hashlib.sha256(p.read_bytes()).hexdigest() for p in self.source.iterdir()}
        result = self.run_protocol()
        self.assertEqual(result.returncode, 0, result.stderr)
        report = json.loads(result.stdout.split('DISK_STATE_JSON=', 1)[1])
        self.assertEqual(len(report['rows']), 27)
        self.assertEqual(len(report['summaries']), 9)
        self.assertEqual(report['bytes'], 5348)
        self.assertTrue(all(row['verified'] for row in report['rows']))
        self.assertTrue(all(row['seconds'] >= 0 for row in report['rows']))
        self.assertGreater(report['compression']['bytes'], 0)
        self.assertIn('RAM', report['scope'])
        self.assertEqual(before, {p.name: hashlib.sha256(p.read_bytes()).hexdigest()
                                  for p in self.source.iterdir()})
        self.assertEqual(list(Path(self.temp.name).iterdir()), [self.source])

    def test_unsafe_missing_empty_and_symlink_inputs_fail(self):
        unsafe = Path(self.temp.name) / 'other'
        result = self.run_protocol(unsafe)
        self.assertNotEqual(result.returncode, 0)
        self.assertIn('private immutable', result.stderr)
        missing = Path(self.temp.name) / 'disk-state-missing' / 'state-snapshot'
        result = self.run_protocol(missing)
        self.assertNotEqual(result.returncode, 0)
        self.assertIn('snapshot missing', result.stderr)
        for p in self.source.iterdir():
            p.unlink()
        result = self.run_protocol()
        self.assertNotEqual(result.returncode, 0)
        self.assertIn('snapshot empty', result.stderr)
        (self.source / 'link').symlink_to(Path(self.temp.name) / 'elsewhere')
        result = self.run_protocol()
        self.assertNotEqual(result.returncode, 0)
        self.assertIn('symlink', result.stderr)


if __name__ == '__main__':
    unittest.main()
