"""Literal proc_macro classification controls, including unchanged exact inputs."""
import copy
import hashlib
import json
import os
from pathlib import Path
import tempfile
import unittest
from unittest.mock import patch

import classify_proc_macro_details as classify
import acquire_pinned_api_html as pinned


def header(stable=True):
    marker = '<span class="since" title="Stable since Rust version 1.15.0">1.15.0</span>' if stable else ''
    return ('<meta data-rustdoc-version="1.98.1 (48a229cea 2026-09-01)">'
            '<section id="main-content"><div class="main-heading">' + marker + '</div>'
            '<pre class="rust item-decl">pub struct Example;</pre>')


def since():
    return '<span class="since" title="Stable since Rust version 1.29.0">1.29.0</span>'


def unstable():
    return '<span class="item-info"><div class="stab unstable">Nightly-only (example_feature)</div></span>'


def method(anchor, annotation='', body=''):
    return ('<details class="toggle method-toggle"><summary><section id="' + anchor + '" class="method">'
            '<h4 class="code-header">pub fn sample(&amp;self)</h4></section>' + annotation + '</summary>' + body + '</details>')


def parse(text):
    rows, unowned = classify.classify_page(text.encode(), 'proc_macro/struct.Example.html',
                                         pinned.BASE + 'proc_macro/struct.Example.html')
    return {r['anchor']: r for r in rows}, unowned


class ScopeTests(unittest.TestCase):
    def test_stable_parent_sibling_summary_unstable_method_does_not_leak(self):
        rows, _ = parse(header() + method('method.experimental', unstable()) + '</section>')
        self.assertEqual(rows['main-content']['literal_stability'], 'stable')
        self.assertEqual(rows['method.experimental']['literal_stability'], 'unstable')
        self.assertEqual(rows['method.experimental']['stability_evidence'][0]['ownership_basis'], 'item-summary')
        self.assertIn('associated', rows['method.experimental']['categories'])

    def test_unstable_parent_does_not_override_explicit_stable_child(self):
        rows, _ = parse(header(False) + unstable() + method('method.documented', since()) + '</section>')
        self.assertEqual(rows['main-content']['literal_stability'], 'unstable')
        self.assertEqual(rows['method.documented']['literal_stability'], 'stable')
        for row in rows.values():
            self.assertEqual(row['public_reachability'], 'unresolved')
            self.assertEqual(row['target_availability'], 'unresolved')

    def test_nested_impl_and_method_headers_keep_separate_owners(self):
        text = (header() + '<details class="toggle implementors-toggle"><summary>'
                '<section id="impl-Example" class="impl"><h3 class="code-header">impl Example</h3></section>'
                + unstable() + '</summary><div class="impl-items">'
                + method('method.inner', since()) + '</div></details></section>')
        rows, _ = parse(text)
        self.assertEqual(rows['impl-Example']['literal_stability'], 'unstable')
        self.assertEqual(rows['method.inner']['literal_stability'], 'stable')
        self.assertEqual(rows['main-content']['literal_stability'], 'stable')

    def test_missing_ownership_remains_ambiguous_not_inherited(self):
        text = header() + method('method.unknown', '', '<div class="docblock">' + unstable() + '</div>') + '</section>'
        rows, unowned = parse(text)
        self.assertEqual(rows['method.unknown']['literal_stability'], 'ambiguous')
        self.assertTrue(unowned)
        self.assertEqual(rows['main-content']['literal_stability'], 'stable')

    def test_ambiguous_summary_with_multiple_item_headers_does_not_guess_owner(self):
        text = (header() + '<details><summary><section id="method.a" class="method"></section>'
                '<section id="method.b" class="method"></section>' + unstable() + '</summary></details></section>')
        rows, unknown = parse(text)
        self.assertEqual(rows['method.a']['literal_stability'], 'ambiguous')
        self.assertEqual(rows['method.b']['literal_stability'], 'ambiguous')
        self.assertTrue(any(e['ownership_basis'] == 'ambiguous-summary-ownership' for e in unknown))

    def test_conflicting_explicit_markers_are_ambiguous(self):
        rows, _ = parse(header() + method('method.conflict', since() + unstable()) + '</section>')
        self.assertEqual(rows['method.conflict']['literal_stability'], 'ambiguous')

    def test_const_unstable_is_not_runtime_unstable(self):
        annotation = '<span class="since" title="Stable since Rust version 1.0.0, const unstable">1.0.0 (const: unstable)</span>'
        rows, _ = parse(header() + method('method.const_case', annotation) + '</section>')
        self.assertEqual(rows['method.const_case']['literal_stability'], 'stable')
        self.assertEqual(rows['method.const_case']['const_stability'], 'unstable-explicit')

    def test_reexport_requires_literal_marker_not_a_different_source_filename(self):
        text = header().replace('pub struct Example;', 'pub use other::Example;') + '</section>'
        rows, _ = parse(text)
        self.assertIn('reexport', rows['main-content']['categories'])
        rows, _ = parse(header() + '<a class="src" href="../src/other.rs.html">Source</a></section>')
        self.assertNotIn('reexport', rows['main-content']['categories'])
        self.assertEqual(rows['main-content']['reexport_review'], 'unresolved')

    def test_duplicate_item_anchor_fails_closed(self):
        with self.assertRaisesRegex(ValueError, 'duplicate item anchor'):
            parse(header() + method('method.repeat') + method('method.repeat') + '</section>')

    def test_wrong_version_fails_closed(self):
        with self.assertRaisesRegex(ValueError, 'marker mismatch'):
            parse((header() + '</section>').replace('48a229cea', '000000000'))


