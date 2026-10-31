#!/usr/bin/env python3
"""Record offline checks and explicit retained lock delta. No native execution."""
import difflib
import json
from pathlib import Path
import subprocess
import sys
sys.dont_write_bytecode = True
import replay

ROOT = Path(__file__).resolve().parent
checks = ROOT / 'checks'
checks.mkdir(exist_ok=True)
original = (ROOT / 'provenance/original-vendor-Cargo.lock').read_bytes()
selected = replay.tar_bytes(ROOT / 'snapshots/rust-analyzer-v49.tar.gz')['Cargo.lock']
(ROOT / 'provenance/private-snapshot-Cargo.lock').write_bytes(selected)
(ROOT / 'provenance/private-lock.patch').write_text(''.join(difflib.unified_diff(original.decode().splitlines(True), selected.decode().splitlines(True), fromfile='original-vendor-Cargo.lock', tofile='private-snapshot-Cargo.lock')))
replay.seal(ROOT)
result = replay.validate(ROOT)
(checks / 'replay.json').write_text(json.dumps(result, indent=2) + '\n')
replay.seal(ROOT)
control = subprocess.run([sys.executable, '-B', str(ROOT / 'test_replay.py')], text=True, capture_output=True)
(checks / 'controls.stdout').write_text(control.stdout)
(checks / 'controls.stderr').write_text(control.stderr)
(checks / 'controls.exit').write_text(str(control.returncode) + '\n')
replay.seal(ROOT)
if control.returncode:
    raise SystemExit(control.returncode)
replay.validate(ROOT)
print('PASS: actual raw28/native/source/lock joins, isolated relocation, all14 coherently re-sealed semantic mutation controls. REJECTED experiment, no native jobs.')
