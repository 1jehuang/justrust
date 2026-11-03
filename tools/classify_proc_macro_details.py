#!/usr/bin/env python3
"""Bounded literal proc_macro HTML classification, not Rust/API resolution.
Reads unchanged retained pinned HTML/source sidecars and compact provenance.
Writes one fresh scratch JSON sidecar. Stability labels mean explicit scoped
rustdoc annotations only, never language support/public reachability credit.
"""
import argparse
import hashlib
from html.parser import HTMLParser
import json
import os
from pathlib import Path
import sys

import acquire_pinned_api_html as pinned

VOID = pinned.VOID
ASSOCIATED = ('method.', 'tymethod.', 'associatedtype.', 'associatedconstant.', 'variant.', 'structfield.', 'impl-')


class Node:
    def __init__(self, tag='', attrs=None, parent=None):
        self.tag, self.attrs, self.parent = tag, dict(attrs or []), parent
        self.children = []

    def walk(self):
        yield self
        for child in self.children:
            if isinstance(child, Node):
                yield from child.walk()

    def text(self):
        def texts(node):
            for child in node.children:
                if isinstance(child, str):
                    yield child
                else:
                    yield from texts(child)
        return ' '.join(''.join(texts(self)).split())

    def ancestors(self):
        current = self.parent
        while current is not None:
            yield current
            current = current.parent

    def classes(self):
        return set(self.attrs.get('class', '').split())


class DOM(HTMLParser):
    def __init__(self):
        super().__init__(convert_charrefs=True)
        self.root = Node()
        self.stack = [self.root]

    def handle_starttag(self, tag, attrs):
        node = Node(tag, attrs, self.stack[-1])
        self.stack[-1].children.append(node)
        if tag not in VOID:
            self.stack.append(node)

    def handle_startendtag(self, tag, attrs):
        self.handle_starttag(tag, attrs)
        if tag not in VOID:
            self.handle_endtag(tag)

    def handle_endtag(self, tag):
        for i in range(len(self.stack) - 1, 0, -1):
            if self.stack[i].tag == tag:
                del self.stack[i:]
                return

    def handle_data(self, data):
        self.stack[-1].children.append(data)


def owned_section(node):
    """Scope header/sibling item-info to the literal summary's item section.
    A docblock outside a header/summary is deliberately not stability evidence.
    """
    for ancestor in (node, *node.ancestors()):
        if ancestor.tag == 'section' and ancestor.attrs.get('id', '').startswith(ASSOCIATED):
            return ancestor.attrs['id'], 'item-section'
        if ancestor.tag == 'summary':
            items = [n for n in ancestor.walk() if n.tag == 'section' and
                     n.attrs.get('id', '').startswith(ASSOCIATED)]
            if len(items) == 1:
                return items[0].attrs['id'], 'item-summary'
            return None, 'ambiguous-summary-ownership'
        if 'docblock' in ancestor.classes() or ancestor.tag == 'details':
            return None, 'outside-item-header'
    # Top-level item metadata occurs in main-heading or direct item-info.
    heading = any('main-heading' in n.classes() for n in (node, *node.ancestors()))
    direct_info = any('item-info' in n.classes() and n.parent is not None and
                      n.parent.attrs.get('id') == 'main-content' for n in (node, *node.ancestors()))
    if heading or direct_info:
        return 'main-content', 'top-level-item-metadata'
    return None, 'unresolved-owner'


