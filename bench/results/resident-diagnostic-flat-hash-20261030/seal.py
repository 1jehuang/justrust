#!/usr/bin/env python3
"""Seal existing evidence offline, no source runner or native command execution."""
import json
from pathlib import Path
import validate

root = Path(__file__).resolve().parent
report = validate.validate(root, sealed=False)
(root / 'audit.json').write_text(json.dumps(report, indent=2) + '\n')
files = {str(p.relative_to(root)): validate.digest(p) for p in sorted(root.rglob('*'))
         if p.is_file() and '__pycache__' not in p.parts and p.name != 'manifest.json'}
(root / 'manifest.json').write_text(json.dumps(dict(files=files), indent=2) + '\n')
validate.validate(root)
print(f'Sealed and replayed {len(files)} local files')
