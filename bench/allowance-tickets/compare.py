#!/usr/bin/env python3
"""Interleave frozen actual-source variants, retaining every individual sample."""
import argparse
import hashlib
import json
from pathlib import Path
import statistics
import subprocess
import sys

ROOT = Path(__file__).resolve().parent
VARIANTS = ['baseline', 'candidate-disabled', 'candidate-enabled']

def main():
    p = argparse.ArgumentParser(description=__doc__)
    p.add_argument('--rounds', type=int, default=15)
    p.add_argument('--operations', type=int, default=10000)
    p.add_argument('--event-capacity', type=int, default=4096)
    p.add_argument('--out', type=Path, default=ROOT / 'results/comparison-034.json')
    args = p.parse_args()
    assert args.rounds > 0 and args.operations > 0
    provenance = {}
    all_samples = {v: [] for v in VARIANTS}
    failures = []
    order = []
    raw_dir = args.out.parent / (args.out.stem + '-individual')
    raw_dir.mkdir(parents=True, exist_ok=True)
    for round_index in range(args.rounds):
        rotated = VARIANTS[round_index % 3:] + VARIANTS[:round_index % 3]
        for variant in rotated:
            binary = ROOT / 'archive' / ('baseline-executable-0.1.34' if variant == 'baseline' else 'candidate-executable-0.1.34')
            out = raw_dir / f'{round_index:02d}-{variant}.json'
            command = [sys.executable, str(ROOT / 'measure.py'), '--binary', str(binary), '--variant', variant,
                       '--samples', '1', '--operations', str(args.operations), '--event-capacity', str(args.event_capacity), '--out', str(out)]
            result = subprocess.run(command, capture_output=True, text=True, timeout=140)
            order.append({'round': round_index, 'variant': variant, 'exit_code': result.returncode,
                          'evidence_file': str(out.relative_to(ROOT))})
            if not out.exists():
                failures.append({'round': round_index, 'variant': variant, 'stdout': result.stdout, 'stderr': result.stderr})
                continue
            data = json.loads(out.read_text())
            for sample in data['samples']:
                sample['round'] = round_index
                all_samples[variant].append(sample)
            failures.extend(data['failures'])
            metadata = {k: v for k, v in data.items() if k not in ['samples', 'failures', 'summary', 'created_unix_ns']}
            if variant in provenance:
                assert provenance[variant] == metadata, 'provenance changed during durable sampling'
            provenance[variant] = metadata
            if result.returncode:
                print(result.stdout, result.stderr, file=sys.stderr)
            print(f'round {round_index + 1}/{args.rounds} {variant}: {len(data["samples"])} samples, {len(data["failures"])} failures', flush=True)
    summaries = {}
    for variant, samples in all_samples.items():
        summaries[variant] = {}
        for scenario in ['one', 'multi', 'immediate']:
            matched = [r for r in samples if r['scenario_command'] == scenario]
            times = [r['elapsed_ns'] for r in matched]
            if times:
                summaries[variant][scenario] = {'samples': len(times), 'min_ns': min(times),
                    'median_ns': statistics.median(times), 'max_ns': max(times),
                    'median_throughput_pairs_per_second': statistics.median(r['throughput_pairs_per_second'] for r in matched)}
    if len(provenance) == 3:
        for key in ['compiler_version', 'artifact_compiler_stamp', 'local_pinned_compiler_binary_sha256', 'lockfile_sha256', 'harness_sha256', 'runner_sha256', 'machine']:
            assert len({json.dumps(p[key], sort_keys=True) for p in provenance.values()}) == 1, f'unmatched {key}'
        assert provenance['candidate-disabled']['artifact_sha256'] == provenance['candidate-enabled']['artifact_sha256']
    data = {'scope': 'focused actual-module comparison, no broad/default promotion or resource identity claim',
            'rounds': args.rounds, 'operations_per_thread': args.operations, 'order': order,
            'compare_runner_sha256': hashlib.sha256(Path(__file__).read_bytes()).hexdigest(),
            'provenance': provenance, 'samples': all_samples, 'summary': summaries, 'failures': failures}
    args.out.write_text(json.dumps(data, indent=2) + '\n')
    print(json.dumps({'summary': summaries, 'failures': failures, 'out': str(args.out)}, indent=2))
    if failures or any(len(s) != 4 * args.rounds for s in all_samples.values()):
        raise SystemExit(1)

if __name__ == '__main__':
    main()
