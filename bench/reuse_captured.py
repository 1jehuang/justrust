#!/usr/bin/env python3
"""Report observed reuse events and historical cost estimates, not a safety ceiling.

Process CPU, query execution and historical cost are different, potentially
overlapping views. Do not add them or infer a maximum safe reuse percentage.
"""
import argparse
from collections import Counter
from datetime import datetime
import json
import math
import os
from pathlib import Path


HITS = {'hit', 'peer', 'local', 'remote', 'prebuilt'}


def records(path):
    if path.is_file():
        with path.open() as stream:
            for line in stream:
                try:
                    row = json.loads(line)
                except ValueError:
                    continue
                if isinstance(row, dict):
                    yield row


def seconds(x):
    return type(x) in (float, int) and math.isfinite(x) and x >= 0


def report(path):
    folder = path.parent
    counts, process, ledger, estimates = (Counter() for _ in range(4))
    summary_path = folder / 'summary.json'
    try:
        summary = json.loads(summary_path.read_text())
    except (OSError, ValueError):
        summary = {}
    for u in records(path):
        counts['unit_rows'] += 1
        if u.get('build_script') and not u.get('crate_types'):
            counts['build_script_runs_excluded'] += 1
            continue
        dc = u.get('depcache')
        key = 'dependency_cache:' + (dc or 'not_reported')
        counts[key] += 1
        if dc in ('miss', 'local-miss'):
            counts['miss_reason:' + (u.get('depcache_why') or 'not_reported').split(':', 1)[0]] += 1
        if all(seconds(u.get(k)) for k in ('user', 'sys')):
            process[key] += u['user'] + u['sys']
            counts['units_with_process_cpu'] += 1
        if dc in HITS and seconds(u.get('depcache_saved_secs')):
            estimates['dependency_hit_original_wall_seconds'] += u['depcache_saved_secs']
            counts['hits_with_original_wall'] += 1
        for row in u.get('reuse') or []:
            q, outcome, n, t = (row[k] for k in ('query', 'outcome', 'count', 'secs'))
            category = q if q.startswith('<') else 'query'
            key = f'{category}:{outcome}'
            counts[key] += n
            if not seconds(t):
                continue
            if (q, outcome) == ('<cgu_avoided>', 'measured') or outcome == 'loaded_avoided':
                estimates[key + ':historical_cost_seconds'] += t
            elif (category == 'query' and not outcome.startswith('loaded_')) or q in ('<cgu_cpu>', '<link>'):
                ledger[key] += t
    fresh = 0
    for r in records(folder / 'fresh.jsonl'):
        fresh += 1
        if seconds(r.get('cpu_secs')):
            # Despite its name, cpu_secs can contain the original wall time.
            basis = r.get('basis') or 'unspecified'
            estimates[f'cargo_fresh_historical_{basis}_seconds'] += r['cpu_secs']
            counts['fresh_rows_with_cost'] += 1
    counts['fresh_sidecar_rows'] = fresh
    counts['summary_fresh_without_sidecar_row'] = max(0, ((summary.get('units') or {}).get('fresh') or 0) - fresh)
    return dict(run=folder.name, args=summary.get('args'), toolchain=summary.get('toolchain'),
                where=(summary.get('remote') or {}).get('machine'), counts=counts,
                recorded_unit_cpu_seconds=process, recorded_ledger_seconds=ledger,
                estimated_gross_costs=estimates, maximum_safe_reuse=None)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('since', nargs='?', type=datetime.fromisoformat)
    parser.add_argument('--runs', type=Path, default=Path.home() / '.justrust/runs')
    parser.add_argument('--run', type=Path, help='read just this run directory')
    parser.add_argument('--output', type=Path, default=Path(os.environ.get('JCODE_SCRATCH_DIR', '/tmp')) / 'reuse-captured.jsonl')
    args = parser.parse_args()
    paths = [args.run / 'units.jsonl'] if args.run else args.runs.glob('*/units.jsonl')
    totals = {k: Counter() for k in ('counts', 'recorded_unit_cpu_seconds', 'recorded_ledger_seconds', 'estimated_gross_costs')}
    n = 0
    with args.output.open('w') as stream:
        for p in paths:
            if not p.is_file() or (args.since and p.stat().st_mtime < args.since.timestamp()):
                continue
            row = report(p)
            print(json.dumps(row), file=stream)
            for key in totals:
                totals[key].update(row[key])
            n += 1
    print(f'{n} builds, per-build rows: {args.output}')
    print(json.dumps(totals, sort_keys=True))
    print('Maximum safe reuse and captured percentage are not identifiable from these records.')


if __name__ == '__main__':
    main()
