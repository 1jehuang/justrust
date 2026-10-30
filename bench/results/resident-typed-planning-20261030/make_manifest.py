#!/usr/bin/env python3
"""Seal preserved raw artifacts once. Not used by the validator to repair evidence.

Only bundle-local files are read. Validators and docs are intentionally not self-hashed.
Run manually after archival, never as a substitute for the semantic audit.
"""
import hashlib
import json
from pathlib import Path

ROOT = Path(__file__).resolve().parent
EXCLUDED = {'manifest.json', 'validation.json', 'README.md', 'validate.py', 'test_validate.py', 'make_manifest.py'}


def main():
    files = {}
    for path in sorted(ROOT.rglob('*')):
        if '__pycache__' in path.parts:
            continue
        if path.is_symlink():
            raise ValueError('no symlinks permitted: ' + str(path))
        if path.is_file() and path.relative_to(ROOT).as_posix() not in EXCLUDED:
            raw = path.read_bytes()
            if raw.startswith(b'\x7fELF') or path.suffix in ('.rlib', '.rmeta', '.so', '.sqlite', '.db'):
                raise ValueError('no binaries/caches permitted: ' + str(path))
            files[path.relative_to(ROOT).as_posix()] = hashlib.sha256(raw).hexdigest()
    data = dict(schema='rejected-resident-typed-planning/v1', decision='REJECTED_NOT_PROMOTED',
                baseline_commit='23a1490cb84fa0aaca872dcf6a430f35e19a3863',
                files_sha256=files,
                policy='Exact raw preservation, no native binaries/caches. Absolute paths inside raw receipts are provenance only. No speedup or default promotion.')
    (ROOT / 'manifest.json').write_text(json.dumps(data, indent=2, sort_keys=True) + '\n')
    print('sealed', len(files), 'raw artifacts')


if __name__ == '__main__':
    main()
