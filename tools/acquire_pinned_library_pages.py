#!/usr/bin/env python3
"""Acquire exactly three pinned library rustdoc pages, not execution evidence.

All bytes are authenticated before creating a fresh output directory. Existing
parents must be real directories, and inputs must be regular non-symlink files.
Publication is not atomic. Consumers must wait for success. Hostile concurrent
writers with access to the destination parent are outside this threat model.
No external full sidecars or authored source fixture snapshots are required.
"""
from __future__ import annotations

import argparse
import hashlib
import json
import os
from pathlib import Path
import re
import stat
import sys
import urllib.request

import library_witness_sources as witness
from acquire_pinned_reference import _directory, _parent_fd, _remove_owned

_ROOT = Path(witness.__file__).absolute().parent.parent
_LIMIT = 1_000_000
_METADATA_LIMIT = 1_000_000
_VERSION = '1.98.1 (48a229cea 2026-09-01)'
_BASE = 'https://doc.rust-lang.org/1.98.1/'
_ANCHORS = {
    'alloc/vec/struct.Vec.html': ('method.swap_remove', 'method.truncate', 'method.into_iter'),
    'core/iter/trait.Iterator.html': ('method.nth', 'tymethod.next'),
    'core/option/enum.Option.html': ('method.replace', 'method.take'),
}


def _require(ok, message):
    if not ok:
        raise ValueError(message)


def _absolute(path):
    path = Path(path)
    _require('..' not in path.parts, 'parent traversal is forbidden')
    return Path(os.path.abspath(path))


def _read_file(path, limit):
    """Bound the read, refuse symlinks in every component and special files."""
    path = _absolute(path)
    flags = os.O_RDONLY | os.O_DIRECTORY | os.O_NOFOLLOW
    parent = os.open('/', flags)
    try:
        for part in path.parts[1:-1]:
            child = os.open(part, flags, dir_fd=parent)
            os.close(parent)
            parent = child
        fd = os.open(path.name, os.O_RDONLY | os.O_NOFOLLOW | os.O_NONBLOCK, dir_fd=parent)
        with os.fdopen(fd, 'rb') as stream:
            info = os.fstat(stream.fileno())
            _require(stat.S_ISREG(info.st_mode), 'input must be a regular file')
            _require(info.st_size <= limit, 'input exceeds byte limit')
            raw = stream.read(limit + 1)
            _require(len(raw) <= limit, 'input exceeds byte limit')
            return raw
    finally:
        os.close(parent)


def _unique_object(pairs):
    result = {}
    for key, value in pairs:
        _require(key not in result, 'duplicate metadata key')
        result[key] = value
    return result


def _pages():
    """Use only the small raw-hash-authenticated manifest, not sidecar claims."""
    raw = _read_file(_ROOT / witness.PROVENANCE, _METADATA_LIMIT)
    _require(hashlib.sha256(raw).hexdigest() == witness.PINS[witness.PROVENANCE],
             'compact provenance pin mismatch')
    try:
        manifest = json.loads(raw, object_pairs_hook=_unique_object)
        _require(type(manifest) is dict and type(manifest['pages']) is list,
                 'malformed compact provenance')
        seen, selected = set(), {}
        for page in manifest['pages']:
            _require(type(page) is dict and type(page.get('path')) is str,
                     'malformed page record')
            path = page['path']
            _require(path not in seen, 'duplicate page record')
            seen.add(path)
            if path not in _ANCHORS:
                continue
            _require(set(page) == {'path', 'url', 'byte_length', 'content_sha256', 'rustdoc_version'},
                     'malformed selected page record')
            _require(type(page['byte_length']) is int and 0 < page['byte_length'] <= _LIMIT,
                     'invalid pinned byte length')
            _require(type(page['content_sha256']) is str and
                     re.fullmatch('[0-9a-f]{64}', page['content_sha256']) is not None,
                     'invalid pinned digest')
            _require(page['url'] == _BASE + path, 'foreign documentation URL')
            _require(page['rustdoc_version'] == _VERSION, 'pinned version drift')
            selected[path] = page
        _require(set(selected) == set(_ANCHORS), 'missing selected page record')
        return [selected[path] for path in sorted(selected)]
    except (KeyError, TypeError, UnicodeError, RecursionError) as error:
        raise ValueError('malformed compact provenance') from error


