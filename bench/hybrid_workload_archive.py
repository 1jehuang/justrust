#!/usr/bin/env python3
"""Archive existing hybrid receipts only, never source, targets or environment dumps."""
import argparse
import hashlib
import io
import json
from pathlib import Path
import re
import sys
import tarfile


SECRET = re.compile(rb'(?i)authorization:\s*(?:bearer|basic)\s+\S+|(?:api[_-]?key|access[_-]?token|secret[_-]?key|password)\s*[=:]\s*[\"\x27]?[A-Za-z0-9_+/.=-]{12,}|\b(?:AKIA|ASIA)[A-Z0-9]{16}\b')


def receipts(root, names):
    selected = {}
    for name in names:
        if not re.fullmatch(r'[A-Za-z0-9_-]+', name):
            raise ValueError('run names must be simple directory names')
        run = root / name
        if run.is_symlink() or not run.is_dir():
            raise ValueError('run must be an existing non-symlink directory')
        paths = [p for p in run.iterdir() if p.suffix in ('.json', '.log', '.stdout', '.stderr')]
        paths += [root / (name + suffix) for suffix in ('.full.log', '.exit')]
        for stack in ('shipping', 'hybrid'):
            home = run / stack / 'engine-home'
            paths += [home / 'prewarm.log', home / 'resident-v1/runtime/daemon.log']
            paths += list((home / 'resident-v1/runtime/workers').glob('*/log'))
        for path in paths:
            if not path.exists():
                continue
            if path.is_symlink() or not path.is_file() or not path.resolve().is_relative_to(root.resolve()):
                raise ValueError('receipt must be a regular in-root file: ' + str(path))
            if path.stat().st_size > 64 * 1024 * 1024:
                raise ValueError('receipt exceeds 64MiB bound: ' + str(path))
            data = path.read_bytes()
            if SECRET.search(data):
                raise ValueError('possible secret in receipt, refusing archive: ' + str(path))
            selected[str(path.relative_to(root))] = data
    return selected


def archive(root, names, stream):
    files = receipts(root, names)
    manifest = {'schema': 1, 'scope': 'existing raw receipts only, no source or targets',
                'files': {name: {'bytes': len(data), 'sha256': hashlib.sha256(data).hexdigest()}
                          for name, data in sorted(files.items())}}
    files['RECEIPTS.json'] = (json.dumps(manifest, indent=2) + '\n').encode()
    with tarfile.open(fileobj=stream, mode='w|gz') as bundle:
        for name, data in sorted(files.items()):
            info = tarfile.TarInfo(name)
            info.size = len(data)
            info.mode = 0o600
            info.mtime = 0
            bundle.addfile(info, io.BytesIO(data))


if __name__ == '__main__':
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--root', required=True, type=Path)
    parser.add_argument('--run', required=True, action='append')
    args = parser.parse_args()
    archive(args.root, args.run, sys.stdout.buffer)
