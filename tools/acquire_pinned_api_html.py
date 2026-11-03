#!/usr/bin/env python3
"""Acquire actual pinned official rustdoc HTML and extract literal links/anchors.
This is an HTML indexer, not a Rust parser or exhaustive stable public API proof.
All four crate index/all pages are fetched. --page adds explicit item pages.
--all-proc-macro-pages fetches the bounded item links in proc_macro/all.html.
Writes only to a fresh directory beneath JCODE_SCRATCH_DIR. No Rust builds.
"""
import argparse
import hashlib
from html.parser import HTMLParser
import json
import os
from pathlib import Path, PurePosixPath
import sys
from urllib.parse import urljoin, urlparse, urldefrag
from urllib.request import Request, urlopen

import acquire_pinned_library_sources as pinned_source

RELEASE = '1.98.1'
COMMIT = '48a229ceaefd4985c50990b14116b6d856af0985'
BASE = 'https://doc.rust-lang.org/' + RELEASE + '/'
CRATES = ('core', 'alloc', 'std', 'proc_macro')
VOID = {'area', 'base', 'br', 'col', 'embed', 'hr', 'img', 'input', 'link', 'meta', 'param', 'source', 'track', 'wbr'}


class RustdocHTML(HTMLParser):
    def __init__(self):
        super().__init__(convert_charrefs=True)
        self.version = None
        self.frames = []
        self.links = []
        self.anchors = []
        self.declarations = []
        self.annotations = []
        self.headings = []

    def context(self):
        return next((f['section'] for f in reversed(self.frames) if f.get('section')), None)

    def handle_starttag(self, tag, attrs):
        attrs = dict(attrs)
        if 'data-rustdoc-version' in attrs:
            self.version = attrs['data-rustdoc-version']
        classes = attrs.get('class', '').split()
        frame = {'tag': tag, 'capture': None}
        if tag == 'section' and attrs.get('id'):
            frame['section'] = attrs['id']
        if attrs.get('id') and (tag == 'section' or tag.startswith('h')):
            self.anchors.append({'tag': tag, 'id': attrs['id'], 'classes': classes,
                                 'parent_section': self.context()})
        context = frame.get('section') or self.context()
        if tag == 'a' and attrs.get('href'):
            link = {'href': attrs['href'], 'text': '', 'classes': classes, 'section': context}
            self.links.append(link)
            frame['capture'] = link
        elif (tag == 'pre' and 'item-decl' in classes) or (tag in ('h3', 'h4') and 'code-header' in classes):
            declaration = {'section': context, 'tag': tag, 'text': ''}
            self.declarations.append(declaration)
            frame['capture'] = declaration
        elif 'stab' in classes or (tag == 'span' and 'since' in classes):
            annotation = {'section': context, 'classes': classes, 'text': '',
                          'title': attrs.get('title')}
            self.annotations.append(annotation)
            frame['capture'] = annotation
        elif tag in ('h1', 'h2'):
            heading = {'id': attrs.get('id'), 'text': ''}
            self.headings.append(heading)
            frame['capture'] = heading
        if tag not in VOID:
            self.frames.append(frame)

    def handle_startendtag(self, tag, attrs):
        self.handle_starttag(tag, attrs)
        if tag not in VOID:
            self.handle_endtag(tag)

    def handle_endtag(self, tag):
        # Rustdoc emits normal HTML. Recover from optional closing elements
        # conservatively without assigning semantic Rust identities.
        for i in range(len(self.frames) - 1, -1, -1):
            if self.frames[i]['tag'] == tag:
                del self.frames[i:]
                break

    def handle_data(self, text):
        for frame in self.frames:
            if frame['capture'] is not None:
                frame['capture']['text'] += text

    def result(self):
        for rows in (self.links, self.declarations, self.annotations, self.headings):
            for row in rows:
                row['text'] = ' '.join(row['text'].split())
        return {'rustdoc_version': self.version, 'links': self.links, 'anchors': self.anchors,
                'declarations': self.declarations, 'annotations': self.annotations, 'headings': self.headings}


def parse(raw):
    parser = RustdocHTML()
    parser.feed(raw.decode('utf-8'))
    result = parser.result()
    version = result['rustdoc_version'] or ''
    if not version.startswith(RELEASE + ' (' + COMMIT[:9] + ' '):
        raise ValueError('pinned rustdoc version/commit marker mismatch: ' + repr(version))
    return result


def page_url(path):
    parsed = PurePosixPath(path)
    if parsed.is_absolute() or '..' in parsed.parts or '\\' in path or not path.endswith('.html') or parsed.parts[0] not in CRATES:
        raise ValueError('page must be a relative core/alloc/std/proc_macro HTML path')
    url = urljoin(BASE, path)
    if not url.startswith(BASE) or urlparse(url).query or urlparse(url).fragment:
        raise ValueError('invalid page URL')
    return url


def fetch(path):
    url = page_url(path)
    with urlopen(Request(url, headers={'User-Agent': 'pinned-rustdoc-inventory/1'}), timeout=60) as response:
        if response.geturl() != url:
            raise ValueError('versioned documentation redirected: ' + response.geturl())
        data = response.read(16 * 1024 * 1024 + 1)
        if len(data) > 16 * 1024 * 1024:
            raise ValueError('HTML page exceeds per-page limit')
        return data


