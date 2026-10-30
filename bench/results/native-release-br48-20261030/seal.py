#!/usr/bin/env python3
"""Create the evidence inventory once, without reading private files or running tools."""
import hashlib
import json
from pathlib import Path

root = Path(__file__).resolve().parent
manifest = {str(p.relative_to(root)): hashlib.sha256(p.read_bytes()).hexdigest()
            for p in sorted(root.rglob('*')) if p.is_file()
            and p.name != 'sha256.json'
            and '__pycache__' not in p.parts}
with (root / 'sha256.json').open('x') as stream:
    stream.write(json.dumps(manifest, indent=2) + '\n')
print(f'Sealed {len(manifest)} retained artifacts')
