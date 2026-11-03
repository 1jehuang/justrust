#!/usr/bin/env python3
"""Create compact review-only provenance for pinned library source/HTML inputs.
No compatibility state or public API denominator is emitted. Full source/link
sidecars remain scratch artifacts and are bound by byte hashes for reproduction.
Usage: python3 compact_library_surface_provenance.py --surface-index SURFACE_JSON
       --source-index SOURCE_JSON --output "$JCODE_SCRATCH_DIR/.../candidate.json"
"""
import argparse
import hashlib
import json
import os
from pathlib import Path
import sys

import acquire_pinned_api_html as html

TOOLS = ('acquire_pinned_library_sources.py', 'acquire_pinned_api_html.py',
         'compact_library_surface_provenance.py')
MAX_BYTES = 200_000


def sha(raw):
    return hashlib.sha256(raw).hexdigest()


def compact(surface_path, source_path):
    surface_bytes = surface_path.read_bytes()
    source_bytes = source_path.read_bytes()
    surface = json.loads(surface_bytes)
    if surface.get('schema_version') != 1 or surface.get('review_status') != 'unreviewed':
        raise ValueError('expected unreviewed schema1 surface candidate')
    expected_scope = {'edition': '2024', 'target': 'x86_64-unknown-linux-gnu'}
    if surface.get('scope') != expected_scope:
        raise ValueError('surface scope mismatch')
    verified_identity = html.verify_source_index(source_path)
    identity = surface['source_identity']
    for field in ('commit', 'release', 'root_tree_sha1', 'library_tree_sha1', 'source_index_content_sha256'):
        if identity.get(field) != verified_identity[field]:
            raise ValueError('surface-to-source provenance binding mismatch: ' + field)
    if identity.get('official_documentation_root') != html.BASE:
        raise ValueError('unexpected official documentation root')
    pages = []
    paths = set()
    for page in sorted(surface['pages'], key=lambda p: p['path']):
        path = page['path']
        if path in paths:
            raise ValueError('duplicate observed HTML page')
        paths.add(path)
        if html.page_url(path) != page['url']:
            raise ValueError('page path-to-URL mismatch')
        raw = (surface_path.parent / 'html' / path).read_bytes()
        if sha(raw) != page['content_sha256'] or len(raw) != page['byte_length']:
            raise ValueError('page content hash/length mismatch: ' + path)
        actual = html.parse(raw)
        if actual['rustdoc_version'] != page['rustdoc_version']:
            raise ValueError('recorded page version marker mismatch: ' + path)
        pages.append({'url': page['url'], 'path': path, 'content_sha256': sha(raw),
                      'byte_length': len(raw), 'rustdoc_version': actual['rustdoc_version']})
    expected_index_pages = {c + '/' + p for c in html.CRATES for p in ('index.html', 'all.html')}
    if not expected_index_pages <= paths:
        raise ValueError('missing required crate index/all pages')
    tools_root = Path(__file__).resolve().parent
    tools = [{'filename': name, 'content_sha256': sha((tools_root / name).read_bytes())} for name in TOOLS]
    result = {
        'schema_version': 1, 'review_status': 'unreviewed', 'scope': expected_scope,
        'inventory_kind': 'pinned-library-source-and-documentation-input-provenance',
        'source_identity': identity,
        'artifacts': [
            {'id': 'pinned-html-link-anchor-candidate', 'filename': surface_path.name,
             'content_sha256': sha(surface_bytes), 'byte_length': len(surface_bytes),
             'role': 'Full observed literal link/anchor/detail extraction, retained separately in scratch.'},
            {'id': 'pinned-library-source-path-index', 'filename': source_path.name,
             'content_sha256': sha(source_bytes), 'byte_length': len(source_bytes),
             'role': 'Source-file Git object identities, selected exact bytes and source tree evidence, retained separately in scratch.'}
        ],
        'observations': {
            'downloaded_page_records': len(pages),
            'literal_index_link_anchor_candidate_records': len(surface['index_link_candidates']),
            'missing_detail_page_records': len(surface['missing_detail_pages']),
            'interpretation': 'Observed records only, including documentation navigation, aliases, unstable and cross-target entries. These are not API counts, supported-feature counts or a compatibility denominator.'
        },
        'pages': pages,
        'reproduction': {
            'tools': tools,
            'working_directory': 'Directory containing the three recorded Python tools. Use Python3 standard library only.',
            'commands': [
                'PYTHONDONTWRITEBYTECODE=1 python3 acquire_pinned_library_sources.py --output "$JCODE_SCRATCH_DIR/pinned-source"',
                'PYTHONDONTWRITEBYTECODE=1 python3 acquire_pinned_api_html.py --source-index "$JCODE_SCRATCH_DIR/pinned-source/library-source-index.json" --output "$JCODE_SCRATCH_DIR/pinned-html" --all-proc-macro-pages --page core/primitive.u8.html --page core/option/enum.Option.html --page core/iter/trait.Iterator.html --page alloc/vec/struct.Vec.html --page std/io/trait.Read.html --page std/sync/atomic/enum.Ordering.html',
                'PYTHONDONTWRITEBYTECODE=1 python3 compact_library_surface_provenance.py --source-index "$JCODE_SCRATCH_DIR/pinned-source/library-source-index.json" --surface-index "$JCODE_SCRATCH_DIR/pinned-html/pinned-html-api-index.json" --output "$JCODE_SCRATCH_DIR/compiler-library-surface-provenance.json"'
            ],
            'requirements': ['Network access to official GitHub API/raw sources and versioned doc.rust-lang.org.',
                             'Fresh output directories beneath JCODE_SCRATCH_DIR. No builds, Rust AST parsing or checkout writes are needed.',
                             'Retain acquired raw pages/tree responses/selected blobs beside full indices for verification. HTTP content drift changes recorded hashes and must be reviewed rather than silently accepted.']
        },
        'limitations': [
            'Compact input provenance only. No complete API enumeration, stable public target reachability classification, compiler result or compatibility claim.',
            'Versioned rustdoc pages include unstable and cross-target APIs. Literal links, navigation anchors, reexports and duplicate identities are not normalized into API items.',
            'Only downloaded pages are represented here. Missing detail-page records and API-specific tests remain required. The full sidecars, not this compact manifest, retain literal candidate details.',
            'Git root/library tree bindings and selected blob hashes are reverified. HTML version/short-commit markers and page SHA256 bind observed bytes, not a signed source-to-documentation correspondence.',
            'Target scope is the intended comparison cell, not a claim that documentation was filtered to Linux x86-64 or edition2024 stable APIs.',
            'Trait and blanket/auto implementations, inherited/primitive methods, module visibility, cfg, stability and aliases need independent semantic review. No pass state or percentage is emitted.',
            'Full sidecar artifacts remain in scratch initially. Their hashes identify observed candidates, not a guarantee of remote durable availability.'
        ],
        'review_needs': [
            'Classify stable public item identities and target availability from authoritative pinned compiler/rustdoc semantics.',
            'Reconcile reexports, primitive/associated methods, trait/auto/blanket implementations and cfg branches without treating duplicate links as distinct obligations.',
            'Expand per-item behavioral/invalid-input requirements and map exact executable tests. Preserve all missing cells and existing fixture/rejection identities.',
            'Independently review category completeness before considering any inventory exhaustive.'
        ]
    }
    if len(encode(result)) > MAX_BYTES:
        raise ValueError('compact manifest exceeds 200000-byte ceiling')
    return result


def encode(value):
    return (json.dumps(value, indent=2, sort_keys=True) + '\n').encode('utf-8')


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--surface-index', type=Path, required=True)
    parser.add_argument('--source-index', type=Path, required=True)
    parser.add_argument('--output', type=Path, required=True)
    args = parser.parse_args()
    try:
        output = args.output.resolve()
        scratch = Path(os.environ['JCODE_SCRATCH_DIR']).resolve()
        if output == scratch or scratch not in output.parents or output.exists():
            raise ValueError('choose a fresh output file beneath JCODE_SCRATCH_DIR')
        value = compact(args.surface_index, args.source_index)
        output.parent.mkdir(parents=True, exist_ok=True)
        output.write_bytes(encode(value))
    except (ValueError, OSError, KeyError, TypeError) as error:
        print('compact provenance rejected: ' + str(error), file=sys.stderr)
        return 2
    print(str(output))
    return 0


if __name__ == '__main__':
    sys.exit(main())
