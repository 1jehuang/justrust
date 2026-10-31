#!/usr/bin/env python3
"""Inventory only this portable evidence directory, never a native build."""
import hashlib
import json
from pathlib import Path

root = Path(__file__).resolve().parent
inventory = {}
for path in sorted(root.rglob('*')):
    if path.is_file() and path.name != 'SHA256.json' and '__pycache__' not in path.parts:
        assert not path.is_symlink()
        inventory[str(path.relative_to(root))] = hashlib.sha256(path.read_bytes()).hexdigest()
(root / 'SHA256.json').write_text(json.dumps(inventory, sort_keys=True, indent=2) + '\n')
print(json.dumps({'files': len(inventory), 'manifest_sha256': hashlib.sha256((root/'SHA256.json').read_bytes()).hexdigest()}, sort_keys=True))
