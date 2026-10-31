#!/usr/bin/env python3
"""Scoped actual native builder-context-pool transitions, not a general performance gate."""
import argparse
import json
import sys
import uuid
from pathlib import Path
sys.path.insert(0, '/home/jeremy/justrust/bench')
import resident_engine_compare as shared
import resident_state_matrix as matrix

p = argparse.ArgumentParser()
p.add_argument('--engine', type=Path, required=True)
p.add_argument('--output', type=Path, required=True)
p.add_argument('--private', type=Path, required=True)
p.add_argument('--cache-miss-stats', choices=('0', '1'), required=True)
p.add_argument('--builder-pool', choices=('0', '1'), required=True)
a = p.parse_args()
source = Path('/home/jeremy/.jcode/scratch/resident-matrix-native-20261030/source63')
tc = Path('/home/jeremy/.blaze/toolchains/nightly-2026-10-27-x86_64-unknown-linux-gnu')
shared.pinned_source(source)
a.output.mkdir(mode=0o700, exist_ok=False)
a.private.mkdir(mode=0o700, exist_ok=False)
env = shared.environment(a.private, 'engine', tc)
env.update(RA_JIT_DEMAND_STD='1', RA_JIT_CACHE_MISS_STATS=a.cache_miss_stats, RA_JIT_STATS='1', RA_JIT_BUILDER_POOL=a.builder_pool)
originals = {name: (source / name).read_bytes() for name in matrix.FILES}
state = dict(scope=__doc__, status='incomplete', rows=[], startup_controls={k: env[k] for k in ('RA_JIT_DEMAND_STD', 'RA_JIT_CACHE_MISS_STATS', 'RA_JIT_STATS', 'RA_JIT_BUILDER_POOL')}, source_before=shared.source_hashes(source), tool_before=shared.runtime.base.digest(a.engine), toolchain_before=shared.toolchain_identity(tc), private=str(a.private))
def save():
    matrix.durable(a.output / 'state.json', (json.dumps(state, indent=2) + '\n').encode())
try:
    for i, (name, data) in enumerate(originals.items()):
        matrix.durable(a.output / f'original-{i}', data, exclusive=True)
    save()
    expected = None
    for index, phase in enumerate(('cold-prepare', 'repeated-noop-1', 'manifest-feature-add', 'type-error', 'type-recovery', 'assert-failure', 'assert-recovery')):
        nonce = uuid.uuid4().hex
        wanted = matrix.inputs(originals, phase, 0, nonce)
        for name, data in wanted.items():
            if (source / name).read_bytes() != data:
                matrix.durable(source / name, data)
        before = shared.source_hashes(source)
        prefix = a.output / f'{index:02}-{phase}'
        row = dict(phase=phase, nonce=nonce, inputs=before, accepted=False)
        state['rows'].append(row)
        save()
        row.update(shared.runtime.run_process(shared.command(a.engine, 'engine', arguments=matrix.arguments(phase)), source, dict(env, JR_MATRIX_NONCE=nonce), prefix, 180))
        raw, text, error = matrix.captured(prefix)
        row.update(raw=raw, raw_error=error, parsed=shared.runtime.test_results(text), input_unchanged=before == shared.source_hashes(source))
        if phase == 'cold-prepare':
            expected = matrix.contract(row['parsed'], phase)
            if expected is None:
                raise ValueError('cold exact summary test contract missing')
        row['failures'] = matrix.oracle(row, text, expected, phase, nonce, 0)
        row['accepted'] = not row['failures']
        logs = list(Path(env['BLAZE_HOME']).glob('workers/*/log'))
        if not logs:
            raise ValueError('native worker logs absent')
        for j, log in enumerate(logs):
            (a.output / f'{index:02}-{phase}-worker-{j}.log').write_bytes(log.read_bytes())
        save()
        if not row['accepted']:
            raise ValueError(f'{phase}: strict original matrix oracle failed')
    state['status'] = 'passed'
except BaseException as exc:
    state.update(status='failed', error=repr(exc))
    raise
finally:
    for i, (name, data) in enumerate(originals.items()):
        backup = (a.output / f'original-{i}').read_bytes()
        if backup != data:
            raise ValueError('durable backup mismatch')
        matrix.durable(source / name, backup)
    state.update(source_after=shared.source_hashes(source), tool_after=shared.runtime.base.digest(a.engine), toolchain_after=shared.toolchain_identity(tc))
    state['unchanged'] = state['source_before'] == state['source_after'] and state['tool_before'] == state['tool_after'] and state['toolchain_before'] == state['toolchain_after']
    save()
assert state['unchanged']
print(json.dumps(dict(status=state['status'], rows=len(state['rows']), scope=__doc__)))
