"""No-network unit tests for actual-source acquisition and HTML indexing helpers."""
import copy
import hashlib
import json
from pathlib import Path
import os
import shutil
import tempfile
import unittest
from unittest.mock import patch

import acquire_pinned_library_sources as source
import acquire_pinned_api_html as html


class SourceIdentityTests(unittest.TestCase):
    def test_known_git_blob_hash_not_content_sha1(self):
        self.assertEqual(source.object_hash('blob', b'hello\n'), 'ce013625030ba8dba906f756967f9e9ca394464a')
        self.assertNotEqual(source.object_hash('blob', b'hello\n'), hashlib.sha1(b'hello\n').hexdigest())

    def fixture(self):
        blob = {'path': 'lib.rs', 'mode': '100644', 'type': 'blob',
                'sha': source.object_hash('blob', b'pub fn sample() {}\n')}
        tree = {'path': 'src', 'mode': '040000', 'type': 'tree', 'sha': source.tree_hash([blob])}
        root = source.tree_hash([tree])
        return root, {'sha': root, 'truncated': False,
                      'tree': [tree, dict(blob, path='src/lib.rs')]}

    def test_recursive_tree_reconstruction(self):
        root, doc = self.fixture()
        self.assertEqual(source.verify_recursive_tree(doc, root), doc['tree'])

    def test_tampered_blob_hash_rejects(self):
        root, doc = self.fixture()
        doc['tree'][1]['sha'] = '0' * 40
        with self.assertRaisesRegex(ValueError, 'tree hash mismatch'):
            source.verify_recursive_tree(doc, root)

    def test_truncated_tree_rejects(self):
        root, doc = self.fixture()
        doc['truncated'] = True
        with self.assertRaisesRegex(ValueError, 'truncated'):
            source.verify_recursive_tree(doc, root)

    def test_duplicate_and_missing_directory_reject(self):
        root, doc = self.fixture()
        doc['tree'].append(copy.deepcopy(doc['tree'][1]))
        with self.assertRaisesRegex(ValueError, 'duplicate recursive source path'):
            source.verify_recursive_tree(doc, root)
        root, doc = self.fixture()
        doc['tree'].pop(0)
        with self.assertRaisesRegex(ValueError, 'missing directory entry'):
            source.verify_recursive_tree(doc, root)

    def test_path_traversal_rejects(self):
        for path in ('../evil', '/absolute', 'folder/../../evil', 'back\\slash'):
            with self.subTest(path=path), self.assertRaises(ValueError):
                source.safe_path(path)

    def test_real_pinned_acquisition_tree_and_blob_identity(self):
        # Optional representative acceptance of downloaded official bytes.
        root = Path(os.environ.get('JR_COMPAT_LIBRARY_FIXTURES', str(Path(__file__).parent))) / 'pinned-source-acquisition'
        if not root.exists():
            self.skipTest('real pinned-source acquisition not present')
        index = json.loads((root / 'library-source-index.json').read_text())
        identity = index['source_identity']
        self.assertEqual(identity['commit'], source.COMMIT)
        self.assertEqual(source.tree_hash(json.loads((root / 'root-tree.json').read_text())['tree']),
                         identity['root_tree_sha1'])
        source.verify_recursive_tree(json.loads((root / 'library-tree.json').read_text()), identity['library_tree_sha1'])
        for record in index['downloaded_files']:
            raw = (root / 'sources' / record['path']).read_bytes()
            self.assertEqual(source.object_hash('blob', raw), record['git_blob_sha1'])
            self.assertEqual(hashlib.sha256(raw).hexdigest(), record['content_sha256'])


