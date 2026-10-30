#!/usr/bin/env python3
"""Private, timing-instrumented owned runtime diagnostic, not a speed benchmark."""
import argparse
import hashlib
import json
from pathlib import Path
import sys

sys.path.insert(0, '/home/jeremy/justrust/bench')
import blaze_runtime_compare as h

ROOT = Path('/home/jeremy/.jcode/scratch/bp36')
SOURCE = Path('/home/jeremy/.jcode/scratch/ba30/source')
ARGS = argparse.Namespace(output=ROOT, source=SOURCE,
    rustc=Path('/home/jeremy/.jcode/scratch/dead-item-cache-20261030/sysroot/bin/rustc'),
    cargo=Path('/home/jeremy/.jcode/scratch/lint-profile-20261030/home/toolchains/nightly-2026-10-08/bin/cargo'),
    justrust=Path('/home/jeremy/.jcode/scratch/query-retention-bench-20261030/justrust-client'),
    blaze=Path('/home/jeremy/.jcode/scratch/blaze-comparison-20261030/cargo/bin/blaze'),
    compiler_nightly=True)


def main():
    ROOT.mkdir(mode=0o700, exist_ok=False)
    path = SOURCE / 'src/summary.rs'
    original = path.read_bytes()
    (ROOT / 'summary.rs.backup').write_bytes(original)
    initial = h.source_hashes(SOURCE, ROOT)
    identities = h.base.identities(ARGS)
    state = dict(scope='instrumented-runtime-phase-diagnostic-not-benchmark',
                 status='running', source_before=initial, identities_before=identities, rows=[])
    def save():
        (ROOT / 'state.json').write_text(json.dumps(state, indent=2) + '\n')
    save()
    expected = None
    try:
        for mode in ('off', 'on'):
            nested = argparse.Namespace(**vars(ARGS))
            nested.output = ROOT / mode
            env = h.environment(nested, 'all4on')
            env['JUSTRUST_PASSES'] = 'on'
            env['RUSTFLAGS'] += ' -Zcache-proc-macros=' + ('yes' if mode == 'on' else 'no')
            for phase in ('prepare', 'edit', 'restore'):
                contents = h.edit_bytes(original, replicate=36, anchor=b'fn shares_overlapping_time() {', kind='edit') if phase == 'edit' else original
                path.write_bytes(contents)
                prefix = ROOT / f'{mode}-{phase}'
                before = h.recorded_runs(env)
                row = h.run_process([str(ARGS.justrust), 'test', '--lib', '--', 'summary::tests::', '--test-threads=1', '--nocapture'], SOURCE, env, prefix, 360)
                output, receipt = h.recorded_output(env, before, prefix)
                row.update(mode=mode, phase=phase, raw_output=receipt,
                           source_sha256=hashlib.sha256(contents).hexdigest(), parsed=h.test_results(output))
                state['rows'].append(row)
                save()
                marker = 'JR_RUNTIME_EXECUTED_37' if phase == 'edit' else None
                if not h.accepted(row, output, expected, marker):
                    raise RuntimeError('instrumented runtime correctness gate failed')
                expected = row['parsed']
        state['status'] = 'completed'
    finally:
        path.write_bytes(original)
        state['source_after'] = h.source_hashes(SOURCE, ROOT)
        state['identities_after'] = h.base.identities(ARGS)
        state['source_restored'] = state['source_after'] == initial
        state['identities_unchanged'] = state['identities_after'] == identities
        save()
    assert state['source_restored'] and state['identities_unchanged']
    print(json.dumps({'status': state['status'], 'rows': len(state['rows']), 'source_restored': True}))


if __name__ == '__main__':
    main()
