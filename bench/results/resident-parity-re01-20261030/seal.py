#!/usr/bin/env python3
"""Recreate the bundle allowlist after an intentional reviewed change."""
import hashlib
import json
from pathlib import Path

root = Path(__file__).resolve().parent
manifest = {}
for path in sorted(root.rglob('*')):
    if '__pycache__' in path.parts or path.name == 'SHA256.json':
        continue
    if path.is_symlink():
        raise ValueError('links are not bundle inputs: ' + str(path))
    if path.is_file():
        data = path.read_bytes()
        if data.startswith(b'\x7fELF'):
            raise ValueError('native binary is not a bundle input: ' + str(path))
        manifest[path.relative_to(root).as_posix()] = hashlib.sha256(data).hexdigest()
(root / 'SHA256.json').write_text(json.dumps(manifest, indent=2) + '\n')
print('Sealed', len(manifest), 'allowlisted files')
