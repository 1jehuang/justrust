#!/usr/bin/env python3
"""Record offline replay/falsification checks only, then reseal the evidence inventory."""
import json
from pathlib import Path
import sys
sys.dont_write_bytecode = True
import replay
import test_replay

root = Path(__file__).resolve().parent
replay.seal(root)
result = replay.validate(root)
tests = test_replay.run()
(root / 'checks').mkdir(exist_ok=True)
(root / 'checks/replay.json').write_text(json.dumps(result, indent=2) + '\n')
(root / 'checks/mutations-relocation.json').write_text(json.dumps(tests, indent=2) + '\n')
replay.seal(root)
assert replay.validate(root) == result
print(json.dumps(dict(status='PASS', raw_rows=result['raw_rows'], offline_cases=len(tests['cases']), native_jobs=False)))
