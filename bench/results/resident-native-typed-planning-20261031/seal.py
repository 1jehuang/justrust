#!/usr/bin/env python3
"""Explicit maintainer sealing utility. Does not make semantic assertions."""
import hashlib
import json
from pathlib import Path

root = Path(__file__).resolve().parent

def digest(path):
    return hashlib.sha256(path.read_bytes()).hexdigest()

def emit(name, value):
    (root / name).write_text(json.dumps(value, indent=2, sort_keys=True) + '\n')

if __name__ == '__main__':
    history = {p.relative_to(root).as_posix(): digest(p) for p in sorted(root.rglob('*')) if p.is_file() and p.relative_to(root).parts[0] in ('source', 'harness', 'oracle', 'drivers', 'gates', 'inventories', 'profile', 'strict', 'abba', 'cleanup')}
    emit('HISTORICAL.json', history)
    print('Historical commitment:', digest(root / 'HISTORICAL.json'))
    emit('SHA256.json', {p.relative_to(root).as_posix(): {'sha256': digest(p), 'bytes': p.stat().st_size} for p in sorted(root.rglob('*')) if p.is_file() and p.name != 'SHA256.json'})
