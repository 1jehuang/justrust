#!/usr/bin/env python3
"""Test the installed launcher's branch guard without opening Jcode or a terminal."""
import os
from pathlib import Path
import subprocess
import tempfile
import unittest

LAUNCHER = Path(__file__).with_name('resource-lab-launch.sh').resolve()


class ResourceLabLauncherTests(unittest.TestCase):
    def setUp(self):
        self.temporary = tempfile.TemporaryDirectory()
        self.addCleanup(self.temporary.cleanup)
        self.root = Path(self.temporary.name)
        self.repo = self.root / 'checkout with spaces'
        self.repo.mkdir()
        (self.repo / '.git').mkdir()
        self.bin = self.root / 'bin'
        self.bin.mkdir()
        self.fake_git = self.bin / 'git'
        self.fake_git.write_text('#!/bin/sh\nprintf "%s\\n" "$TEST_BRANCH"\nexit "${TEST_GIT_STATUS:-0}"\n')
        self.fake_git.chmod(0o755)
        self.env = dict(os.environ, PATH=str(self.bin) + os.pathsep + os.environ['PATH'],
                        JUSTRUST_RESOURCE_LAB_DIR=str(self.repo), JCODE_BIN='/usr/bin/pwd',
                        TEST_BRANCH='experiment/resource-sharing')

    def run_launcher(self, *args):
        return subprocess.run(['bash', str(LAUNCHER), *args], env=self.env,
                              text=True, capture_output=True, check=False)

    def test_opens_expected_checkout_with_spaces(self):
        result = self.run_launcher()
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertEqual(result.stdout.splitlines()[-1], str(self.repo))

    def test_refuses_shipping_branch(self):
        self.env['TEST_BRANCH'] = 'stable'
        result = self.run_launcher()
        self.assertNotEqual(result.returncode, 0)
        self.assertIn('Refusing', result.stderr)

    def test_refuses_missing_checkout(self):
        self.env['JUSTRUST_RESOURCE_LAB_DIR'] = str(self.root / 'missing')
        result = self.run_launcher()
        self.assertNotEqual(result.returncode, 0)
        self.assertIn('missing', result.stderr)

    def test_refuses_detached_head(self):
        self.env.update(TEST_BRANCH='', TEST_GIT_STATUS='1')
        self.assertNotEqual(self.run_launcher().returncode, 0)

    def test_passes_arguments_unchanged(self):
        executable = self.bin / 'fake-jcode'
        executable.write_text('#!/bin/sh\nprintf "ARG:%s\\n" "$@"\n')
        executable.chmod(0o755)
        self.env['JCODE_BIN'] = str(executable)
        result = self.run_launcher('--model', 'model with spaces')
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertEqual(result.stdout.splitlines()[-2:], ['ARG:--model', 'ARG:model with spaces'])


if __name__ == '__main__':
    unittest.main()
