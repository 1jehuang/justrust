#!/usr/bin/env python3
"""Acquire a pinned Rust library source-file index, NOT a public API enumerator.
Only official GitHub API/raw origins are used. Output must be under the caller's
JCODE_SCRATCH_DIR. No Rust builds, source parsing, imports from Rust or Git writes.
Usage: python3 acquire_pinned_library_sources.py --output "$JCODE_SCRATCH_DIR/pinned-library-source"
The index records all library paths and Git object hashes, including submodule
boundaries. Selected manifests/entry points/licenses are acquired as exact bytes.
Git tree/blob hashes are recomputed, never confused with content SHA256 hashes.
"""
import argparse
import hashlib
import json
import os
from pathlib import Path, PurePosixPath
import sys
import urllib.request

COMMIT = '48a229ceaefd4985c50990b14116b6d856af0985'
RELEASE = '1.98.1'
API = 'https://api.github.com/repos/rust-lang/rust'
RAW = 'https://raw.githubusercontent.com/rust-lang/rust/' + COMMIT + '/'
CRATES = ('core', 'alloc', 'std', 'proc_macro')


def require(ok, message):
    if not ok:
        raise ValueError(message)


def object_hash(kind, data):
    return hashlib.sha1(kind.encode('ascii') + b' ' + str(len(data)).encode('ascii') + b'\0' + data).hexdigest()


def safe_path(value):
    p = PurePosixPath(value)
    require(bool(value) and not p.is_absolute() and '..' not in p.parts and
            '\\' not in value and '\0' not in value, 'unsafe source path')
    return p


def tree_hash(entries):
    """Git orders a directory name as if suffixed by slash, normal files by NUL."""
    require(len({e['path'] for e in entries}) == len(entries), 'duplicate tree path')
    body = bytearray()
    for e in sorted(entries, key=lambda e: e['path'].encode() + (b'/' if e['type'] == 'tree' else b'\0')):
        require('/' not in str(safe_path(e['path'])), 'tree entry must be a direct child')
        require(e['mode'] in ('040000', '100644', '100755', '120000', '160000'), 'unknown git file mode')
        require(len(e['sha']) == 40, 'invalid object hash')
        mode = e['mode'].lstrip('0')
        body.extend(mode.encode('ascii') + b' ' + e['path'].encode('utf-8') + b'\0' + bytes.fromhex(e['sha']))
    return object_hash('tree', bytes(body))


def verify_recursive_tree(doc, expected):
    require(doc.get('truncated') is False, 'recursive GitHub tree truncated')
    require(doc.get('sha') == expected, 'recursive tree identity mismatch')
    require(isinstance(doc.get('tree'), list), 'missing recursive tree entries')
    entries = doc['tree']
    require(len({e['path'] for e in entries}) == len(entries), 'duplicate recursive source path')
    children = {'': []}
    expected_trees = {'': expected}
    for e in entries:
        path = safe_path(e['path'])
        parent = '' if str(path.parent) == '.' else str(path.parent)
        child = dict(e, path=path.name)
        children.setdefault(parent, []).append(child)
        if e['type'] == 'tree':
            expected_trees[str(path)] = e['sha']
            children.setdefault(str(path), [])
    require(set(children) == set(expected_trees), 'missing directory entry in recursive tree')
    for directory, expected_hash in expected_trees.items():
        require(tree_hash(children[directory]) == expected_hash,
                'Git tree hash mismatch: ' + (directory or '<root>'))
    return entries


def fetch(url):
    require(url.startswith(API + '/') or url.startswith(RAW), 'unapproved download origin')
    request = urllib.request.Request(url, headers={'User-Agent': 'pinned-rust-library-inventory/1',
                                                 'Accept': 'application/vnd.github+json'})
    with urllib.request.urlopen(request, timeout=60) as response:
        require(response.geturl().startswith(('https://api.github.com/', 'https://raw.githubusercontent.com/')),
                'download redirected outside official origins')
        data = response.read(32 * 1024 * 1024 + 1)
        require(len(data) <= 32 * 1024 * 1024, 'download exceeds per-response limit')
        return data


