#!/usr/bin/env python3
"""Rotating mirrored three-way actual TicketedProxy comparison, production MAIN only."""
import argparse
import hashlib
import json
from pathlib import Path
import statistics
import subprocess
import sys

ROOT = Path(__file__).resolve().parent
STAGES = ['original', 'fast', 'hint']
VARIANTS = [stage + '-' + mode for stage in STAGES for mode in ['disabled', 'enabled']]
SCENARIOS = ['one', 'multi', 'immediate', 'pending-one', 'pending-multi']

def main():
    p = argparse.ArgumentParser(description=__doc__)
    p.add_argument('--rounds', type=int, default=15)
    p.add_argument('--operations', type=int, default=10000)
    p.add_argument('--pending-rounds', type=int, default=200)
    p.add_argument('--event-capacity', type=int, default=4096)
    p.add_argument('--out', type=Path, required=True)
    args = p.parse_args()
    args.out = args.out.resolve()
    assert args.rounds > 0 and args.operations > 0 and args.pending_rounds > 0
    assert not args.out.exists(), 'use a fresh evidence filename, never overwrite samples'
    raw_dir = args.out.parent / (args.out.stem + '-individual')
    assert not raw_dir.exists(), 'use a fresh raw batch directory'
    raw_dir.mkdir(parents=True)
    provenance = {}
    samples = {v: [] for v in VARIANTS}
    failures, order, paired_blocks = [], [], []
    for round_index in range(args.rounds):
        modes = ['disabled', 'enabled'] if round_index % 2 == 0 else ['enabled', 'disabled']
        rotated = STAGES[round_index % 3:] + STAGES[:round_index % 3]
        if round_index % 2: rotated = list(reversed(rotated))
        sequence = rotated + list(reversed(rotated))
        for mode in modes:
            block = {'round': round_index, 'capture': mode, 'sequence': sequence, 'samples': {}}
            for position, stage in enumerate(sequence):
                variant = f'{stage}-{mode}'
                out = raw_dir / f'{round_index:02d}-{mode}-{position}-{stage}.json'
                command = [sys.executable, str(ROOT / 'measure.py'), '--binary', str(ROOT / 'archive' / stage / 'executable'),
                    '--variant', variant, '--samples', '1', '--operations', str(args.operations),
                    '--pending-rounds', str(args.pending_rounds), '--event-capacity', str(args.event_capacity), '--out', str(out)]
                result = subprocess.run(command, capture_output=True, text=True, timeout=160)
                order.append({'round': round_index, 'capture': mode, 'position': position, 'variant': variant,
                    'exit_code': result.returncode, 'evidence_file': str(out.relative_to(ROOT))})
                if not out.exists():
                    failures.append({'round': round_index, 'variant': variant, 'stdout': result.stdout, 'stderr': result.stderr})
                    continue
                data = json.loads(out.read_text())
                for sample in data['samples']:
                    sample.update(round=round_index, block_position=position)
                    samples[variant].append(sample)
                    if 'elapsed_ns' in sample:
                        block['samples'].setdefault(sample['scenario_command'], {}).setdefault(stage, []).append(sample['elapsed_ns'])
                failures.extend(data['failures'])
                metadata = {k: v for k, v in data.items() if k not in ['samples', 'failures', 'summary', 'created_unix_ns']}
                if variant in provenance:
                    assert provenance[variant] == metadata, 'provenance changed during sampling'
                provenance[variant] = metadata
                if result.returncode:
                    print(result.stdout, result.stderr, file=sys.stderr)
                print(f'round {round_index + 1}/{args.rounds} {mode} {position} {stage}: {len(data["samples"])} samples, {len(data["failures"])} failures', flush=True)
            block['paired_median_ratios'] = {
                scenario: {'hint_over_original': statistics.median(times['hint']) / statistics.median(times['original']),
                    'hint_over_fast': statistics.median(times['hint']) / statistics.median(times['fast']),
                    'fast_over_original': statistics.median(times['fast']) / statistics.median(times['original'])}
                for scenario, times in block['samples'].items() if all(len(times.get(stage, [])) == 2 for stage in STAGES)}
            paired_blocks.append(block)
    summary = {}
    for variant, records in samples.items():
        summary[variant] = {}
        for scenario in SCENARIOS:
            matched = [r for r in records if r['scenario_command'] == scenario]
            times = [r['elapsed_ns'] for r in matched]
            if times:
                summary[variant][scenario] = {'samples': len(times), 'min_ns': min(times), 'median_ns': statistics.median(times),
                    'max_ns': max(times), 'median_pairs_per_second': statistics.median(r['throughput_pairs_per_second'] for r in matched)}
    if len(provenance) == 6:
        for key in ['compiler_version', 'artifact_compiler_stamp', 'local_pinned_compiler_binary_sha256', 'lockfile_sha256', 'harness_sha256', 'runner_sha256', 'machine', 'operations_per_thread', 'pending_rounds']:
            assert len({json.dumps(meta[key], sort_keys=True) for meta in provenance.values()}) == 1, f'unmatched {key}'
        for stage in STAGES:
            assert provenance[stage + '-disabled']['artifact_sha256'] == provenance[stage + '-enabled']['artifact_sha256']
        assert len({tuple(record['cpu_affinity']) for records in samples.values() for record in records}) == 1
    data = {'scope': 'focused production TicketedProxy original/fast/hint only, no legacy speed/default promotion claim',
        'quiet_status': 'unknown', 'cpu_policy': 'unchanged inherited affinity, not pinned; heterogeneous CPU and scheduling may vary',
        'rounds': args.rounds, 'operations_per_thread': args.operations, 'pending_rounds': args.pending_rounds,
        'compare_runner_sha256': hashlib.sha256(Path(__file__).read_bytes()).hexdigest(),
        'order': order, 'paired_blocks': paired_blocks, 'provenance': provenance, 'samples': samples,
        'summary': summary, 'failures': failures}
    args.out.write_text(json.dumps(data, indent=2) + '\n')
    print(json.dumps({'summary': summary, 'failures': failures, 'out': str(args.out)}, indent=2))
    if failures or any(len(records) != 12 * args.rounds for records in samples.values()):
        raise SystemExit(1)

if __name__ == '__main__':
    main()
