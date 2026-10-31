#!/usr/bin/env python3
"""Seal authored audit inputs and captured exact receipts once."""
import hashlib
import json
from pathlib import Path
ROOT = Path(__file__).resolve().parent
EXCLUDED = {'manifest.json', 'audit.json', 'offline-tests.log'}
if (ROOT / 'manifest.json').exists():
    raise ValueError('refusing to replace sealed inventory')
inventory = {str(p.relative_to(ROOT)): hashlib.sha256(p.read_bytes()).hexdigest()
             for p in sorted(ROOT.rglob('*')) if p.is_file()
             and p.name not in EXCLUDED and '__pycache__' not in p.parts}
(ROOT / 'manifest.json').write_text(json.dumps(inventory, indent=2) + '\n')
print('Sealed', len(inventory), 'files')
