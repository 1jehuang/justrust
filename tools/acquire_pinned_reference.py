#!/usr/bin/env python3
"""Acquire fixed official Reference inputs, not compiler compatibility evidence.

Only new output directories are accepted. Parents must already exist and may not
be symlinks. Compressed input is limited to 16 MiB, decompressed tar bytes
(including headers/padding) to 32 MiB, entries to 1000, files to 4 MiB.
No archive permissions, ownership, timestamps or executable bits are restored.
Publication is not atomic: consumers must wait for successful return. A hostile
process with write access to output ancestors is outside the verification model.
"""
import argparse
import gzip
import hashlib
import io
import json
import os
from pathlib import Path
import stat
import sys
import tarfile
import urllib.request

from extract_reference_rules import verify_archive

_COMMIT = '86635e30bf861a038dc197d7e16fd09e7e514e7a'
_SHA256 = '580d2250b29ac221aefc647c5e2c3e9d648ba833b6634e41105de2de1316cd91'
_URL = 'https://codeload.github.com/rust-lang/reference/tar.gz/' + _COMMIT
_COMPRESSED = 16 * 1024 * 1024
_UNCOMPRESSED = 32 * 1024 * 1024
_FILE = 4 * 1024 * 1024
_ENTRIES = 1000


def _require(ok, message):
    if not ok:
        raise ValueError(message)


class _NoRedirect(urllib.request.HTTPRedirectHandler):
    def redirect_request(self, req, fp, code, msg, headers, newurl):
        raise ValueError('archive redirects are forbidden')


def _download():
    # No authorization headers, credential handlers or caller-selected URL.
    opener = urllib.request.build_opener(_NoRedirect())
    request = urllib.request.Request(_URL, headers={'User-Agent': 'pinned-reference-input/1'})
    with opener.open(request, timeout=60) as response:
        _require(response.geturl() == _URL, 'unexpected final archive URL')
        return response.read(_COMPRESSED + 1)


def _parent_fd(output):
    """Walk without following symlinks, retaining the exact destination parent."""
    flags = os.O_RDONLY | os.O_DIRECTORY | os.O_NOFOLLOW
    fd = os.open('/', flags)
    try:
        for part in output.parts[1:-1]:
            child = os.open(part, flags, dir_fd=fd)
            os.close(fd)
            fd = child
        try:
            os.stat(output.name, dir_fd=fd, follow_symlinks=False)
        except FileNotFoundError:
            return fd
        raise FileExistsError('output already exists: ' + str(output))
    except BaseException:
        os.close(fd)
        raise


def _members(raw):
    # Bound decompression before tarfile can consume extended headers or padding.
    with gzip.GzipFile(fileobj=io.BytesIO(raw)) as compressed:
        data = compressed.read(_UNCOMPRESSED + 1)
    _require(len(data) <= _UNCOMPRESSED, 'decompressed archive exceeds limit')
    prefix = 'reference-' + _COMMIT
    entries = {}
    total = 0
    with tarfile.open(fileobj=io.BytesIO(data), mode='r:') as archive:
        for member in archive:
            _require(len(entries) < _ENTRIES, 'archive entry count exceeds limit')
            name = member.name
            # One terminal slash is conventional for directory tar entries.
            if member.isdir() and name.endswith('/'):
                name = name[:-1]
            parts = name.split('/')
            _require(all(p and p not in ('.', '..') for p in parts) and
                     '\\' not in name and '\0' not in name and parts[0] == prefix,
                     'unsafe or unexpected archive path')
            _require(name not in entries, 'duplicate archive path')
            _require(member.isdir() or member.isreg(), 'nonregular archive entry')
            _require(member.isdir() or len(parts) > 1, 'source root must be a directory')
            _require(0 <= member.size <= _FILE, 'archive member exceeds file limit')
            _require(not member.isdir() or member.size == 0, 'directory has payload')
            total += member.size
            _require(total <= _UNCOMPRESSED, 'archive payload exceeds limit')
            payload = None
            if member.isreg():
                with archive.extractfile(member) as stream:
                    payload = stream.read(_FILE + 1)
                _require(len(payload) == member.size, 'truncated archive member')
            entries[name] = payload
    _require(any(v is not None for v in entries.values()), 'archive contains no files')
    for name in entries:
        parts = name.split('/')
        for i in range(1, len(parts)):
            parent = '/'.join(parts[:i])
            _require(parent not in entries or entries[parent] is None,
                     'conflicting file/directory paths')
    return entries


