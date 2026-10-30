#!/usr/bin/env python3
"""Read-only rsm02 export. Original cross-engine exclusions remain unchanged."""
import importlib.util
from pathlib import Path
import shutil

HERE = Path(__file__).resolve().parent
BASE = HERE.parent / 'resident-state-matrix-20261030'
spec = importlib.util.spec_from_file_location('matrix_exporter', BASE / 'collect.py')
c = importlib.util.module_from_spec(spec)
spec.loader.exec_module(c)


def main():
    transform = c.export_run(Path('/home/jeremy/rsm02'), HERE / 'runs/rsm02')
    for directory in ('input', 'provenance'):
        shutil.copytree(BASE / directory, HERE / directory)
    c.emit(HERE / 'provenance/matrix-evidence-validator.py', (BASE / 'validate.py').read_bytes())
    c.emit(HERE / 'evidence.json', c.encoded(dict(version=1, original_matrix='rsm02',
        source_commit='f97fcc2', native_fix_commit='91220ff43',
        private_commitments=transform['private_original_sha256'],
        scope='Original 140-call cross-engine matrix remains failed/excluded for ten error observations. A separately replayed intrinsic candidate error oracle is not cross-engine admission, a speedup or a comprehensive/default gate.')))


if __name__ == '__main__':
    main()
