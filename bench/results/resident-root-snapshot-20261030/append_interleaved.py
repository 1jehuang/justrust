#!/usr/bin/env python3
"""Append terminal warm diagnostic bytes without recapturing any original artifact."""
import hashlib
import json
from pathlib import Path
import shutil

root = Path(__file__).resolve().parent
scratch = Path('/home/jeremy/.jcode/scratch')
manifest = json.loads((root/'manifest.json').read_text())
for path, digest in manifest['sha256'].items():
    assert hashlib.sha256((root/path).read_bytes()).hexdigest() == digest, path
source = scratch/'resident-interleaved-v19-20261030'
assert json.loads((source/'state.json').read_text())['status'] == 'passed'
target = root/'observations/interleaved-v19'
assert not target.exists(), 'never overwrite an observation'
shutil.copytree(source, target)
shutil.copy2(scratch/'blaze-native-engine-20261030/interleaved-v19.py', target/'runner.py')
for path in [Path(__file__).resolve(), *sorted(target.rglob('*'))]:
    if path.is_file():
        relative = str(path.relative_to(root))
        assert relative not in manifest['sha256']
        manifest['sha256'][relative] = hashlib.sha256(path.read_bytes()).hexdigest()
(root/'manifest.json').write_text(json.dumps(manifest, indent=2)+'\n')
print('Appended separate terminal warm observation. Original artifact hashes unchanged.')