def classify_page(raw, path, url):
    pinned.parse(raw)  # Mandatory pinned version marker, independent of category evidence.
    dom = DOM()
    dom.feed(raw.decode('utf-8'))
    nodes = list(dom.root.walk())
    main = [n for n in nodes if n.tag == 'section' and n.attrs.get('id') == 'main-content']
    if len(main) != 1:
        raise ValueError('expected exactly one main-content section: ' + path)
    main = main[0]
    sections = {'main-content': main}
    for node in main.walk():
        anchor = node.attrs.get('id', '')
        if node.tag == 'section' and anchor.startswith(ASSOCIATED):
            if anchor in sections:
                raise ValueError('duplicate item anchor: ' + path + '#' + anchor)
            sections[anchor] = node
    annotations = {anchor: [] for anchor in sections}
    unowned = []
    for node in main.walk():
        classes = node.classes()
        if 'stab' not in classes and not (node.tag == 'span' and 'since' in classes):
            continue
        owner, basis = owned_section(node)
        evidence = {'tag': node.tag, 'classes': sorted(classes), 'title': node.attrs.get('title'),
                    'text': node.text(), 'ownership_basis': basis}
        if owner in annotations:
            annotations[owner].append(evidence)
        else:
            unowned.append(evidence)
    records = []
    for anchor, section in sections.items():
        declarations = []
        for node in section.walk():
            is_decl = (node.tag == 'pre' and 'item-decl' in node.classes()) or (
                node.tag in ('h3', 'h4') and 'code-header' in node.classes())
            if not is_decl:
                continue
            if anchor == 'main-content':
                # Only the direct top-level declaration, never nested impl/method headers.
                if node.tag != 'pre' or node.parent is not main:
                    continue
            elif owned_section(node)[0] != anchor:
                continue
            declarations.append(node.text())
        evidence = annotations[anchor]
        stable = [e for e in evidence if 'since' in e['classes'] and
                  (e['title'] or '').startswith('Stable since Rust version ')]
        unstable = [e for e in evidence if 'stab' in e['classes'] and 'unstable' in e['classes']]
        if stable and not unstable:
            status = 'stable'
        elif unstable and not stable:
            status = 'unstable'
        else:
            status = 'ambiguous'
        categories = [status]
        if anchor != 'main-content':
            categories.append('associated')
        explicit_reexport = 'reexport' in section.classes() or anchor.startswith('reexport.') or any(
            d.startswith('pub use ') for d in declarations)
        if explicit_reexport:
            categories.append('reexport')
        const_unstable = any('const unstable' in (e['title'] or '') or 'const: unstable' in e['text'] for e in stable)
        sources = []
        for node in section.walk():
            if node.tag == 'a' and 'src' in node.classes():
                if anchor == 'main-content' and not any('main-heading' in a.classes() for a in node.ancestors()):
                    continue
                if anchor != 'main-content' and owned_section(node)[0] != anchor:
                    continue
                sources.append(node.attrs.get('href'))
        records.append({'page': path, 'url': url + '#' + anchor, 'anchor': anchor,
                        'categories': categories, 'literal_stability': status,
                        'classification_basis': 'Only explicit stability annotations scoped to this literal item header/summary. Missing or conflicting annotations remain ambiguous.',
                        'declarations': declarations, 'stability_evidence': evidence,
                        'const_stability': 'unstable-explicit' if const_unstable else 'unresolved',
                        'source_links': sorted(set(sources)),
                        'reexport_review': 'literal-marker-observed' if explicit_reexport else 'unresolved',
                        'public_reachability': 'unresolved', 'target_availability': 'unresolved',
                        'semantic_item_identity': 'unresolved',
                        'page_content_sha256': hashlib.sha256(raw).hexdigest()})
    return records, unowned


