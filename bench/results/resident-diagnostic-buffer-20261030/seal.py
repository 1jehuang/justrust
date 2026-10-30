#!/usr/bin/env python3
"""Seal the allowlisted evidence inventory after collection, not during validation."""
import json
from pathlib import Path
from validate import EXCLUDED
from raw_checks import sha
root=Path(__file__).resolve().parent
files={str(p.relative_to(root)):sha(p.read_bytes()) for p in sorted(root.rglob('*')) if p.is_file() and p.name not in EXCLUDED and p.name!='seal.py' and '__pycache__' not in p.parts}
(root/'manifest.json').write_text(json.dumps({'sha256':files},indent=2)+'\n')
