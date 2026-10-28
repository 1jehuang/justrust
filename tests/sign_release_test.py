"""Synthetic header fixtures and mocked signing only. No keys or native signers used."""
import contextlib
import hashlib
import importlib.util
import io
import os
from pathlib import Path
import struct
import subprocess
import tempfile
import unittest
from unittest.mock import patch

SPEC = importlib.util.spec_from_file_location(
    'sign_release', Path(__file__).resolve().parents[1] / 'scripts/sign-release.py')
signing = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(signing)


class SigningTests(unittest.TestCase):
    def setUp(self):
        self.tmp = tempfile.TemporaryDirectory()
        self.addCleanup(self.tmp.cleanup)
        self.root = Path(self.tmp.name)
        self.source = self.root / 'input'
        self.output = self.root / 'signed'
        self.source.write_bytes(struct.pack('<IIIIIIII', 0xFEEDFACF, 0x0100000C,
                                            0, 2, 0, 0, 0, 0))
        self.original = self.source.read_bytes()

    def args(self, *extra):
        return signing.parser().parse_args([
            '--input', str(self.source), '--output', str(self.output),
            '--sha256', hashlib.sha256(self.source.read_bytes()).hexdigest(),
            '--target', 'aarch64-apple-darwin',
            '--identity', 'Developer ID Application: Example (TESTTEAM)', *extra])

    def invoke(self, a):
        with contextlib.redirect_stdout(io.StringIO()):
            signing.run(a)

    def clean(self):
        self.assertFalse(self.output.exists())
        self.assertEqual(self.source.read_bytes(), self.original)
        self.assertEqual(sorted(p.name for p in self.root.iterdir()), ['input'])

    @patch.object(signing.subprocess, 'run')
    def test_default_plan_never_runs_or_mutates(self, run):
        self.invoke(self.args())
        run.assert_not_called()
        self.clean()

    @patch.object(signing.subprocess, 'run')
    def test_refusals_before_native_invocation(self, run):
        for change in [dict(sha256='0' * 64), dict(sha256='bad'),
                       dict(identity='-'), dict(identity='Apple Development: Fake'),
                       dict(target='x86_64-apple-darwin'), dict(identifier='bad" or true')]:
            with self.subTest(change=change):
                a = self.args('--execute')
                for key, value in change.items():
                    setattr(a, key, value)
                with self.assertRaises(ValueError):
                    self.invoke(a)
                self.clean()
        run.assert_not_called()

    @patch.object(signing.subprocess, 'run')
    @patch.object(signing.platform, 'system', return_value='Linux')
    def test_host_mismatch(self, system, run):
        with self.assertRaisesRegex(ValueError, 'operating system'):
            self.invoke(self.args('--execute'))
        run.assert_not_called()
        self.clean()

    @patch.object(signing.platform, 'system', return_value='Darwin')
    def test_success_copy_verify_and_scrubbed_env(self, system):
        calls = []

        def native(command, **kwargs):
            calls.append(command)
            self.assertFalse(self.output.exists())
            self.assertFalse(kwargs['shell'])
            self.assertNotIn('AWS_SECRET_ACCESS_KEY', kwargs['env'])
            self.assertNotIn('CLOUDFLARE_API_TOKEN', kwargs['env'])
            artifact = Path(command[-1])
            self.assertNotEqual(artifact, self.source)
            if '--sign' in command:
                artifact.write_bytes(artifact.read_bytes() + b'mock-signature')
            else:
                self.assertIn('--verify', command)
                self.assertIn('com.jcode.cli', command[-2])
                self.assertTrue(artifact.read_bytes().endswith(b'mock-signature'))

        with patch.dict(os.environ, {'AWS_SECRET_ACCESS_KEY': 'never-pass',
                                    'CLOUDFLARE_API_TOKEN': 'never-pass'}), \
                patch.object(signing.subprocess, 'run', side_effect=native):
            self.invoke(self.args('--execute'))
        self.assertEqual(len(calls), 2)
        self.assertIn('--timestamp', calls[0])
        self.assertIn('runtime', calls[0])
        self.assertEqual(self.output.read_bytes(), self.original + b'mock-signature')
        self.assertEqual(self.source.read_bytes(), self.original)
        self.assertEqual(sorted(p.name for p in self.root.iterdir()), ['input', 'signed'])

    @patch.object(signing.platform, 'system', return_value='Darwin')
    def test_sign_verify_timeout_and_launch_failure_cleanup(self, system):
        for effects in [
            [subprocess.CalledProcessError(1, ['mock'])],
            [None, subprocess.CalledProcessError(1, ['mock'])],
            [subprocess.TimeoutExpired(['mock'], 300)],
            [FileNotFoundError('mock signer unavailable')],
        ]:
            with self.subTest(effects=effects), \
                    patch.object(signing.subprocess, 'run', side_effect=effects):
                with self.assertRaises((OSError, subprocess.SubprocessError)):
                    self.invoke(self.args('--execute'))
                self.clean()

    def test_existing_output_and_same_path_refused(self):
        self.output.write_bytes(b'keep')
        with self.assertRaises(ValueError):
            self.invoke(self.args())
        self.assertEqual(self.output.read_bytes(), b'keep')
        a = self.args()
        a.output = self.source
        with self.assertRaises(ValueError):
            self.invoke(a)
        self.assertEqual(self.source.read_bytes(), self.original)

    @patch.object(signing.platform, 'system', return_value='Darwin')
    @patch.object(signing.subprocess, 'run')
    def test_atomic_publish_refuses_concurrent_destination(self, run, system):
        def verify_then_race(*args, **kwargs):
            if '--verify' in args[0]:
                self.output.write_bytes(b'concurrent owner')
        run.side_effect = verify_then_race
        with self.assertRaises(FileExistsError):
            self.invoke(self.args('--execute'))
        self.assertEqual(self.output.read_bytes(), b'concurrent owner')
        self.assertEqual(sorted(p.name for p in self.root.iterdir()), ['input', 'signed'])

    @patch.object(signing.platform, 'system', return_value='Darwin')
    @patch.object(signing.subprocess, 'run')
    def test_copy_race_is_hash_checked(self, run, system):
        original_copy = signing.shutil.copyfile
        def altered(src, dst):
            original_copy(src, dst)
            Path(dst).write_bytes(b'changed')
        with patch.object(signing.shutil, 'copyfile', side_effect=altered):
            with self.assertRaisesRegex(ValueError, 'copied input SHA256'):
                self.invoke(self.args('--execute'))
        run.assert_not_called()
        self.clean()

    def windows_args(self):
        data = bytearray(128)
        data[:2] = b'MZ'
        struct.pack_into('<I', data, 60, 64)
        data[64:68] = b'PE\0\0'
        struct.pack_into('<H', data, 68, 0x8664)
        struct.pack_into('<H', data, 86, 2)
        struct.pack_into('<H', data, 88, 0x20B)
        self.source.write_bytes(data)
        a = self.args('--execute')
        a.target = 'x86_64-pc-windows-msvc'
        a.identity = None
        a.thumbprint = 'AB' * 20
        a.signtool = self.root / 'signtool.exe'
        return a

    @patch.object(signing.platform, 'system', return_value='Windows')
    @patch.object(signing.subprocess, 'run')
    def test_windows_store_signing_arguments(self, run, system):
        a = self.windows_args()
        self.invoke(a)
        sign, verify = [call.args[0] for call in run.call_args_list]
        self.assertEqual(sign[:6], [str(a.signtool), 'sign', '/s', 'My', '/sha1', a.thumbprint])
        self.assertIn('/fd', sign)
        self.assertIn('/td', sign)
        self.assertEqual(verify[1:5], ['verify', '/pa', '/all', '/v'])
        self.assertNotIn('/p', sign)
        self.assertNotIn('/f', sign)

    def test_macos_requirement_uses_inline_mode_and_preserves_checks(self):
        for identity in ['Developer ID Application: Example (TESTTEAM)', 'AB' * 20]:
            with self.subTest(identity=identity):
                a = self.args('--identifier', 'com.example.signed-cli')
                a.identity = identity
                sign, verify = signing.commands(a, '<artifact>')
                self.assertEqual(sign[sign.index('--sign') + 1], identity)
                self.assertEqual(verify, [
                    '/usr/bin/codesign', '--verify', '--strict', '--verbose=2', '-R',
                    '=anchor apple generic and certificate 1[field.1.2.840.113635.100.6.2.6] exists '
                    'and certificate leaf[field.1.2.840.113635.100.6.1.13] exists '
                    'and identifier "com.example.signed-cli"', '<artifact>',
                ])

    def test_fingerprint_keeps_developer_id_chain_requirement(self):
        a = self.args()
        a.identity = 'AB' * 20
        sign, verify = signing.commands(a, '<artifact>')
        self.assertEqual(sign[sign.index('--sign') + 1], a.identity)
        requirement = verify[verify.index('-R') + 1]
        self.assertIn('anchor apple generic', requirement)
        self.assertIn('1.2.840.113635.100.6.2.6', requirement)
        self.assertIn('1.2.840.113635.100.6.1.13', requirement)
        for bad in ['AB' * 32, '-', 'password', '/tmp/key.p12']:
            a.identity = bad
            with self.assertRaises(ValueError):
                signing.commands(a, '<artifact>')

    def test_fingerprint_signing_still_refuses_failed_chain_verification(self):
        a = self.args('--execute')
        a.identity = 'AB' * 20
        with patch.object(signing.platform, 'system', return_value='Darwin'), \
                patch.object(signing.subprocess, 'run', side_effect=[
                    None, subprocess.CalledProcessError(1, ['codesign', '--verify'])]):
            with self.assertRaises(subprocess.CalledProcessError):
                self.invoke(a)
        self.clean()

    def test_abbreviated_identity_is_not_accepted(self):
        with contextlib.redirect_stderr(io.StringIO()):
            with self.assertRaises(SystemExit):
                self.args('--iden', 'AB' * 20)

    def test_windows_bad_thumbprint_tool_and_header(self):
        a = self.windows_args()
        for field, bad in [('thumbprint', 'bad'), ('thumbprint', 'AB' * 32), ('signtool', Path('relative.exe')),
                           ('timestamp_url', 'https://user:password@example.com')]:
            b = signing.argparse.Namespace(**vars(a))
            setattr(b, field, bad)
            with self.assertRaises(ValueError):
                signing.commands(b, self.source)
        self.source.write_bytes(b'MZ')
        with self.assertRaises(ValueError):
            signing.validate_header(self.source, a.target)

    @patch.object(signing.platform, 'system', return_value='Darwin')
    @patch.object(signing.subprocess, 'run')
    def test_cleanup_failure_rolls_back_published_output(self, run, system):
        original_exit = signing.tempfile.TemporaryDirectory.__exit__

        def failing_exit(directory, *args):
            original_exit(directory, *args)
            raise OSError('simulated cleanup failure')

        with patch.object(signing.tempfile.TemporaryDirectory, '__exit__', failing_exit):
            with self.assertRaisesRegex(OSError, 'cleanup failure'):
                self.invoke(self.args('--execute'))
        self.clean()

    def test_hash_argument_is_required(self):
        with contextlib.redirect_stderr(io.StringIO()):
            with self.assertRaises(SystemExit) as exit_info:
                signing.parser().parse_args(['--input', 'in', '--output', 'out',
                                             '--target', 'aarch64-apple-darwin'])
        self.assertNotEqual(exit_info.exception.code, 0)

    def test_unsupported_headers(self):
        for data in [b'', b'\x7fELF' + bytes(60), b'\xca\xfe\xba\xbe' + bytes(60)]:
            self.source.write_bytes(data)
            with self.assertRaises(ValueError):
                signing.validate_header(self.source, 'aarch64-apple-darwin')


if __name__ == '__main__':
    unittest.main()