class HTMLIndexTests(unittest.TestCase):
    def fixture(self, version='1.98.1 (48a229cea 2026-09-01)'):
        return ('''<html><head><meta data-rustdoc-version="''' + version + '''"></head><body>
<section id="main-content"><pre class="rust item-decl"><code>pub struct Widget&lt;T&gt; { /* private */ }</code></pre>
<section id="impl-Widget" class="impl"><h3 class="code-header">impl&lt;T&gt; Widget&lt;T&gt;</h3>
<section id="method.inspect" class="method"><h4 class="code-header">pub fn <a href="#method.inspect">inspect</a>(&amp;self) -&gt; bool</h4>
<div class="stab unstable">Nightly-only <code>sample_feature</code></div><span class="since" title="Stable since Rust version 1.0.0">1.0.0</span>
</section></section><a class="src" href="../src/lib.rs.html#1-4">Source</a></section></body></html>''').encode()

    def test_literal_declarations_anchors_context_and_stability_preserved(self):
        result = html.parse(self.fixture())
        self.assertEqual(result['declarations'][0]['text'], 'pub struct Widget<T> { /* private */ }')
        method = next(d for d in result['declarations'] if d['section'] == 'method.inspect')
        self.assertEqual(method['text'], 'pub fn inspect(&self) -> bool')
        self.assertTrue(any(a['id'] == 'method.inspect' and a['parent_section'] == 'impl-Widget'
                            for a in result['anchors']))
        self.assertTrue(any('unstable' in a['classes'] and a['section'] == 'method.inspect'
                            for a in result['annotations']))
        self.assertTrue(any(a['title'] == 'Stable since Rust version 1.0.0' for a in result['annotations']))
        self.assertEqual(next(l for l in result['links'] if l['text'] == 'inspect')['section'], 'method.inspect')
        self.assertEqual(result, html.parse(self.fixture()))

    def test_wrong_or_missing_version_rejects(self):
        for version in ('1.99.1 (58cf8f9c3 2026-10-13)', '1.98.1 (000000000 2026-09-01)', ''):
            with self.subTest(version=version), self.assertRaisesRegex(ValueError, 'marker mismatch'):
                html.parse(self.fixture(version))

    def test_page_paths_cannot_escape_pinned_origin(self):
        for path in ('../std/index.html', '/std/index.html', 'https://evil.example/x.html',
                     'core/../../std/index.html', 'core/index.html?bad=1', 'core/index.html#x', 'other/index.html'):
            with self.subTest(path=path), self.assertRaises(ValueError):
                html.page_url(path)
        self.assertEqual(html.page_url('core/option/enum.Option.html'),
                         html.BASE + 'core/option/enum.Option.html')

    def test_wrong_page_version_does_not_publish_output(self):
        with tempfile.TemporaryDirectory(prefix='html-rejection-') as temp:
            output = Path(temp) / 'out'
            with patch.dict(os.environ, {'JCODE_SCRATCH_DIR': temp}), \
                    patch.object(html, 'verify_source_index', return_value={'commit': html.COMMIT}), \
                    patch.object(html, 'fetch', return_value=self.fixture('1.99.1 (58cf8f9c3 2026-10-13)')):
                with self.assertRaisesRegex(ValueError, 'marker mismatch'):
                    html.acquire(output, Path('unused-mocked-source-index'))
            self.assertFalse(output.exists())

    def test_real_source_tree_binding_and_tampered_source_rejected(self):
        source_dir = Path(os.environ.get('JR_COMPAT_LIBRARY_FIXTURES', str(Path(__file__).parent))) / 'pinned-source-acquisition'
        if not source_dir.exists():
            self.skipTest('real pinned-source acquisition not present')
        identity = html.verify_source_index(source_dir / 'library-source-index.json')
        self.assertEqual(identity['commit'], html.COMMIT)
        self.assertIn('library_tree_sha1', identity)
        self.assertIn('source_index_content_sha256', identity)
        with tempfile.TemporaryDirectory(prefix='source-tamper-') as temp:
            copied = Path(temp) / 'source'
            shutil.copytree(source_dir, copied)
            (copied / 'sources/library/core/Cargo.toml').write_bytes(b'tampered')
            with self.assertRaisesRegex(ValueError, 'downloaded source hash mismatch'):
                html.verify_source_index(copied / 'library-source-index.json')

    def test_forged_source_content_and_index_hashes_cannot_bypass_tree(self):
        source_dir = Path(os.environ.get('JR_COMPAT_LIBRARY_FIXTURES', str(Path(__file__).parent))) / 'pinned-source-acquisition'
        if not source_dir.exists():
            self.skipTest('real pinned-source acquisition not present')
        with tempfile.TemporaryDirectory(prefix='forged-source-') as temp:
            copied = Path(temp) / 'source'
            shutil.copytree(source_dir, copied)
            path = copied / 'sources/library/core/Cargo.toml'
            raw = b'forged but internally consistent source'
            path.write_bytes(raw)
            index_path = copied / 'library-source-index.json'
            doc = json.loads(index_path.read_text())
            entry = next(e for e in doc['downloaded_files'] if e['path'] == 'library/core/Cargo.toml')
            entry['git_blob_sha1'] = source.object_hash('blob', raw)
            entry['content_sha256'] = hashlib.sha256(raw).hexdigest()
            index_path.write_text(json.dumps(doc))
            with self.assertRaisesRegex(ValueError, 'path-to-tree binding mismatch'):
                html.verify_source_index(index_path)

    def test_real_root_to_library_binding_rejects_tampered_tree(self):
        source_dir = Path(os.environ.get('JR_COMPAT_LIBRARY_FIXTURES', str(Path(__file__).parent))) / 'pinned-source-acquisition'
        if not source_dir.exists():
            self.skipTest('real pinned-source acquisition not present')
        with tempfile.TemporaryDirectory(prefix='tree-tamper-') as temp:
            copied = Path(temp) / 'source'
            shutil.copytree(source_dir, copied)
            path = copied / 'root-tree.json'
            doc = json.loads(path.read_text())
            next(e for e in doc['tree'] if e['path'] == 'library')['sha'] = '0' * 40
            path.write_text(json.dumps(doc))
            with self.assertRaisesRegex(ValueError, 'root tree hash mismatch'):
                html.verify_source_index(copied / 'library-source-index.json')

    def test_real_pinned_tokenstream_detail(self):
        path = Path(os.environ.get('JR_COMPAT_LIBRARY_FIXTURES', str(Path(__file__).parent))) / 'pinned-html-acquisition/html/proc_macro/struct.TokenStream.html'
        if not path.exists():
            self.skipTest('real pinned HTML acquisition not present')
        raw = path.read_bytes()
        result = html.parse(raw)
        self.assertTrue(any(a['id'] == 'method.new' for a in result['anchors']))
        self.assertTrue(any(d['section'] == 'method.new' and 'new' in d['text'] for d in result['declarations']))
        self.assertTrue(any('TokenStream' in d['text'] for d in result['declarations']))
        self.assertTrue(any('unstable' in a['classes'] for a in result['annotations']))


if __name__ == '__main__':
    unittest.main()
