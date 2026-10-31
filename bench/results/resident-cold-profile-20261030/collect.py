#!/usr/bin/env python3
"""Capture frozen receipts only. Never execute a native tool or read live sources."""
import hashlib
import json
from pathlib import Path
import subprocess

ROOT = Path('/home/jeremy/.jcode/scratch/blaze-native-engine-20261030')
OUT = Path(__file__).resolve().parent
COMMIT = '91220ff43'


def put(name, raw):
    path = OUT / name
    path.parent.mkdir(parents=True, exist_ok=True)
    if path.exists():
        raise ValueError('refusing to overwrite ' + name)
    path.write_bytes(raw)


def main():
    if (OUT / 'manifest.json').exists():
        raise ValueError('already sealed')
    for path in sorted((ROOT / 'cold-path-v30-statistics').iterdir()):
        if path.is_file() and not path.is_symlink():
            put('raw/' + path.name, path.read_bytes())
    put('runner/profile-cold-v30.py', (ROOT / 'profile-cold-v30.py').read_bytes())
    for name in ('main.rs.frozen', 'all-source-before.sha256', 'engine-identity.sha256',
                 'engine-version.txt', 'check.exit', 'compile.exit', 'orchestration-exit'):
        put('provenance/' + name, (ROOT / 'candidate-build-v30' / name).read_bytes())
    for name in ('bench/resident_engine_compare.py', 'bench/blaze_runtime_compare.py',
                 'bench/blaze_edit_compare.py', 'toolchain/resident/LICENSE-APACHE',
                 'toolchain/resident/LICENSE-MIT', 'toolchain/resident/JUSTRUST-SOURCE.md'):
        raw = subprocess.check_output(['git', 'show', COMMIT + ':' + name])
        put(('runner/' if name.startswith('bench/') else 'licenses/') + Path(name).name, raw)
    committed = subprocess.check_output(['git', 'show', COMMIT + ':toolchain/resident/crates/ra-jit/src/main.rs'])
    if committed != (OUT / 'provenance/main.rs.frozen').read_bytes():
        raise ValueError('frozen source is not committed candidate')
    state = json.loads((OUT / 'raw/state.json').read_bytes())
    put('provenance/source63-manifest.json', (json.dumps(state['source_before'], indent=2) + '\n').encode())
    put('provenance/capture.json', (json.dumps({'source_commit': COMMIT,
        'frozen_main_sha256': hashlib.sha256(committed).hexdigest(),
        'source_scope': 'frozen main plus build full-source hash inventory, not a compiler source redistribution',
        'capture_scope': 'raw diagnostic receipts, committed helpers/licenses, no native binaries or cache data',
        'native_jobs_executed_by_capture': False}, indent=2) + '\n').encode())
    print('Captured frozen diagnostic without native execution.')


if __name__ == '__main__':
    main()
