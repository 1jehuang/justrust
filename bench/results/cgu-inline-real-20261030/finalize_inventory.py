#!/usr/bin/env python3
"""Freeze this offline bundle's complete inventory after receipt generation."""
import hashlib
import json
from pathlib import Path

ROOT = Path(__file__).resolve().parent


def main():
    files = {}
    for path in sorted(ROOT.rglob('*')):
        assert not path.is_symlink(), str(path)
        if path.is_file() and '__pycache__' not in path.parts and path.name != 'sha256.json':
            files[str(path.relative_to(ROOT))] = hashlib.sha256(path.read_bytes()).hexdigest()
    (ROOT / 'sha256.json').write_text(json.dumps(files, indent=2) + '\n')
    print(f'Inventoried {len(files)} files, excluding inventory itself and bytecode')


if __name__ == '__main__':
    main()
