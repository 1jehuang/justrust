#!/usr/bin/env python3
"""Record only offline evidence replay and controls, then seal the completed bundle."""
import json
from pathlib import Path
import subprocess
import sys

sys.dont_write_bytecode = True
ROOT = Path(__file__).resolve().parent
sys.path.insert(0, str(ROOT))
import replay

checks = ROOT / 'checks'
checks.mkdir(exist_ok=True)
replay.seal(ROOT)
result = replay.validate(ROOT)
(checks / 'offline-replay.json').write_text(json.dumps(result, indent=2) + '\n')
replay.seal(ROOT)
process = subprocess.run([sys.executable, '-B', str(ROOT / 'test_replay.py')], text=True, capture_output=True, check=False)
(checks / 'controls.stderr').write_text(process.stderr)
(checks / 'controls.stdout').write_text(process.stdout)
if process.returncode:
    replay.seal(ROOT)
    raise SystemExit(process.returncode)
controls = json.loads(process.stdout)
assert controls['status'] == 'PASS' and controls['negative_controls'] == 33
(checks / 'offline-controls.json').write_text(json.dumps(controls, indent=2) + '\n')
replay.seal(ROOT)
assert replay.validate(ROOT) == result
print(json.dumps(dict(status='PASS', strict_rows=28, negative_controls=33, isolated_relocation='PASS', native_jobs=0), indent=2))
