#!/usr/bin/env python3
"""Synthetic backlog rejection tests plus opt-in retained sidecar verification."""
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

import library_surface_backlog as backlog


class BacklogTests(unittest.TestCase):
    def setUp(self):
        # Never put temporary fixtures in the checkout or the system temp directory.
        root = Path(os.environ.get('JCODE_SCRATCH_DIR', Path.home() / '.jcode' / 'scratch'))
        root.mkdir(parents=True, exist_ok=True)
        self.temp = tempfile.TemporaryDirectory(prefix='library-backlog-test-', dir=root)
        self.addCleanup(self.temp.cleanup)
        self.root = Path(self.temp.name)
        self.surface_path = self.root / 'pinned-html-api-index.json'
        self.provenance_path = self.root / 'provenance.json'
        self.identity = {'release': '1.98.1', 'commit': '48a229cea' + '0' * 31,
            'library_tree_sha1': '1' * 40, 'root_tree_sha1': '2' * 40,
            'source_index_content_sha256': '3' * 64,
            'official_documentation_root': 'https://doc.rust-lang.org/1.98.1/',
            'repository': 'https://github.com/rust-lang/rust', 'verification': 'recorded input provenance'}
        self.pages = []
        for path in sorted([c + '/' + n for c in backlog.CRATES for n in ('index.html', 'all.html')] +
                           ['std/struct.Example.html']):
            self.pages.append({'path': path, 'url': self.url(path), 'byte_length': 10,
                'content_sha256': '4' * 64, 'rustdoc_version': '1.98.1 (48a229cea 2026-09-01)'})
        self.missing = sorted(['alloc/fn.missing.html', 'std/fn.missing.html'] +
                              ['core/fn.missing%03d.html' % i for i in range(105)])
        candidates = [self.candidate(p) for p in [x['path'] for x in self.pages] + self.missing]
        candidates.append(self.candidate('alloc/index.html', 'main-content'))
        self.surface = {'schema_version': 1, 'review_status': 'unreviewed',
            'scope': {'edition': '2024', 'target': 'x86_64-unknown-linux-gnu'},
            'source_identity': self.identity, 'pages': self.pages,
            'index_link_candidates': candidates, 'missing_detail_pages': self.missing,
            'limitations': ['synthetic literal observations']}
        self.provenance = {'schema_version': 1, 'review_status': 'unreviewed',
            'source_identity': self.identity, 'scope': self.surface['scope'],
            'pages': self.pages, 'artifacts': [], 'observations': {
                'downloaded_page_records': len(self.pages),
                'literal_index_link_anchor_candidate_records': len(candidates),
                'missing_detail_page_records': len(self.missing)}}
        self.save()

    def url(self, path):
        return self.identity['official_documentation_root'] + path

    def candidate(self, path, anchor=''):
        return {'path': path, 'anchor': anchor, 'url': self.url(path),
            'index_pages': [path.split('/')[0] + '/index.html'], 'labels': ['literal'],
            'stable_api_review': 'required', 'target_availability_review': 'required'}

    def save(self):
        raw = backlog.encode(self.surface)
        self.surface_path.write_bytes(raw)
        self.provenance['artifacts'] = [{'id': 'pinned-html-link-anchor-candidate',
            'filename': self.surface_path.name, 'content_sha256': hashlib.sha256(raw).hexdigest(),
            'byte_length': len(raw), 'role': 'retained synthetic sidecar'}]
        self.provenance_path.write_bytes(backlog.encode(self.provenance))

    def build(self):
        return backlog.build(self.surface_path, self.provenance_path)

    def test_deterministic_literal_counts_and_summary(self):
        data = self.build()
        self.assertEqual(backlog.encode(data), backlog.encode(self.build()))
        self.assertLess(len(backlog.encode(data)), 100_000)
        self.assertEqual(data['observations']['global'], {
            'literal_candidate_records': 117, 'unique_candidate_page_paths': 116,
            'downloaded_page_records': 9, 'downloaded_index_all_pages': 8,
            'downloaded_other_detail_pages': 1, 'missing_unique_paths': 107,
            'candidate_records_collapsed_by_page_path': 1})
        self.assertEqual([x['path'] for x in data['missing_path_sample']], self.missing[:100])
        self.assertIs(data['missing_path_sample_truncated'], True)
        summary = backlog.validate_metadata(data, self.provenance)
        self.assertEqual(summary['readiness'], 'review-required')
        self.assertEqual(summary['external_verification'], 'recorded-not-recomputed')
        self.assertEqual(summary['review_status'], 'unreviewed')
        self.assertIsNone(summary['full_rust_completion_percent'])
        self.assertIsNone(summary['api_denominator'])
        self.assertIsNone(summary['full_rust_denominator'])
        for key in ('review_credit', 'execution_credit', 'completion_credit'):
            self.assertEqual(summary[key], 0)
        self.assertEqual(backlog.verify(data, self.surface_path, self.provenance_path), data)
        self.assertFalse((self.root / 'html').exists())  # No HTML reread or verification claim.

    def test_zero_and_exactly_100_missing_not_truncated(self):
        for n in (0, 100):
            with self.subTest(n=n):
                missing = self.missing[:n]
                downloaded = {p['path'] for p in self.pages}
                self.surface['missing_detail_pages'] = missing
                self.surface['index_link_candidates'] = [self.candidate(p) for p in sorted(downloaded | set(missing))]
                self.provenance['observations']['missing_detail_page_records'] = n
                self.provenance['observations']['literal_index_link_anchor_candidate_records'] = len(downloaded) + n
                self.save()
                data = self.build()
                self.assertEqual(len(data['missing_path_sample']), n)
                self.assertIs(data['missing_path_sample_truncated'], False)

    def test_raw_byte_tamper_and_length_tamper(self):
        raw = self.surface_path.read_bytes()
        self.surface_path.write_bytes(raw + b' ')
        with self.assertRaisesRegex(ValueError, 'byte authentication'):
            self.build()
        self.surface_path.write_bytes(raw.replace(b'literal', b'LITERAL', 1))
        with self.assertRaisesRegex(ValueError, 'byte authentication'):
            self.build()
        self.surface_path.write_bytes(raw)
        self.provenance['artifacts'][0]['byte_length'] += 1
        self.provenance_path.write_bytes(backlog.encode(self.provenance))
        with self.assertRaisesRegex(ValueError, 'byte authentication'):
            self.build()

    def test_authenticated_duplicate_candidate_and_missing(self):
        for key in ('index_link_candidates', 'missing_detail_pages', 'pages'):
            with self.subTest(key=key):
                original = copy.deepcopy(self.surface[key])
                self.surface[key].append(copy.deepcopy(original[0]))
                self.save()
                with self.assertRaisesRegex(ValueError, 'duplicate'):
                    self.build()
                self.surface[key] = original

    def test_missing_set_exactness_even_when_count_matches(self):
        self.surface['missing_detail_pages'][-1] = 'std/fn.not_a_candidate.html'
        self.save()
        with self.assertRaisesRegex(ValueError, 'missing set'):
            self.build()

    def test_noncanonical_foreign_paths_urls_and_anchor(self):
        original = copy.deepcopy(self.surface['index_link_candidates'][0])
        for path in ('../core/index.html', 'core/../index.html', 'core//index.html',
                     '/core/index.html', 'core/%2e%2e/index.html', 'foreign/index.html',
                     'core/index.html?query', 'core/index.html#anchor', 'core\\index.html'):
            with self.subTest(path=path):
                self.surface['index_link_candidates'][0] = self.candidate(path)
                self.save()
                with self.assertRaises(ValueError):
                    self.build()
        self.surface['index_link_candidates'][0] = copy.deepcopy(original)
        for url in ('https://evil.invalid/alloc/all.html', self.url('alloc/all.html') + '#x'):
            with self.subTest(url=url):
                self.surface['index_link_candidates'][0]['url'] = url
                self.save()
                with self.assertRaisesRegex(ValueError, 'URL'):
                    self.build()
        self.surface['index_link_candidates'][0] = copy.deepcopy(original)
        self.surface['index_link_candidates'][0]['anchor'] = 'x#bad'
        self.save()
        with self.assertRaisesRegex(ValueError, 'anchor'):
            self.build()

    def test_page_metadata_and_identity_mismatch(self):
        for field, value in (('byte_length', 11), ('content_sha256', '9' * 64),
                             ('rustdoc_version', '1.99.0 (48a229cea 2026-09-01)'),
                             ('url', 'https://evil.invalid/alloc/all.html')):
            with self.subTest(field=field):
                self.surface['pages'] = copy.deepcopy(self.pages)
                self.surface['pages'][0][field] = value
                self.save()
                with self.assertRaises(ValueError):
                    self.build()
        self.surface['pages'] = copy.deepcopy(self.pages)
        self.surface['source_identity'] = {**self.identity, 'commit': '0' * 40}
        self.save()
        with self.assertRaisesRegex(ValueError, 'identity'):
            self.build()

    def test_structure_counts_bools_fields_truncation_and_binding(self):
        data = self.build()
        mutations = [lambda d: d.update(extra=0), lambda d: d.update(schema_version=True),
            lambda d: d.update(review_credit=False), lambda d: d.update(api_denominator=0),
            lambda d: d.update(full_rust_denominator=1), lambda d: d.update(missing_path_sample_truncated=1),
            lambda d: d.update(missing_path_sample_truncated=False),
            lambda d: d['observations']['global'].update(literal_candidate_records=True),
            lambda d: d['observations']['per_crate']['core'].update(missing_unique_paths=-1),
            lambda d: d['observations']['global'].update(missing_unique_paths=108),
            lambda d: d['observations']['per_crate']['std'].update(downloaded_other_detail_pages=0),
            lambda d: d['missing_path_sample'].append(d['missing_path_sample'][0]),
            lambda d: d['missing_path_sample'][0].update(path='core/../fn.missing.html'),
            lambda d: d['missing_path_sample'][0].update(url='https://evil.invalid/a'),
            lambda d: d['downloaded_pages'][0].update(byte_length=True),
            lambda d: d['downloaded_pages'][0].update(extra=0),
            lambda d: d['surface_artifact'].update(content_sha256='0' * 64),
            lambda d: d['source_identity'].update(verification='different recorded text')]
        for mutation in mutations:
            with self.subTest(mutation=mutation):
                changed = copy.deepcopy(data)
                mutation(changed)
                with self.assertRaises(ValueError):
                    backlog.validate_metadata(changed, self.provenance)
        changed = copy.deepcopy(data)
        changed['missing_path_sample'][1] = copy.deepcopy(changed['missing_path_sample'][0])
        with self.assertRaisesRegex(ValueError, 'duplicate'):
            backlog.validate_structure(changed)

    def test_offline_cannot_recompute_selfconsistent_percrate_counts_or_sample(self):
        data = self.build()
        changed = copy.deepcopy(data)
        # Move one observation from core to std, preserving every global aggregate.
        for key in ('literal_candidate_records', 'unique_candidate_page_paths', 'missing_unique_paths'):
            changed['observations']['per_crate']['core'][key] -= 1
            changed['observations']['per_crate']['std'][key] += 1
        backlog.validate_metadata(changed, self.provenance)
        with self.assertRaisesRegex(ValueError, 'recomputation'):
            backlog.verify(changed, self.surface_path, self.provenance_path)
        changed = copy.deepcopy(data)
        changed['missing_path_sample'][-1] = {'path': 'core/fn.missing999.html',
                                             'url': self.url('core/fn.missing999.html')}
        backlog.validate_metadata(changed, self.provenance)
        with self.assertRaisesRegex(ValueError, 'recomputation'):
            backlog.verify(changed, self.surface_path, self.provenance_path)

    def test_incorrect_provenance_aggregates_and_duplicate_descriptor(self):
        for key in self.provenance['observations']:
            with self.subTest(key=key):
                self.provenance['observations'][key] += 1
                self.save()
                with self.assertRaises(ValueError):
                    self.build()
                self.provenance['observations'][key] -= 1
        self.save()
        self.provenance['artifacts'].append(copy.deepcopy(self.provenance['artifacts'][0]))
        self.provenance_path.write_bytes(backlog.encode(self.provenance))
        with self.assertRaisesRegex(ValueError, 'duplicate'):
            self.build()

    def test_duplicate_json_keys_nonfinite_and_byte_bounds(self):
        self.surface_path.write_bytes(b'{"schema_version":1,"schema_version":1}')
        with self.assertRaisesRegex(ValueError, 'duplicate JSON'):
            self.build()
        self.surface_path.write_bytes(b'{"schema_version":NaN}')
        with self.assertRaises(ValueError):
            self.build()
        self.save()
        with mock.patch.object(backlog, 'MAX_SURFACE_BYTES', 10):
            with self.assertRaises(ValueError):
                self.build()
        data = self.build()
        with mock.patch.object(backlog, 'MAX_BYTES', 10):
            with self.assertRaisesRegex(ValueError, '100KB'):
                backlog.validate_structure(data)

    def test_public_malformed_nested_inputs_raise_value_error(self):
        data = self.build()
        for key in ('source_identity', 'scope', 'artifacts', 'pages', 'observations'):
            for value in (None, [], True, 1, 'malformed'):
                with self.subTest(input='provenance', key=key, value=value):
                    bad = copy.deepcopy(self.provenance)
                    bad[key] = value
                    self.provenance_path.write_bytes(backlog.encode(bad))
                    with self.assertRaises(ValueError):
                        backlog.validate_metadata(data, bad)
                    with self.assertRaises(ValueError):
                        self.build()
            bad = copy.deepcopy(self.provenance)
            del bad[key]
            self.provenance_path.write_bytes(backlog.encode(bad))
            with self.assertRaises(ValueError):
                backlog.validate_metadata(data, bad)
            with self.assertRaises(ValueError):
                self.build()
        self.save()
        original = copy.deepcopy(self.surface)
        for key in ('source_identity', 'scope', 'pages', 'index_link_candidates', 'missing_detail_pages'):
            for value in (None, {}, True, 1, 'malformed'):
                with self.subTest(input='surface', key=key, value=value):
                    self.surface = copy.deepcopy(original)
                    self.surface[key] = value
                    self.save()
                    with self.assertRaises(ValueError):
                        self.build()
        for key in ('source_identity', 'downloaded_pages', 'observations', 'missing_path_sample'):
            with self.subTest(input='data', key=key):
                bad = copy.deepcopy(data)
                bad[key] = None
                with self.assertRaises(ValueError):
                    backlog.validate_metadata(bad, self.provenance)

    def test_impossible_dates_and_inconsistent_markers_rejected(self):
        data = self.build()
        for marker in ('1.98.1 (48a229cea 2026-02-30)',
                       '1.98.1 (48a229cea 2026-00-01)',
                       '1.98.1 (48a229cea 2026-09-02)'):
            with self.subTest(marker=marker):
                bad = copy.deepcopy(data)
                bad['downloaded_pages'][0]['rustdoc_version'] = marker
                with self.assertRaises(ValueError):
                    backlog.validate_structure(bad)
                bad_provenance = copy.deepcopy(self.provenance)
                bad_provenance['pages'][0]['rustdoc_version'] = marker
                with self.assertRaises(ValueError):
                    backlog.validate_metadata(data, bad_provenance)

    def test_cli_options_checked_before_io_and_exclusive_output(self):
        with mock.patch.object(Path, 'open', side_effect=AssertionError('IO before option validation')):
            for args in ([], ['generate'], ['generate', '--surface-index', 'x', '--provenance', 'y',
                           '--output', 'z', '--backlog', 'w']):
                with contextlib.redirect_stderr(io.StringIO()), self.assertRaises(SystemExit):
                    backlog.main(args)
        output = self.root / 'backlog.json'
        args = ['generate', '--surface-index', str(self.surface_path), '--provenance',
                str(self.provenance_path), '--output', str(output)]
        self.assertEqual(backlog.main(args), 0)
        before = output.read_bytes()
        with contextlib.redirect_stderr(io.StringIO()):
            self.assertEqual(backlog.main(args), 1)
        self.assertEqual(output.read_bytes(), before)
        self.assertEqual(backlog.main(['verify', '--surface-index', str(self.surface_path),
            '--provenance', str(self.provenance_path), '--backlog', str(output)]), 0)
        output.write_bytes(b'{broken')
        with contextlib.redirect_stderr(io.StringIO()):
            self.assertEqual(backlog.main(['verify', '--surface-index', str(self.surface_path),
                '--provenance', str(self.provenance_path), '--backlog', str(output)]), 1)

    @unittest.skipUnless(os.environ.get('JR_COMPAT_LIBRARY_FIXTURES'), 'retained sidecar fixture is optional')
    def test_actual_retained_fixture(self):
        # Accept the shared acquisition root or the retained HTML directory itself.
        fixture = Path(os.environ['JR_COMPAT_LIBRARY_FIXTURES'])
        surface_path = fixture / 'pinned-html-final' / 'pinned-html-api-index.json'
        if not surface_path.exists():
            surface_path = fixture / 'pinned-html-api-index.json'
        provenance_path = Path(__file__).resolve().parents[1] / 'data' / 'compiler-library-surface-provenance.json'
        data = backlog.build(surface_path, provenance_path)
        self.assertEqual(data['observations']['global'], {
            'literal_candidate_records': 35236, 'unique_candidate_page_paths': 35154,
            'downloaded_page_records': 40, 'downloaded_index_all_pages': 8,
            'downloaded_other_detail_pages': 32, 'missing_unique_paths': 35114,
            'candidate_records_collapsed_by_page_path': 82})
        self.assertLess(len(backlog.encode(data)), 100_000)
        backlog.verify(data, surface_path, provenance_path)


if __name__ == '__main__':
    unittest.main()