class BoundInputTests(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory(prefix='literal-proc-tests-', dir=Path(__file__).parent)
        self.addCleanup(self.temp.cleanup)
        self.root = Path(self.temp.name)
        self.surface = self.root / 'surface.json'
        self.source = self.root / 'source.json'
        self.provenance = self.root / 'provenance.json'
        self.source.write_text('{}')
        self.identity = {'release': pinned.RELEASE, 'commit': pinned.COMMIT,
                         'root_tree_sha1': 'a' * 40, 'library_tree_sha1': 'b' * 40,
                         'source_index_content_sha256': hashlib.sha256(self.source.read_bytes()).hexdigest()}
        self.page_path = self.root / 'html/proc_macro/struct.Example.html'
        self.page_path.parent.mkdir(parents=True)
        self.page_path.write_bytes((header() + method('method.new', since()) + '</section>').encode())
        self.doc = {'schema_version': 1, 'review_status': 'unreviewed',
                    'scope': {'edition': '2024', 'target': 'x86_64-unknown-linux-gnu'},
                    'source_identity': copy.deepcopy(self.identity), 'pages': []}
        self.sync()

    def sync(self):
        raw = self.page_path.read_bytes()
        record = {'path': 'proc_macro/struct.Example.html', 'url': pinned.BASE + 'proc_macro/struct.Example.html',
                  'content_sha256': hashlib.sha256(raw).hexdigest(), 'byte_length': len(raw),
                  'rustdoc_version': '1.98.1 (48a229cea 2026-09-01)'}
        self.doc['pages'] = [record]
        self.surface.write_text(json.dumps(self.doc))
        provenance = {'schema_version': 1, 'review_status': 'unreviewed',
                      'source_identity': self.identity, 'pages': [record],
                      'artifacts': [{'id': i, 'content_sha256': hashlib.sha256(p.read_bytes()).hexdigest(),
                                     'byte_length': len(p.read_bytes())}
                                    for i, p in (('pinned-html-link-anchor-candidate', self.surface),
                                                 ('pinned-library-source-path-index', self.source))]}
        self.provenance.write_text(json.dumps(provenance))

    def build(self):
        with patch.object(pinned, 'verify_source_index', return_value=self.identity):
            return classify.build(self.surface, self.source, self.provenance)

    def test_bound_synthetic_input_is_deterministic_and_keeps_unresolved_states(self):
        first = self.build()
        self.assertEqual(first, self.build())
        self.assertTrue(first['records'])
        self.assertEqual(first['review_status'], 'unreviewed')
        self.assertTrue(all(r['public_reachability'] == 'unresolved' and r['target_availability'] == 'unresolved'
                            for r in first['records']))
        self.assertNotIn('pass', first)
        self.assertNotIn('api_count', first)

    def test_duplicate_retained_page_rejected_even_when_digest_rebound(self):
        self.doc['pages'].append(copy.deepcopy(self.doc['pages'][0]))
        self.surface.write_text(json.dumps(self.doc))
        compact = json.loads(self.provenance.read_text())
        artifact = compact['artifacts'][0]
        artifact['content_sha256'] = hashlib.sha256(self.surface.read_bytes()).hexdigest()
        artifact['byte_length'] = len(self.surface.read_bytes())
        self.provenance.write_text(json.dumps(compact))
        with self.assertRaisesRegex(ValueError, 'duplicate retained'):
            self.build()

    def test_missing_page_rejected(self):
        self.page_path.unlink()
        with self.assertRaises(FileNotFoundError):
            self.build()

    def test_tampered_page_rejected(self):
        self.page_path.write_bytes(b'changed')
        with self.assertRaisesRegex(ValueError, 'page bytes changed'):
            self.build()

    def test_page_wrong_version_rejected_even_after_rehashing_metadata(self):
        self.page_path.write_bytes(self.page_path.read_bytes().replace(b'48a229cea', b'000000000'))
        self.sync()
        with self.assertRaisesRegex(ValueError, 'marker mismatch'):
            self.build()

    def test_tampered_original_candidate_rejected(self):
        self.surface.write_text(self.surface.read_text() + ' ')
        with self.assertRaisesRegex(ValueError, 'artifact digest/length mismatch'):
            self.build()

    def test_source_identity_mismatch_rejected(self):
        self.doc['source_identity']['commit'] = '0' * 40
        self.sync()
        with self.assertRaisesRegex(ValueError, 'source identity mismatch'):
            self.build()

    def actual_paths(self):
        root = Path(os.environ.get("JR_COMPAT_LIBRARY_FIXTURES", str(Path(__file__).parent)))
        paths = (root / 'pinned-html-final/pinned-html-api-index.json',
                 root / 'pinned-source-final/library-source-index.json',
                 root / 'compact-final/data/compiler-library-surface-provenance.json')
        if not all(p.exists() for p in paths):
            self.skipTest('unchanged real pinned acquisition artifacts not available')
        return paths

    def test_actual_exact_pages_stable_unstable_and_const_scopes(self):
        paths = self.actual_paths()
        before = [hashlib.sha256(p.read_bytes()).hexdigest() for p in paths]
        result = classify.build(*paths)
        self.assertEqual(result, classify.build(*paths))
        rows = {(r['page'], r['anchor']): r for r in result['records']}
        token = 'proc_macro/struct.TokenStream.html'
        self.assertEqual(rows[token, 'main-content']['literal_stability'], 'stable')
        self.assertEqual(rows[token, 'method.new']['literal_stability'], 'stable')
        self.assertEqual(rows[token, 'method.expand_expr']['literal_stability'], 'unstable')
        self.assertEqual(rows['proc_macro/struct.Diagnostic.html', 'main-content']['literal_stability'], 'unstable')
        self.assertEqual(rows[token, 'method.clone_from']['literal_stability'], 'stable')
        self.assertEqual(rows[token, 'method.clone_from']['const_stability'], 'unstable-explicit')
        self.assertTrue(any(r['literal_stability'] == 'ambiguous' for r in rows.values()))
        self.assertTrue(all(r['public_reachability'] == 'unresolved' for r in rows.values()))
        self.assertEqual(before, [hashlib.sha256(p.read_bytes()).hexdigest() for p in paths])

    def test_actual_page_bytes_tamper_rejected_without_changing_originals(self):
        paths = self.actual_paths()
        target = paths[0].parent / 'html/proc_macro/struct.TokenStream.html'
        original = Path.read_bytes
        def read(path):
            return b'tampered' if path == target else original(path)
        with patch.object(Path, 'read_bytes', new=read):
            with self.assertRaisesRegex(ValueError, 'page bytes changed'):
                classify.build(*paths)

    def test_actual_missing_page_rejected_without_changing_originals(self):
        paths = self.actual_paths()
        target = paths[0].parent / 'html/proc_macro/struct.TokenStream.html'
        original = Path.read_bytes
        def read(path):
            if path == target:
                raise FileNotFoundError('controlled missing exact page')
            return original(path)
        with patch.object(Path, 'read_bytes', new=read):
            with self.assertRaises(FileNotFoundError):
                classify.build(*paths)


if __name__ == '__main__':
    unittest.main()
