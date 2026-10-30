#!/usr/bin/env python3
"""Actual full-lib support probe, not a statistical or comprehensive performance gate."""
import hashlib
import json
from pathlib import Path
import sys

sys.path.insert(0, '/home/jeremy/justrust/bench')
import resident_engine_compare as shared
import resident_workload_compare as broader

ROOT = Path('/home/jeremy/.jcode/scratch/blaze-native-engine-20261030')
SOURCE = Path('/home/jeremy/.jcode/scratch/resident-matrix-probe-20261030/source63')
OUT = Path('/home/jeremy/rmp01')
TC = Path('/home/jeremy/.blaze/toolchains/nightly-2026-10-27-x86_64-unknown-linux-gnu')
TOOLS = {'reference': ROOT / 'reference-bin/ra-jit', 'engine': ROOT / 'candidate-v28-bin/jr-engine'}
shared.pinned_source(SOURCE)
OUT.mkdir(mode=0o700)
state = dict(scope=__doc__, status='incomplete', source_before=shared.source_hashes(SOURCE), toolchain_before=shared.toolchain_identity(TC), tools_before={k: dict(path=str(v), sha256=shared.runtime.base.digest(v)) for k,v in TOOLS.items()}, rows=[])

def save():
    (OUT / 'state.json').write_text(json.dumps(state, indent=2) + '\n')

try:
    save()
    for stack, tool in TOOLS.items():
        env = shared.environment(OUT, stack, TC)
        prefix = OUT / f'{stack}-full-lib'
        cmd = shared.command(tool, stack, arguments=['--lib', '--', '--test-threads=1', '--nocapture'])
        row = shared.runtime.run_process(cmd, SOURCE, env, prefix, 360)
        row['environment'] = {k:v for k,v in row['environment'].items() if k in broader.ENV_RECEIPT_ALLOWLIST}
        text = prefix.with_suffix('.stdout').read_text(errors='replace') + '\n' + prefix.with_suffix('.stderr').read_text(errors='replace')
        row.update(stack=stack, parsed=shared.runtime.test_results(text), stdout_sha256=shared.runtime.base.digest(prefix.with_suffix('.stdout')), stderr_sha256=shared.runtime.base.digest(prefix.with_suffix('.stderr')))
        state['rows'].append(row)
        save()
finally:
    state['source_after'] = shared.source_hashes(SOURCE)
    state['toolchain_after'] = shared.toolchain_identity(TC)
    state['tools_after'] = {k: dict(path=str(v), sha256=shared.runtime.base.digest(v)) for k,v in TOOLS.items()}
    state['unchanged'] = all(state[k+'_before'] == state[k+'_after'] for k in ('source','toolchain','tools'))
    rows=state['rows']
    success = len(rows)==2 and all(shared.runtime.base.succeeded(r) and r['parsed'] is not None and r['parsed']['failed']==0 and r['parsed']['ignored']==0 for r in rows)
    state['status'] = 'passed' if success and rows[0]['parsed']==rows[1]['parsed'] and state['unchanged'] else 'failed/support-not-established'
    save()
print(json.dumps(dict(status=state['status'], unchanged=state['unchanged'], rows=[dict(stack=r['stack'],status=r['status'],elapsed_seconds=r['elapsed_seconds'],tests=r['parsed']) for r in state['rows']]), indent=2))
raise SystemExit(0 if state['status']=='passed' else 1)
