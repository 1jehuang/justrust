#!/usr/bin/env python3
"""Seal only local evidence files, no workload or live-source access."""
import hashlib
import json
from pathlib import Path


def seal(root):
    files = {}
    for path in sorted(root.rglob('*')):
        if path.is_symlink():
            raise ValueError(f'symlink in evidence: {path}')
        if path.is_file() and path.name != 'manifest.json' and '__pycache__' not in path.parts:
            files[str(path.relative_to(root))] = hashlib.sha256(path.read_bytes()).hexdigest()
    (root / 'manifest.json').write_text(json.dumps({'version': 1, 'files': files}, indent=2) + '\n')
    return len(files)


if __name__ == '__main__':
    print(json.dumps({'sealed': seal(Path(__file__).resolve().parent)}))