def verify_source_index(path):
    raw = path.read_bytes()
    index = json.loads(raw)
    identity = index['source_identity']
    if identity.get('commit') != COMMIT or identity.get('release') != RELEASE:
        raise ValueError('pinned library source identity mismatch')
    root = path.parent
    commit = json.loads((root / 'commit.json').read_bytes())
    root_tree = json.loads((root / 'root-tree.json').read_bytes())
    library_tree = json.loads((root / 'library-tree.json').read_bytes())
    if commit.get('sha') != COMMIT or commit['tree']['sha'] != identity['root_tree_sha1']:
        raise ValueError('pinned commit-to-tree binding mismatch')
    if pinned_source.tree_hash(root_tree['tree']) != identity['root_tree_sha1']:
        raise ValueError('pinned root tree hash mismatch')
    library_entry = next(e for e in root_tree['tree'] if e['path'] == 'library')
    if library_entry['sha'] != identity['library_tree_sha1']:
        raise ValueError('pinned root-to-library tree binding mismatch')
    pinned_source.verify_recursive_tree(library_tree, identity['library_tree_sha1'])
    verified_entries = {e['path']: e for e in root_tree['tree']}
    verified_entries.update({'library/' + e['path']: e for e in library_tree['tree']})
    for entry in index['downloaded_files']:
        pinned_source.safe_path(entry['path'])
        expected = verified_entries.get(entry['path'])
        if expected is None or expected['type'] != 'blob' or expected['sha'] != entry['git_blob_sha1']:
            raise ValueError('downloaded source path-to-tree binding mismatch: ' + entry['path'])
        content = (root / 'sources' / entry['path']).read_bytes()
        if (pinned_source.object_hash('blob', content) != entry['git_blob_sha1'] or
                hashlib.sha256(content).hexdigest() != entry['content_sha256']):
            raise ValueError('pinned downloaded source hash mismatch: ' + entry['path'])
    return {**identity, 'source_index_content_sha256': hashlib.sha256(raw).hexdigest(),
            'verification': 'commit/root/library tree bindings and Git tree hashes recomputed, selected blobs rehashed'}


def acquire(output, source_index, extra_pages=(), proc_macro=False):
    scratch = Path(os.environ['JCODE_SCRATCH_DIR']).resolve()
    output = output.resolve()
    if output == scratch or scratch not in output.parents or output.exists():
        raise ValueError('choose a fresh output directory beneath JCODE_SCRATCH_DIR')
    source_identity = verify_source_index(source_index)
    materials = {}
    records = {}

    def read(path):
        if path not in records:
            raw = fetch(path)
            parsed = parse(raw)
            materials[path] = raw
            records[path] = {'path': path, 'url': page_url(path),
                             'content_sha256': hashlib.sha256(raw).hexdigest(), 'byte_length': len(raw), **parsed}

    for crate in CRATES:
        for name in ('index.html', 'all.html'):
            read(crate + '/' + name)
    candidates = {}
    for path, record in list(records.items()):
        for link in record['links']:
            resolved = urljoin(page_url(path), link['href'])
            target, anchor = urldefrag(resolved)
            if target.startswith(BASE) and urlparse(target).path.endswith('.html'):
                relative = target[len(BASE):]
                if relative.split('/')[0] not in CRATES:
                    continue
                candidate = candidates.setdefault((target, anchor), {'url': target, 'anchor': anchor,
                            'path': relative, 'labels': [], 'index_pages': [],
                            'stable_api_review': 'required', 'target_availability_review': 'required'})
                if link['text'] not in candidate['labels']:
                    candidate['labels'].append(link['text'])
                if path not in candidate['index_pages']:
                    candidate['index_pages'].append(path)
    requested = set(extra_pages)
    if proc_macro:
        requested.update(r['path'] for r in candidates.values() if r['path'].startswith('proc_macro/'))
    for path in sorted(requested):
        read(path)
    result = {'schema_version': 1, 'review_status': 'unreviewed',
              'scope': {'edition': '2024', 'target': 'x86_64-unknown-linux-gnu'},
              'source_identity': {**source_identity, 'official_documentation_root': BASE},
              'limitations': [
                  'Actual HTML link/anchor inventory, not a Rust AST parser or exhaustive public API/stability proof.',
                  'all.html and index pages include unstable and cross-target items. No link is silently counted as a stable Linux API. Stability/target annotations remain literal review evidence.',
                  'Only requested detail pages are downloaded. Other candidates remain explicit missing-detail cells. Module/reexport paths and methods require further resolution.',
                  'Blanket/auto trait impls, aliases, inherited methods and duplicated anchors are preserved as literal documentation records, not independent API counts.',
                  'Version/short-commit markers are checked on every downloaded page. HTTPS page hashes preserve observed content, not a signed mapping to the full Git source commit.',
                  'No test or compiler support result is emitted. Full pinned API enumeration and per-item behavioral/test reconciliation remain required.'
              ], 'index_link_candidates': sorted(candidates.values(), key=lambda r: (r['url'], r['anchor'])),
              'pages': [records[p] for p in sorted(records)],
              'missing_detail_pages': sorted({r['path'] for r in candidates.values()} - set(records))}
    output.mkdir(parents=True)
    for path, raw in materials.items():
        dest = output / 'html' / path
        dest.parent.mkdir(parents=True, exist_ok=True)
        dest.write_bytes(raw)
    (output / 'pinned-html-api-index.json').write_text(json.dumps(result, indent=2, sort_keys=True) + '\n')
    return result


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--output', type=Path, required=True)
    parser.add_argument('--source-index', type=Path, required=True)
    parser.add_argument('--page', action='append', default=[])
    parser.add_argument('--all-proc-macro-pages', action='store_true')
    args = parser.parse_args()
    try:
        acquire(args.output, args.source_index, args.page, args.all_proc_macro_pages)
    except (ValueError, OSError, KeyError, TypeError) as error:
        print('pinned HTML acquisition rejected: ' + str(error), file=sys.stderr)
        return 2
    print(str(args.output.resolve() / 'pinned-html-api-index.json'))
    return 0


if __name__ == '__main__':
    sys.exit(main())
