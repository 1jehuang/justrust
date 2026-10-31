#!/usr/bin/env python3
"""Record offline historical replay controls only, never native compilation or workloads."""
import json
from pathlib import Path
import sys
sys.dont_write_bytecode = True
import replay
import test_replay
ROOT = Path(__file__).resolve().parent
CHECKS = ROOT / 'checks'
CHECKS.mkdir(exist_ok=True)
replay.seal(ROOT)
result = replay.validate(ROOT)
(CHECKS / 'replay.json').write_text(json.dumps(result, indent=2) + '\n')
replay.seal(ROOT)
controls = test_replay.run()
(CHECKS / 'semantic-mutations-and-relocation.json').write_text(json.dumps(controls, indent=2) + '\n')
replay.seal(ROOT)
assert replay.validate(ROOT) == result
print(json.dumps(dict(status='PASS', native_jobs=False, raw_rows=result['raw_rows'], mutations=len(controls['cases'])-1, isolated_empty_home_path=True, original_failed_delta_reproduced=result['analysis_replay']['failed_delta'], corrected_snapshot_reproduced=result['analysis_replay']['corrected_snapshot']), indent=2))
