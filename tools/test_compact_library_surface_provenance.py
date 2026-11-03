"""Compact provenance checks, with synthetic corruption controls and real inputs."""
import copy
import os
import hashlib
import json
from pathlib import Path
import tempfile
import unittest
from unittest.mock import patch

import compact_library_surface_provenance as compact
import acquire_pinned_api_html as html


class CompactTests(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory(prefix='compact-tests-')
        self.addCleanup(self.temp.cleanup)
        self.root = Path(self.temp.name)
        self.source = self.root / 'source.json'
        self.source.write_text('{}')
        self.surface = self.root / 'surface.json'
        self.identity = {'release': html.RELEASE, 'commit': html.COMMIT,
                         'root_tree_sha1': 'a' * 40, 'library_tree_sha1': 'b' * 40,
                         'source_index_content_sha256': compact.sha(self.source.read_bytes()),
                         'official_documentation_root': html.BASE}
        raw = b'<meta data-rustdoc-version="1.98.1 (48a229cea 2026-09-01)"><section id="main-content"></section>'
        self.doc = {'schema_version': 1, 'review_status': 'unreviewed',
                    'scope': {'edition': '2024', 'target': 'x86_64-unknown-linux-gnu'},
                    'source_identity': copy.deepcopy(self.identity), 'pages': [],
                    'index_link_candidates': [{'url': 'candidate-one'}, {'url': 'candidate-two'}],
                    'missing_detail_pages': ['core/struct.NotDownloaded.html']}
        for crate in html.CRATES:
            for name in ('index.html', 'all.html'):
                path = crate + '/' + name
                dest = self.root / 'html' / path
                dest.parent.mkdir(parents=True, exist_ok=True)
                dest.write_bytes(raw)
                self.doc['pages'].append({'path': path, 'url': html.page_url(path),
                    'content_sha256': compact.sha(raw), 'byte_length': len(raw),
                    'rustdoc_version': '1.98.1 (48a229cea 2026-09-01)',
                    'links': [{'arbitrary': 'excluded-large-details'}], 'declarations': ['excluded']})
        self.save()

    def save(self):
        self.surface.write_text(json.dumps(self.doc))

    def build(self):
        with patch.object(html, 'verify_source_index', return_value=self.identity):
            return compact.compact(self.surface, self.source)

    def test_compact_deterministic_metadata_excludes_candidate_details(self):
        first = self.build()
        self.assertEqual(compact.encode(first), compact.encode(self.build()))
        self.assertLess(len(compact.encode(first)), compact.MAX_BYTES)
        self.assertEqual(first['review_status'], 'unreviewed')
        self.assertEqual(first['observations']['literal_index_link_anchor_candidate_records'], 2)
        self.assertEqual(first['observations']['missing_detail_page_records'], 1)
        self.assertEqual(first['observations']['downloaded_page_records'], 8)
        self.assertEqual(first['artifacts'][0]['content_sha256'], compact.sha(self.surface.read_bytes()))
        self.assertEqual(first['artifacts'][1]['content_sha256'], compact.sha(self.source.read_bytes()))
        self.assertNotIn('index_link_candidates', first)
        self.assertNotIn('missing_detail_pages', first)
        self.assertTrue(all(set(p) == {'path', 'url', 'content_sha256', 'byte_length', 'rustdoc_version'}
                            for p in first['pages']))
        self.assertTrue(first['limitations'] and first['review_needs'])
        self.assertNotIn('pass', first)
        self.assertNotIn('percentage', first)
        for tool in first['reproduction']['tools']:
            expected = compact.sha((Path(compact.__file__).parent / tool['filename']).read_bytes())
            self.assertEqual(tool['content_sha256'], expected)

    def test_wrong_scope_rejected(self):
        self.doc['scope']['edition'] = '2021'
        self.save()
        with self.assertRaisesRegex(ValueError, 'scope mismatch'):
            self.build()

    def test_wrong_source_binding_rejected(self):
        self.doc['source_identity']['library_tree_sha1'] = '0' * 40
        self.save()
        with self.assertRaisesRegex(ValueError, 'binding mismatch'):
            self.build()

    def test_tampered_page_bytes_rejected(self):
        (self.root / 'html/core/all.html').write_bytes(b'corrupt')
        with self.assertRaisesRegex(ValueError, 'hash/length mismatch'):
            self.build()

    def test_wrong_version_even_with_updated_hash_rejected(self):
        page = self.doc['pages'][0]
        path = self.root / 'html' / page['path']
        raw = path.read_bytes().replace(b'48a229cea', b'000000000')
        path.write_bytes(raw)
        page['content_sha256'] = compact.sha(raw)
        self.save()
        with self.assertRaisesRegex(ValueError, 'marker mismatch'):
            self.build()

    def test_recorded_page_marker_mismatch_rejected(self):
        self.doc['pages'][0]['rustdoc_version'] = '1.98.1 (000000000 2026-09-01)'
        self.save()
        with self.assertRaisesRegex(ValueError, 'recorded page version'):
            self.build()

    def test_wrong_page_url_rejected(self):
        self.doc['pages'][0]['url'] = 'https://evil.example/index.html'
        self.save()
        with self.assertRaisesRegex(ValueError, 'path-to-URL mismatch'):
            self.build()

    def test_duplicate_page_rejected(self):
        self.doc['pages'].append(copy.deepcopy(self.doc['pages'][0]))
        self.save()
        with self.assertRaisesRegex(ValueError, 'duplicate observed'):
            self.build()

    def test_missing_required_crate_index_rejected(self):
        self.doc['pages'].pop()
        self.save()
        with self.assertRaisesRegex(ValueError, 'missing required crate'):
            self.build()

    def test_observation_byte_ceiling_is_enforced(self):
        with patch.object(compact, 'MAX_BYTES', 100):
            with self.assertRaisesRegex(ValueError, 'ceiling'):
                self.build()

    def test_real_pinned_compaction_binds_all_observed_pages(self):
        root = Path(os.environ.get('JR_COMPAT_LIBRARY_FIXTURES', str(Path(__file__).parent)))
        surface = root / 'pinned-html-final/pinned-html-api-index.json'
        source = root / 'pinned-source-final/library-source-index.json'
        if not surface.exists() or not source.exists():
            self.skipTest('real pinned acquisition inputs not present')
        value = compact.compact(surface, source)
        self.assertEqual(value['source_identity']['commit'], html.COMMIT)
        self.assertEqual(value['observations']['downloaded_page_records'], len(value['pages']))
        self.assertLess(len(compact.encode(value)), compact.MAX_BYTES)
        self.assertEqual(value['artifacts'][0]['content_sha256'], hashlib.sha256(surface.read_bytes()).hexdigest())
        self.assertEqual(value['source_identity']['source_index_content_sha256'], hashlib.sha256(source.read_bytes()).hexdigest())


if __name__ == '__main__':
    unittest.main()
