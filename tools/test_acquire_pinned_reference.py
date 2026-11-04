#!/usr/bin/env python3
"""Offline adversarial acquisition tests. Optional retained official archive proof."""
from contextlib import redirect_stdout
import gzip
import hashlib
import io
import json
import os
from pathlib import Path
import tarfile
import tempfile
import unittest
from unittest.mock import Mock, patch
import urllib.error

import acquire_pinned_reference as acquisition


class AcquireTests(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory(dir=os.environ.get('JCODE_SCRATCH_DIR'))
        self.addCleanup(self.temp.cleanup)
        self.base = Path(self.temp.name)
        self.output = self.base / 'new'
        self.archive = self.base / 'input.tar.gz'
        self.prefix = 'reference-' + acquisition._COMMIT
        self.raw = self.pack([(self.prefix + '/src/one.md', b'r[one]\n', tarfile.REGTYPE)])

    def pack(self, entries):
        data = io.BytesIO()
        with tarfile.open(fileobj=data, mode='w') as archive:
            for name, payload, kind in entries:
                member = tarfile.TarInfo(name)
                member.type = kind
                member.size = len(payload) if kind == tarfile.REGTYPE else 0
                member.linkname = 'target' if kind in (tarfile.SYMTYPE, tarfile.LNKTYPE) else ''
                archive.addfile(member, io.BytesIO(payload) if kind == tarfile.REGTYPE else None)
        return gzip.compress(data.getvalue(), mtime=0)

    def run_fixture(self, raw=None):
        raw = self.raw if raw is None else raw
        self.archive.write_bytes(raw)
        with patch.object(acquisition, '_SHA256', hashlib.sha256(raw).hexdigest()):
            return acquisition.acquire(self.output, self.archive)

    def reject_fixture(self, entries):
        with self.assertRaises((ValueError, tarfile.TarError, OSError)):
            self.run_fixture(self.pack(entries))
        self.assertFalse(self.output.exists())

    def test_offline_success_and_existing_verifier(self):
        with patch.object(acquisition, '_download', side_effect=AssertionError('network')):
            result = self.run_fixture()
        self.assertEqual(result['readiness'], 'input-verified')
        self.assertEqual(result['verified_files'], 1)
        self.assertFalse(result['compatibility_credit'])
        self.assertTrue(Path(result['source_root']).is_absolute())
        self.assertEqual(Path(result['archive_path']).read_bytes(), self.raw)
        self.assertEqual((Path(result['source_root']) / 'src/one.md').read_bytes(), b'r[one]\n')

    def test_digest_mismatch_before_mkdir(self):
        self.archive.write_bytes(self.raw)
        with self.assertRaisesRegex(ValueError, 'SHA256'), patch.object(acquisition.os, 'mkdir') as mkdir:
            acquisition.acquire(self.output, self.archive)
        mkdir.assert_not_called()

    def test_existing_output_rejected_before_io(self):
        for kind in ('directory', 'file', 'dangling-symlink'):
            with self.subTest(kind=kind):
                output = self.base / kind
                if kind == 'directory':
                    output.mkdir()
                elif kind == 'file':
                    output.write_bytes(b'preserve')
                else:
                    output.symlink_to(self.base / 'missing')
                with patch('builtins.open', side_effect=AssertionError('input I/O')), patch.object(acquisition, '_download', side_effect=AssertionError('network')):
                    with self.assertRaises(FileExistsError):
                        acquisition.acquire(output, self.archive)
                self.assertTrue(os.path.lexists(output))

    def test_symlink_ancestor_rejected_before_io(self):
        link = self.base / 'link'
        link.symlink_to(self.base, target_is_directory=True)
        with patch.object(acquisition, '_download', side_effect=AssertionError('network')):
            with self.assertRaises(OSError):
                acquisition.acquire(link / 'new')
        self.assertFalse(self.output.exists())

    def test_unsafe_paths(self):
        for name in ('/absolute', '../escape', self.prefix + '/a/../x',
                     self.prefix + '/./x', self.prefix + '//x', self.prefix + '/a\\x',
                     'reference-wrong/src/x', self.prefix):
            with self.subTest(name=name):
                self.reject_fixture([(name, b'x', tarfile.REGTYPE)])

    def test_links_and_specials(self):
        for kind in (tarfile.SYMTYPE, tarfile.LNKTYPE, tarfile.FIFOTYPE,
                     tarfile.CHRTYPE, tarfile.BLKTYPE):
            with self.subTest(kind=kind):
                self.reject_fixture([(self.prefix + '/x', b'', kind)])

    def test_duplicates_and_conflicts(self):
        file = (self.prefix + '/a', b'x', tarfile.REGTYPE)
        child = (self.prefix + '/a/b', b'y', tarfile.REGTYPE)
        directory = (self.prefix + '/a', b'', tarfile.DIRTYPE)
        for entries in ([file, file], [directory, directory], [file, child],
                        [child, file], [directory, file], [file, directory]):
            with self.subTest(entries=entries):
                self.reject_fixture(entries)

    def test_empty_archive(self):
        self.reject_fixture([(self.prefix, b'', tarfile.DIRTYPE)])

    def test_per_file_and_entry_limits(self):
        with patch.object(acquisition, '_FILE', 3):
            self.reject_fixture([(self.prefix + '/large', b'1234', tarfile.REGTYPE)])
        with patch.object(acquisition, '_ENTRIES', 1):
            self.reject_fixture([(self.prefix, b'', tarfile.DIRTYPE),
                                 (self.prefix + '/x', b'x', tarfile.REGTYPE)])

    def test_decompression_and_compressed_limits(self):
        with patch.object(acquisition, '_UNCOMPRESSED', 100):
            with self.assertRaisesRegex(ValueError, 'decompressed'):
                self.run_fixture()
        with patch.object(acquisition, '_COMPRESSED', len(self.raw) - 1):
            with self.assertRaisesRegex(ValueError, 'compressed'):
                self.run_fixture()
        self.assertFalse(self.output.exists())

    def test_truncated_archive(self):
        with self.assertRaises((EOFError, OSError, ValueError)):
            self.run_fixture(self.raw[:-8])
        self.assertFalse(self.output.exists())

    def test_postproof_failure_cleans_only_owned_output(self):
        sibling = self.base / 'untouched'
        sibling.write_bytes(b'keep')
        with patch.object(acquisition, 'verify_archive', side_effect=ValueError('postproof')):
            with self.assertRaisesRegex(ValueError, 'postproof'):
                self.run_fixture()
        self.assertFalse(self.output.exists())
        self.assertEqual(sibling.read_bytes(), b'keep')

    def network(self, raw=None, url=None):
        response = Mock()
        response.__enter__ = Mock(return_value=response)
        response.__exit__ = Mock(return_value=False)
        response.geturl.return_value = acquisition._URL if url is None else url
        response.read.return_value = self.raw if raw is None else raw
        opener = Mock()
        opener.open.return_value = response
        return opener, response

    def test_network_success_fixed_url_timeout_bounded_read(self):
        opener, response = self.network()
        with patch.object(acquisition.urllib.request, 'build_opener', return_value=opener), patch.object(acquisition, '_SHA256', hashlib.sha256(self.raw).hexdigest()):
            result = acquisition.acquire(self.output)
        self.assertEqual(result['verified_files'], 1)
        request = opener.open.call_args.args[0]
        self.assertEqual(request.full_url, acquisition._URL)
        self.assertIsNone(request.get_header('Authorization'))
        self.assertEqual(opener.open.call_args.kwargs, {'timeout': 60})
        response.read.assert_called_once_with(acquisition._COMPRESSED + 1)

    def test_network_errors_oversize_final_url_wrong_digest(self):
        for raw, url in ((self.raw, acquisition._URL + '?changed'),
                         (b'wrong bytes', None), (b'x' * 33, None)):
            opener, _ = self.network(raw, url)
            with self.subTest(url=url, size=len(raw)), patch.object(acquisition, '_COMPRESSED', 32), patch.object(acquisition.urllib.request, 'build_opener', return_value=opener):
                with self.assertRaises(ValueError):
                    acquisition.acquire(self.output)
                self.assertFalse(self.output.exists())
        with patch.object(acquisition.urllib.request, 'build_opener', side_effect=urllib.error.URLError('offline')):
            with self.assertRaises(urllib.error.URLError):
                acquisition.acquire(self.output)
        self.assertFalse(self.output.exists())

    def test_redirect_handler_rejects_before_following(self):
        with self.assertRaisesRegex(ValueError, 'redirect'):
            acquisition._NoRedirect().redirect_request(None, None, 302, '', {}, 'https://evil.invalid/')

    def test_cli_json_stdout_and_no_report(self):
        self.archive.write_bytes(self.raw)
        stdout = io.StringIO()
        argv = ['acquire', '--output-dir', str(self.output), '--archive', str(self.archive)]
        with patch('sys.argv', argv), patch.object(acquisition, '_SHA256', hashlib.sha256(self.raw).hexdigest()), redirect_stdout(stdout):
            self.assertEqual(acquisition.main(), 0)
        self.assertEqual(json.loads(stdout.getvalue())['readiness'], 'input-verified')
        self.assertEqual(sorted(p.name for p in self.output.iterdir()), [self.prefix, 'reference.tar.gz'])

    @unittest.skipUnless(os.environ.get('JR_REFERENCE_ARCHIVE'), 'retained official archive not supplied')
    def test_retained_official_archive(self):
        result = acquisition.acquire(self.output, os.environ['JR_REFERENCE_ARCHIVE'])
        self.assertEqual(result['archive_sha256'], acquisition._SHA256)
        self.assertEqual(result['reference_commit'], acquisition._COMMIT)
        self.assertGreater(result['verified_files'], 0)
        self.assertFalse(result['compatibility_credit'])


if __name__ == '__main__':
    unittest.main()