class _NoRedirect(urllib.request.HTTPRedirectHandler):
    def redirect_request(self, req, fp, code, msg, headers, newurl):
        raise ValueError('documentation redirects are forbidden')


def _download(page):
    _require(page['path'] in _ANCHORS and page['url'] == _BASE + page['path'],
             'foreign documentation URL')
    opener = urllib.request.build_opener(_NoRedirect())
    request = urllib.request.Request(page['url'], headers={
        'Accept-Encoding': 'identity', 'User-Agent': 'pinned-library-pages-input/1'})
    with opener.open(request, timeout=30) as response:
        _require(response.geturl() == page['url'], 'unexpected final documentation URL')
        _require(response.status == 200, 'unexpected documentation response status')
        _require(response.headers.get('Content-Encoding', 'identity').lower() == 'identity',
                 'encoded documentation response is forbidden')
        length = response.headers.get('Content-Length')
        _require(length is None or length == str(page['byte_length']),
                 'unexpected documentation content length')
        raw = response.read(page['byte_length'] + 1)
        _require(len(raw) <= page['byte_length'], 'documentation response exceeds limit')
        return raw


def acquire(output_dir, html_root=None) -> dict:
    """Verify all three pages and seven IDs, then write only to new output/html."""
    output = _absolute(output_dir)
    parent_fd = _parent_fd(output)  # Reject existing output before any input I/O.
    owned = None
    try:
        pages = _pages()
        source = None if html_root is None else _absolute(html_root)
        payloads = []
        for page in pages:
            raw = (_download(page) if source is None else
                   _read_file(source / page['path'], page['byte_length']))
            witness.verify_html(raw, page, _ANCHORS[page['path']])
            payloads.append((page['path'], raw))
        os.mkdir(output.name, mode=0o700, dir_fd=parent_fd)
        owned = os.stat(output.name, dir_fd=parent_fd, follow_symlinks=False)
        root_fd = os.open(output.name, os.O_RDONLY | os.O_DIRECTORY | os.O_NOFOLLOW,
                          dir_fd=parent_fd)
        try:
            for path, raw in payloads:
                parts = ('html/' + path).split('/')
                directory = _directory(root_fd, parts[:-1])
                try:
                    fd = os.open(parts[-1], os.O_WRONLY | os.O_CREAT | os.O_EXCL |
                                 os.O_NOFOLLOW, 0o600, dir_fd=directory)
                    with os.fdopen(fd, 'wb') as stream:
                        stream.write(raw)
                finally:
                    os.close(directory)
        finally:
            os.close(root_fd)
        return {'readiness': 'input-verified', 'verified_pages': 3,
                'verified_anchors': 7, 'html_root': str(output / 'html'),
                'compatibility_credit': False}
    except BaseException:
        if owned is not None:
            try:
                current = os.stat(output.name, dir_fd=parent_fd, follow_symlinks=False)
            except FileNotFoundError:
                current = None
            if current is not None and stat.S_ISDIR(current.st_mode) and (
                    current.st_dev, current.st_ino) == (owned.st_dev, owned.st_ino):
                _remove_owned(parent_fd, output.name)
        raise
    finally:
        os.close(parent_fd)


def main(argv=None):
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--output-dir', type=Path, required=True)
    parser.add_argument('--html-root', type=Path)
    args = parser.parse_args(argv)
    try:
        result = acquire(args.output_dir, args.html_root)
    except (OSError, ValueError) as error:
        print('Library page acquisition rejected: ' + str(error), file=sys.stderr)
        return 2
    print(json.dumps(result, sort_keys=True, allow_nan=False))
    return 0


if __name__ == '__main__':
    sys.exit(main())
