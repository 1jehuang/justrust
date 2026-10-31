#!/usr/bin/env python3
"""Seal public bundle bytes, not private inputs. No native execution."""
import hashlib
import json
from pathlib import Path

HERE = Path(__file__).resolve().parent


def seal(root=HERE):
    files = {str(p.relative_to(root)): hashlib.sha256(p.read_bytes()).hexdigest() for p in sorted(root.rglob('*')) if p.is_file() and '__pycache__' not in p.parts and p.name != 'SHA256.json'}
    (root/'SHA256.json').write_text(json.dumps(files, indent=2, sort_keys=True)+'\n')
    return len(files)


if __name__ == '__main__':
    print(seal())
