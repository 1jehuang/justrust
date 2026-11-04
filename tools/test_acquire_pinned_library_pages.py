#!/usr/bin/env python3
"""Offline and adversarial tests. No test makes a network request."""
import contextlib
import copy
import hashlib
import io
import json
import os
from pathlib import Path
import tempfile
import unittest
from unittest import mock

import acquire_pinned_library_pages as acquire
import library_witness_sources as witness


class AcquisitionTests(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory()
        self.addCleanup(self.temp.cleanup)
        self.root = Path(self.temp.name)
        self.source = self.root / 'input'
        self.output = self.root / 'output'
        self.payloads, self.pages = {}, []
        for path, anchors in sorted(acquire._ANCHORS.items()):
            raw = ('<html data-rustdoc-version="' + acquire._VERSION + '">' +
                   ''.join('<i id="' + anchor + '"></i>' for anchor in anchors) +
                   '</html>').encode()
            self.payloads[path] = raw
            self.pages.append({'path': path, 'url': acquire._BASE + path,
                               'rustdoc_version': acquire._VERSION,
                               'byte_length': len(raw),
                               'content_sha256': hashlib.sha256(raw).hexdigest()})
            file = self.source / path
            file.parent.mkdir(parents=True, exist_ok=True)
            file.write_bytes(raw)
        self.manifest = {'pages': self.pages}
        self.metadata = self.root / witness.PROVENANCE
        self.metadata.parent.mkdir()
        self.root_patch = mock.patch.object(acquire, '_ROOT', self.root)
        self.root_patch.start()
        self.addCleanup(self.root_patch.stop)
        self.pin_patch = mock.patch.dict(witness.PINS, {})
        self.pin_patch.start()
        self.addCleanup(self.pin_patch.stop)
        self.pin_manifest(self.manifest)
        self.network = mock.patch.object(acquire.urllib.request, 'build_opener',
                                         side_effect=AssertionError('network forbidden'))
        self.network.start()
        self.addCleanup(self.network.stop)

    def pin_manifest(self, manifest):
        raw = json.dumps(manifest).encode()
        self.metadata.write_bytes(raw)
        witness.PINS[witness.PROVENANCE] = hashlib.sha256(raw).hexdigest()

    def assert_rejected(self):
        with self.assertRaises((OSError, ValueError)):
            acquire.acquire(self.output, self.source)
        self.assertFalse(self.output.exists())

    def repin_page(self, path, raw):
        (self.source / path).write_bytes(raw)
        for page in self.pages:
            if page['path'] == path:
                page.update(byte_length=len(raw), content_sha256=hashlib.sha256(raw).hexdigest())
        self.pin_manifest(self.manifest)

    def test_deterministic_offline_and_exact_output(self):
        first = acquire.acquire(self.output, self.source)
        expected = {'readiness': 'input-verified', 'verified_pages': 3,
                    'verified_anchors': 7, 'html_root': str(self.output / 'html'),
                    'compatibility_credit': False}
        self.assertEqual(first, expected)
        second = self.root / 'second'
        result = acquire.acquire(second, self.source)
        self.assertEqual(result, dict(expected, html_root=str(second / 'html')))
        for directory in (self.output, second):
            files = {str(p.relative_to(directory)): p.read_bytes()
                     for p in directory.rglob('*') if p.is_file()}
            self.assertEqual(files, {'html/' + p: raw for p, raw in self.payloads.items()})

    def test_cli_schema_and_required_output(self):
        with contextlib.redirect_stdout(io.StringIO()) as stream:
            self.assertEqual(acquire.main(['--output-dir', str(self.output),
                                          '--html-root', str(self.source)]), 0)
        self.assertEqual(json.loads(stream.getvalue())['verified_anchors'], 7)
        with contextlib.redirect_stderr(io.StringIO()), self.assertRaises(SystemExit):
            acquire.main([])

    def test_raw_manifest_hash_is_authority(self):
        self.metadata.write_bytes(self.metadata.read_bytes() + b' ')
        self.assert_rejected()

    def test_malformed_manifest_and_duplicate_keys(self):
        for raw in (b'{', b'null', b'[]', b'{"pages":null}', b'{"pages":[],"pages":[]}'):
            with self.subTest(raw=raw):
                self.metadata.write_bytes(raw)
                witness.PINS[witness.PROVENANCE] = hashlib.sha256(raw).hexdigest()
                self.assert_rejected()

    def test_malformed_selected_pin_metadata(self):
        for key, value in (('byte_length', True), ('byte_length', 0),
                           ('byte_length', -1), ('byte_length', acquire._LIMIT + 1),
                           ('byte_length', '123'), ('content_sha256', 'A' * 64),
                           ('content_sha256', None), ('rustdoc_version', '1.98.0'),
                           ('url', 'https://foreign.invalid/page'),
                           ('url', acquire._BASE + self.pages[0]['path'] + '?x=1')):
            with self.subTest(key=key, value=value):
                manifest = copy.deepcopy(self.manifest)
                manifest['pages'][0][key] = value
                self.pin_manifest(manifest)
                self.assert_rejected()
        for manifest in ({'pages': self.pages[1:]},
                         {'pages': self.pages + [self.pages[0]]}, {'pages': [None]},
                         {'pages': [dict(self.pages[0], surprise=True)] + self.pages[1:]}):
            self.pin_manifest(manifest)
            self.assert_rejected()

    def test_missing_duplicate_ids_and_version_drift(self):
        path = self.pages[0]['path']
        original = self.payloads[path]
        anchor = acquire._ANCHORS[path][0]
        for raw in (original.replace(anchor.encode(), b'not-an-anchor'),
                    original + ('<i id="' + anchor + '">').encode(),
                    original.replace(b'1.98.1', b'1.98.0'),
                    original + ('<i data-rustdoc-version="' + acquire._VERSION + '">').encode(),
                    original.replace(('id="' + anchor + '"').encode(),
                                     ('id="' + anchor + '" id="' + anchor + '"').encode())):
            with self.subTest(raw=raw):
                self.repin_page(path, raw)
                self.assert_rejected()

    def test_mismatch_truncation_and_overread(self):
        path = self.pages[-1]['path']  # Last page failure still creates no output.
        for raw in (b'', self.payloads[path][:-1], self.payloads[path] + b'x',
                    b'x' * (acquire._LIMIT + 1)):
            (self.source / path).write_bytes(raw)
            self.assert_rejected()

    def test_existing_destination_never_reads_inputs(self):
        for kind in ('directory', 'file', 'link', 'dangling'):
            output = self.root / kind
            if kind == 'directory':
                output.mkdir()
                (output / 'sentinel').write_bytes(b'preserve')
            elif kind == 'file':
                output.write_bytes(b'preserve')
            else:
                output.symlink_to(self.source if kind == 'link' else self.root / 'absent')
            with mock.patch.object(acquire, '_pages', side_effect=AssertionError('input read')):
                with self.assertRaises(OSError):
                    acquire.acquire(output, self.source)
            self.assertTrue(output.exists() or output.is_symlink())
        self.assertEqual((self.root / 'directory/sentinel').read_bytes(), b'preserve')
        self.assertEqual((self.root / 'file').read_bytes(), b'preserve')

    def test_symlink_output_ancestor_and_traversal(self):
        alias = self.root / 'alias'
        alias.symlink_to(self.root, target_is_directory=True)
        with self.assertRaises(OSError):
            acquire.acquire(alias / 'output', self.source)
        with self.assertRaises(ValueError):
            acquire.acquire(alias / '..' / 'output', self.source)
        self.assertFalse(self.output.exists())

    def test_symlink_input_root_ancestor_file_and_metadata(self):
        alias = self.root / 'alias'
        alias.symlink_to(self.source, target_is_directory=True)
        with self.assertRaises(OSError):
            acquire.acquire(self.output, alias)
        path = self.source / self.pages[0]['path']
        saved = self.root / 'saved'
        path.rename(saved)
        path.symlink_to(saved)
        self.assert_rejected()
        path.unlink()
        saved.rename(path)
        subdir = self.source / 'alloc'
        saved_dir = self.root / 'saved-dir'
        subdir.rename(saved_dir)
        subdir.symlink_to(saved_dir, target_is_directory=True)
        self.assert_rejected()
        subdir.unlink()
        saved_dir.rename(subdir)
        saved_meta = self.root / 'saved-meta'
        self.metadata.rename(saved_meta)
        self.metadata.symlink_to(saved_meta)
        self.assert_rejected()

    def test_special_file_input_never_blocks(self):
        file = self.source / self.pages[0]['path']
        file.unlink()
        os.mkfifo(file)
        self.assert_rejected()

    def test_cleanup_on_partial_publication_failure(self):
        real_open = acquire.os.open
        writes = []

        def fail_second_write(path, flags, *args, **kwargs):
            if flags & os.O_CREAT:
                writes.append(path)
                if len(writes) == 2:
                    raise OSError('injected write failure')
            return real_open(path, flags, *args, **kwargs)

        with mock.patch.object(acquire.os, 'open', side_effect=fail_second_write):
            self.assert_rejected()
        self.assertEqual(len(writes), 2)
        self.assertTrue(self.source.is_dir())

    def test_fetch_only_three_fixed_urls(self):
        with mock.patch.object(acquire, '_download',
                               side_effect=lambda page: self.payloads[page['path']]) as download:
            acquire.acquire(self.output)
        self.assertEqual([call.args[0]['url'] for call in download.call_args_list],
                         [acquire._BASE + path for path in sorted(acquire._ANCHORS)])

    def test_download_bounds_redirects_encoding_and_foreign_urls(self):
        page = self.pages[0]
        raw = self.payloads[page['path']]
        response = mock.MagicMock()
        response.__enter__.return_value = response
        response.geturl.return_value = page['url']
        response.status = 200
        response.headers = {}
        response.read.return_value = raw
        opener = mock.Mock()
        opener.open.return_value = response
        with mock.patch.object(acquire.urllib.request, 'build_opener', return_value=opener):
            self.assertEqual(acquire._download(page), raw)
            request = opener.open.call_args.args[0]
            self.assertEqual(request.full_url, page['url'])
            self.assertEqual(request.get_header('Accept-encoding'), 'identity')
            self.assertEqual(opener.open.call_args.kwargs, {'timeout': 30})
            response.read.assert_called_once_with(page['byte_length'] + 1)
            for change in ('url', 'encoding', 'length', 'status', 'overread'):
                response.geturl.return_value = page['url']
                response.headers, response.status, response.read.return_value = {}, 200, raw
                if change == 'url':
                    response.geturl.return_value = 'https://foreign.invalid/'
                elif change == 'encoding':
                    response.headers = {'Content-Encoding': 'gzip'}
                elif change == 'length':
                    response.headers = {'Content-Length': str(acquire._LIMIT + 1)}
                elif change == 'status':
                    response.status = 206
                else:
                    response.read.return_value = raw + b'x'
                with self.subTest(change=change), self.assertRaises(ValueError):
                    acquire._download(page)
        with self.assertRaises(ValueError):
            acquire._download(dict(page, url='https://foreign.invalid/'))
        for target in (page['url'], 'https://foreign.invalid/'):
            with self.assertRaises(ValueError):
                acquire._NoRedirect().redirect_request(None, None, 302, '', {}, target)


class TrustedInputTests(unittest.TestCase):
    def test_actual_compact_pages_equal_witness_summary_links(self):
        pages = acquire._pages()
        summary = witness.summarize(acquire._ROOT)
        self.assertEqual(summary['documentation_page_count'], 3)
        references = {}
        anchors = {}
        for link in summary['links']:
            for reference in link['documentation_references']:
                path = reference['path']
                references[path] = {key: reference[key] for key in
                                    ('path', 'url', 'byte_length', 'content_sha256', 'rustdoc_version')}
                anchors.setdefault(path, set()).add(reference['anchor'])
        self.assertEqual(pages, [references[path] for path in sorted(references)])
        self.assertEqual(anchors, {p: set(a) for p, a in acquire._ANCHORS.items()})

    def test_optional_actual_input(self):
        source = os.environ.get('JR_LIBRARY_WITNESS_HTML_ROOT')
        if not source and os.environ.get('JR_COMPAT_LIBRARY_FIXTURES'):
            source = str(Path(os.environ['JR_COMPAT_LIBRARY_FIXTURES']) / 'pinned-html-final/html')
        if not source:
            self.skipTest('set JR_LIBRARY_WITNESS_HTML_ROOT for optional real three-page replay')
        with tempfile.TemporaryDirectory() as temp, mock.patch.object(
                acquire.urllib.request, 'build_opener', side_effect=AssertionError('network forbidden')):
            output = Path(temp) / 'acquired'
            result = acquire.acquire(output, Path(source))
            self.assertEqual(result['verified_pages'], 3)
            self.assertEqual(result['verified_anchors'], 7)
            self.assertIs(result['compatibility_credit'], False)
            summary = witness.summarize(acquire._ROOT, Path(result['html_root']))
            self.assertEqual(summary['documentation_anchor_verification'], 'reverified_in_this_run')


if __name__ == '__main__':
    unittest.main()
