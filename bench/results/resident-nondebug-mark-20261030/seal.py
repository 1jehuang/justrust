#!/usr/bin/env python3
"""Seal retained evidence once. Does not run a validator or native workload."""
import hashlib
import json
from pathlib import Path

ROOT = Path(__file__).resolve().parent
EXCLUDED = {'manifest.json', 'audit.json', 'offline-tests.log'}


def main():
    manifest = ROOT / 'manifest.json'
    if manifest.exists():
        raise SystemExit('refusing to overwrite existing manifest')
    inventory = {}
    for path in sorted(ROOT.rglob('*')):
        if path.is_file() and path.name not in EXCLUDED and '__pycache__' not in path.parts:
            if path.is_symlink():
                raise SystemExit(f'refusing symlink: {path}')
            inventory[str(path.relative_to(ROOT))] = hashlib.sha256(path.read_bytes()).hexdigest()
    manifest.write_text(json.dumps({'schema': 'rejected-resident-mark-evidence/v1', 'sha256': inventory}, indent=2) + '\n')
    print(f'Sealed {len(inventory)} files. No native or audit tests executed.')


if __name__ == '__main__':
    main()