def _directory(root_fd, parts):
    fd = os.dup(root_fd)
    try:
        for part in parts:
            try:
                os.mkdir(part, mode=0o700, dir_fd=fd)
            except FileExistsError:
                pass
            child = os.open(part, os.O_RDONLY | os.O_DIRECTORY | os.O_NOFOLLOW, dir_fd=fd)
            os.close(fd)
            fd = child
        return fd
    except BaseException:
        os.close(fd)
        raise


def _remove_owned(parent_fd, name):
    """Descriptor-relative cleanup also works on Python 3.9 (no rmtree dir_fd)."""
    fd = os.open(name, os.O_RDONLY | os.O_DIRECTORY | os.O_NOFOLLOW, dir_fd=parent_fd)
    try:
        for child in os.listdir(fd):
            info = os.stat(child, dir_fd=fd, follow_symlinks=False)
            if stat.S_ISDIR(info.st_mode):
                _remove_owned(fd, child)
            else:
                os.unlink(child, dir_fd=fd)
    finally:
        os.close(fd)
    os.rmdir(name, dir_fd=parent_fd)


def acquire(output_dir, archive_path=None):
    """Return JSON-ready evidence after fixed-digest and existing extractor proof."""
    output = Path(os.path.abspath(os.fspath(output_dir)))
    _require(output.name != '', 'output must be a new directory')
    parent_fd = _parent_fd(output)  # Reject existing output before input/network I/O.
    owned = None
    try:
        if archive_path is None:
            raw = _download()
        else:
            with open(archive_path, 'rb') as stream:
                raw = stream.read(_COMPRESSED + 1)
        _require(len(raw) <= _COMPRESSED, 'compressed archive exceeds limit')
        _require(hashlib.sha256(raw).hexdigest() == _SHA256, 'archive SHA256 mismatch')
        entries = _members(raw)  # All paths, types, sizes and conflicts before mkdir.
        os.mkdir(output.name, mode=0o700, dir_fd=parent_fd)
        owned = os.stat(output.name, dir_fd=parent_fd, follow_symlinks=False)
        root_fd = os.open(output.name, os.O_RDONLY | os.O_DIRECTORY | os.O_NOFOLLOW,
                          dir_fd=parent_fd)
        try:
            fd = os.open('reference.tar.gz', os.O_WRONLY | os.O_CREAT | os.O_EXCL |
                         os.O_NOFOLLOW, 0o600, dir_fd=root_fd)
            with os.fdopen(fd, 'wb') as stream:
                stream.write(raw)
            for name, payload in entries.items():
                parts = name.split('/')
                directory = _directory(root_fd, parts if payload is None else parts[:-1])
                try:
                    if payload is not None:
                        fd = os.open(parts[-1], os.O_WRONLY | os.O_CREAT | os.O_EXCL |
                                     os.O_NOFOLLOW, 0o600, dir_fd=directory)
                        with os.fdopen(fd, 'wb') as stream:
                            stream.write(payload)
                finally:
                    os.close(directory)
        finally:
            os.close(root_fd)
        source = output / ('reference-' + _COMMIT)
        archive = output / 'reference.tar.gz'
        count = verify_archive(source, archive, _SHA256, _COMMIT)
        return {'readiness': 'input-verified', 'source_root': str(source),
                'archive_path': str(archive), 'archive_sha256': _SHA256,
                'reference_commit': _COMMIT, 'file_count': count, 'verified_files': count,
                'compatibility_credit': False}
    except BaseException:
        if owned is not None:
            current = os.stat(output.name, dir_fd=parent_fd, follow_symlinks=False)
            if stat.S_ISDIR(current.st_mode) and (current.st_dev, current.st_ino) == (owned.st_dev, owned.st_ino):
                _remove_owned(parent_fd, output.name)
        raise
    finally:
        os.close(parent_fd)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--output-dir', required=True, type=Path)
    parser.add_argument('--archive', type=Path)
    args = parser.parse_args()
    try:
        result = acquire(args.output_dir, args.archive)
    except (OSError, ValueError, EOFError, tarfile.TarError) as error:
        print('Reference input acquisition rejected: ' + str(error), file=sys.stderr)
        return 2
    print(json.dumps(result, sort_keys=True))
    return 0


if __name__ == '__main__':
    sys.exit(main())