def acquire(output):
    scratch = Path(os.environ['JCODE_SCRATCH_DIR']).resolve()
    output = output.resolve()
    require(output != scratch and scratch in output.parents, 'output must be beneath JCODE_SCRATCH_DIR')
    require(not output.exists(), 'output already exists, choose a fresh scratch path')
    commit_url = API + '/git/commits/' + COMMIT
    commit_raw = fetch(commit_url)
    commit_doc = json.loads(commit_raw)
    require(commit_doc.get('sha') == COMMIT, 'commit identity mismatch')
    root_sha = commit_doc['tree']['sha']
    root_url = API + '/git/trees/' + root_sha
    root_raw = fetch(root_url)
    root = json.loads(root_raw)
    require(root.get('sha') == root_sha and root.get('truncated') is False, 'root tree identity mismatch')
    require(tree_hash(root['tree']) == root_sha, 'root Git tree hash mismatch')
    root_entries = {e['path']: e for e in root['tree']}
    library_sha = root_entries['library']['sha']
    library_url = API + '/git/trees/' + library_sha + '?recursive=1'
    library_raw = fetch(library_url)
    library_doc = json.loads(library_raw)
    entries = verify_recursive_tree(library_doc, library_sha)
    full = {'library/' + e['path']: e for e in entries}
    full.update(root_entries)
    selected = ['Cargo.toml', 'LICENSE-MIT', 'LICENSE-APACHE', 'COPYRIGHT', 'library/Cargo.toml']
    selected += ['library/' + c + '/' + f for c in CRATES for f in ('Cargo.toml', 'src/lib.rs')]
    downloaded = []
    material = {}
    for path in selected:
        entry = full[path]
        require(entry['type'] == 'blob' and entry['mode'] in ('100644', '100755'), 'selected source not regular file')
        raw = fetch(RAW + path)
        require(object_hash('blob', raw) == entry['sha'], 'Git blob hash mismatch: ' + path)
        material[path] = raw
        downloaded.append({'path': path, 'git_blob_sha1': entry['sha'],
                           'content_sha256': hashlib.sha256(raw).hexdigest(), 'byte_length': len(raw),
                           'source_url': RAW + path})
    source_records = []
    for e in sorted(entries, key=lambda e: e['path']):
        path = 'library/' + e['path']
        record = {'path': path, 'mode': e['mode'], 'git_object_kind': e['type'],
                  'git_object_sha1': e['sha'], 'source_url': RAW + path if e['type'] == 'blob' else None,
                  'selected_crate': next((c for c in CRATES if e['path'].startswith(c + '/')), None)}
        if 'size' in e:
            record['reported_byte_length'] = e['size']
        source_records.append(record)
    result = {'schema_version': 1, 'review_status': 'unreviewed',
              'scope': {'edition': '2024', 'target': 'x86_64-unknown-linux-gnu'},
              'source_identity': {'repository': 'https://github.com/rust-lang/rust', 'release': RELEASE,
                                  'commit': COMMIT, 'root_tree_sha1': root_sha, 'library_tree_sha1': library_sha},
              'sources': [{'url': commit_url, 'content_sha256': hashlib.sha256(commit_raw).hexdigest()},
                          {'url': root_url, 'content_sha256': hashlib.sha256(root_raw).hexdigest()},
                          {'url': library_url, 'content_sha256': hashlib.sha256(library_raw).hexdigest()}],
              'limitations': [
                  'Source file index only, not public API enumeration, Rust AST parsing, stable item availability or behavior evidence.',
                  'All library source trees and cfg branches are retained. Target-specific reachable APIs require genuine compiler/rustdoc resolution with pinned target configuration.',
                  'Submodules are indexed at their gitlink commit, not recursively acquired. Only recorded gitlink entries are submodule boundaries. Other path-included or reexported code requires reachability review without assuming it is an external submodule.',
                  'Git commit identity is obtained from the official HTTPS API. Commit signature is not required or asserted. Git SHA1 object hashes are recomputed, selected file content SHA256 is recorded separately.',
                  'Only selected manifests, entry sources and licenses are downloaded. Remaining files are indexed by their Git blob identities, not claimed to have local content SHA256 hashes.',
                  'Cargo.toml package version 0.0.0 is not the compiler release. Pinned release is recorded from the compatibility oracle, not inferred from package manifests.'
              ], 'downloaded_files': downloaded, 'library_source_entries': source_records,
              'submodule_boundaries': [r for r in source_records if r['git_object_kind'] == 'commit']}
    # Validation and all network acquisition precede publication of the output tree.
    output.mkdir(parents=True)
    for name, raw in [('commit.json', commit_raw), ('root-tree.json', root_raw), ('library-tree.json', library_raw)]:
        (output / name).write_bytes(raw)
    for path, raw in material.items():
        dest = output / 'sources' / path
        dest.parent.mkdir(parents=True, exist_ok=True)
        dest.write_bytes(raw)
    (output / 'library-source-index.json').write_text(json.dumps(result, indent=2, sort_keys=True) + '\n')
    return result


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--output', type=Path, required=True)
    args = parser.parse_args()
    try:
        result = acquire(args.output)
    except (ValueError, OSError, KeyError, TypeError) as error:
        print('pinned source acquisition rejected: ' + str(error), file=sys.stderr)
        return 2
    print(str(args.output.resolve() / 'library-source-index.json'))
    print('Source indexing only. Public API enumeration and semantic reconciliation remain required.')
    return 0


if __name__ == '__main__':
    sys.exit(main())