def build(surface_path, source_path, provenance_path):
    surface_raw, source_raw, provenance_raw = [p.read_bytes() for p in (surface_path, source_path, provenance_path)]
    surface, provenance = json.loads(surface_raw), json.loads(provenance_raw)
    if provenance.get('schema_version') != 1 or provenance.get('review_status') != 'unreviewed':
        raise ValueError('expected unreviewed schema1 compact provenance')
    expected_artifacts = {a['id']: a for a in provenance['artifacts']}
    for artifact_id, raw in [('pinned-html-link-anchor-candidate', surface_raw), ('pinned-library-source-path-index', source_raw)]:
        entry = expected_artifacts[artifact_id]
        if hashlib.sha256(raw).hexdigest() != entry['content_sha256'] or len(raw) != entry['byte_length']:
            raise ValueError('retained candidate artifact digest/length mismatch: ' + artifact_id)
    verified = pinned.verify_source_index(source_path)
    for data in (surface, provenance):
        for field in ('commit', 'release', 'root_tree_sha1', 'library_tree_sha1', 'source_index_content_sha256'):
            if data['source_identity'].get(field) != verified[field]:
                raise ValueError('pinned source identity mismatch: ' + field)
    if surface.get('scope') != {'edition': '2024', 'target': 'x86_64-unknown-linux-gnu'}:
        raise ValueError('surface comparison scope mismatch')
    pages = {p['path']: p for p in provenance['pages']}
    if len(pages) != len(provenance['pages']):
        raise ValueError('duplicate page in compact provenance')
    selected = [p for p in surface['pages'] if p['path'].startswith('proc_macro/')]
    if len({p['path'] for p in selected}) != len(selected):
        raise ValueError('duplicate retained proc_macro page')
    if not selected or {p['path'] for p in selected} != {p for p in pages if p.startswith('proc_macro/')}:
        raise ValueError('retained proc_macro page set mismatch')
    exclusions, records, page_identities, unowned = [], [], [], []
    for page in sorted(selected, key=lambda p: p['path']):
        path = page['path']
        if pinned.page_url(path) != page['url']:
            raise ValueError('page URL mismatch')
        expected = pages[path]
        for field in ('url', 'content_sha256', 'byte_length', 'rustdoc_version'):
            if expected[field] != page[field]:
                raise ValueError('page metadata mismatch: ' + path)
        raw = (surface_path.parent / 'html' / path).read_bytes()
        if hashlib.sha256(raw).hexdigest() != page['content_sha256'] or len(raw) != page['byte_length']:
            raise ValueError('page bytes changed: ' + path)
        parsed = pinned.parse(raw)
        if parsed['rustdoc_version'] != page['rustdoc_version']:
            raise ValueError('page version metadata mismatch')
        page_identities.append({k: page[k] for k in ('path', 'url', 'content_sha256', 'byte_length', 'rustdoc_version')})
        if path.endswith(('/index.html', '/all.html')):
            exclusions.append({'path': path, 'reason': 'Module/all-item listing page, not a detail-item classification. Listing-row annotations do not establish the enclosing module stability.'})
            continue
        items, unknown = classify_page(raw, path, page['url'])
        records.extend(items)
        unowned.extend({'page': path, **e} for e in unknown)
    return {'schema_version': 1, 'review_status': 'unreviewed',
            'inventory_kind': 'bounded-literal-proc-macro-detail-review', 'scope': surface['scope'],
            'source_identity': surface['source_identity'],
            'input_artifacts': [{'filename': p.name, 'content_sha256': hashlib.sha256(raw).hexdigest(), 'byte_length': len(raw)}
                                for p, raw in ((surface_path, surface_raw), (source_path, source_raw), (provenance_path, provenance_raw))],
            'classifier': {'filename': Path(__file__).name, 'content_sha256': hashlib.sha256(Path(__file__).read_bytes()).hexdigest()},
            'pages': page_identities, 'exclusions': exclusions, 'records': records,
            'unowned_stability_annotations': unowned,
            'limitations': [
                'A first bounded literal documentation review substrate, not complete proc_macro/API enumeration or supported compiler behavior.',
                'stable/unstable labels refer only to explicit item-scoped rustdoc annotations. They do not establish target availability, public reachability, semantic API identity or support credit.',
                'Associated-item shape is retained independently of stability. Inherited/blanket/auto trait methods may lack direct annotations and remain ambiguous, without inferring stability from their enclosing impl or type.',
                'Stable runtime items with explicit const-unstable titles retain stable runtime annotation and a separate const instability observation.',
                'Reexport category requires an explicit literal marker/declaration. Otherwise reexports remain unresolved, even when source links point into another source module.',
                'Only retained proc_macro detail pages are classified. Listing pages are verified but excluded. No fresh downloads, Rust parsing, cfg evaluation or generated test claims are performed.',
                'Raw source/HTML candidates remain unchanged and are hash-bound. Neither literal record counts nor stability labels are an API completeness denominator.'
            ],
            'review_needs': ['Resolve target/public availability using authoritative pinned compiler semantics.',
                             'Review reexport identities, inferred/inherited stability and duplicate trait implementation records.',
                             'Link per-item behavioral and invalid-input obligations to exact executable tests without dropping missing cells.']}


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--surface-index', type=Path, required=True)
    parser.add_argument('--source-index', type=Path, required=True)
    parser.add_argument('--provenance', type=Path, required=True)
    parser.add_argument('--output', type=Path, required=True)
    args = parser.parse_args()
    try:
        output = args.output.resolve()
        scratch = Path(os.environ['JCODE_SCRATCH_DIR']).resolve()
        if output == scratch or scratch not in output.parents or output.exists():
            raise ValueError('choose a fresh output file beneath JCODE_SCRATCH_DIR')
        result = build(args.surface_index, args.source_index, args.provenance)
        output.parent.mkdir(parents=True, exist_ok=True)
        output.write_text(json.dumps(result, indent=2, sort_keys=True) + '\n')
    except (ValueError, OSError, KeyError, TypeError) as error:
        print('literal proc_macro classification rejected: ' + str(error), file=sys.stderr)
        return 2
    print(str(output))
    return 0


if __name__ == '__main__':
    sys.exit(main())
