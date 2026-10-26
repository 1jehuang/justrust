#!/usr/bin/env python3
"""Read-only inventory of justrust run summaries. No subprocesses or source edits."""
import argparse
from collections import Counter, defaultdict
from datetime import datetime, timezone
import hashlib
import json
import math
from pathlib import Path

PHASES = ('startup', 'lock_wait', 'compile', 'build_gaps', 'test_run', 'tail')
WORKSPACES = {'justrust', 'justrust-clean-room', 'jcode', 'jcode-desktop',
              'justrust-cloud', 'solosystems-backend'}


def number(value):
    """Accept finite nonnegative JSON numbers, never bools or numeric strings."""
    if isinstance(value, bool) or not isinstance(value, (int, float)):
        return None
    try:
        return float(value) if math.isfinite(value) and value >= 0 else None
    except (OverflowError, ValueError):
        return None


def obj(value):
    return value if isinstance(value, dict) else {}


def label(cwd):
    """Do not publish arbitrary paths, names, command arguments, or diagnostics."""
    if not isinstance(cwd, str) or not cwd:
        return 'unknown'
    parts = Path(cwd).parts
    if 'scratch' in parts or '.scratch' in parts:
        return 'scratch'
    for name in reversed(parts):
        if name in WORKSPACES:
            return name
    return 'workspace-' + hashlib.sha256(cwd.encode()).hexdigest()[:12]


def classify(summary, meta):
    cwd = summary.get('cwd', meta.get('cwd', ''))
    cwd = cwd if isinstance(cwd, str) else ''
    text = cwd.lower()
    args = summary.get('args', meta.get('args', []))
    args = args if isinstance(args, list) else []
    if any(x in text for x in ('/bench/', '/benchmarks/', '/synth-', '/benchmark')) or 'bench' in args:
        return 'benchmark'
    if any(x in text for x in ('/scratch/', '/.scratch/', '/tmp/', '/var/tmp/')):
        return 'scratch'
    return 'real-workload-candidate' if cwd else 'unknown'


def load(path, coverage, prefix):
    try:
        value = json.loads(path.read_text())
        if not isinstance(value, dict):
            raise ValueError('not an object')
        coverage[prefix + '_loaded'] += 1
        return value
    except FileNotFoundError:
        coverage[prefix + '_missing'] += 1
    except (OSError, ValueError, UnicodeError):
        coverage[prefix + '_malformed_or_unreadable'] += 1
    return {}


def inventory(root):
    coverage = Counter()
    rows = []
    for directory in sorted(root.iterdir()):
        if not directory.is_dir():
            coverage['non_directory_entries'] += 1
            continue
        coverage['run_directories'] += 1
        meta = load(directory / 'meta.json', coverage, 'meta')
        summary = load(directory / 'summary.json', coverage, 'summary')
        wall = number(summary.get('wall'))
        if wall is None:
            coverage['unmeasured_wall'] += 1
            continue
        coverage['measured_runs'] += 1
        start = number(summary.get('start', meta.get('start')))
        if start is None:
            coverage['missing_or_invalid_start'] += 1
        phases = {k: number(obj(summary.get('phases')).get(k)) for k in PHASES}
        measured = {k: v for k, v in phases.items() if v is not None}
        dominant = max(sorted(measured), key=measured.get) if measured and max(measured.values()) > 0 else 'unmeasured'
        remote = obj(summary.get('remote'))
        client = number(remote.get('client_wall'))
        command = summary.get('subcommand', meta.get('subcommand'))
        command = command if isinstance(command, str) and command in {'build', 'check', 'test', 'clippy', 'run', 'bench', 'doc', 'rustc'} else 'other'
        exit_code = summary.get('exit')
        outcome = 'success' if type(exit_code) is int and exit_code == 0 else 'failure' if type(exit_code) is int else 'unknown'
        # IDs normally contain no paths. Hash unexpected names instead of leaking them.
        rid = directory.name
        if not all(c.isalnum() or c in '-_.' for c in rid):
            rid = 'run-' + hashlib.sha256(rid.encode()).hexdigest()[:16]
        rows.append(dict(id=rid, wall=wall, start=start,
                         workspace=label(summary.get('cwd', meta.get('cwd'))),
                         command=command, category=classify(summary, meta), outcome=outcome,
                         place='remote' if remote else meta.get('place') if meta.get('place') in ('local', 'remote') else 'unknown',
                         cause=dominant, phases=measured, client_wall=client))
    return rows, dict(sorted(coverage.items()))


def totals(rows, threshold):
    return dict(runs=len(rows), slow_runs=sum(r['wall'] > threshold for r in rows),
                recorded_wall_secs=round(sum(r['wall'] for r in rows), 3),
                excess_secs=round(sum(max(0, r['wall'] - threshold) for r in rows), 3))


def detail(row, threshold):
    return {k: (round(v, 3) if isinstance(v, float) else v)
            for k, v in row.items() if k not in ('start', 'phases')} | {
                'excess_secs': round(row['wall'] - threshold, 3),
                'measured_phases_secs': {k: round(v, 3) for k, v in row['phases'].items()}}


def aggregate(rows, threshold, limit):
    rows = sorted(rows, key=lambda r: r['id'])
    slow = [r for r in rows if r['wall'] > threshold]
    dimensions = {}
    for dimension in ('workspace', 'command', 'cause', 'category', 'outcome', 'place'):
        groups = defaultdict(list)
        for row in slow:
            groups[row[dimension]].append(row)
        values = [dict(key=k, **totals(v, threshold),
                       example_run_ids=[r['id'] for r in sorted(v, key=lambda r: (-r['wall'], r['id']))[:3]])
                  for k, v in groups.items()]
        dimensions[dimension] = {
            'by_excess': sorted(values, key=lambda v: (-v['excess_secs'], v['key']))[:20],
            'omitted_by_excess': totals([r for v in sorted(values, key=lambda v: (-v['excess_secs'], v['key']))[20:] for r in groups[v['key']]], threshold),
            'by_total_wall_keys': [v['key'] for v in sorted(values, key=lambda v: (-v['recorded_wall_secs'], v['key']))[:20]]}
    clients = [r for r in rows if r['client_wall'] is not None]
    return dict(**totals(rows, threshold), groups=dimensions,
                remote_client_wall_coverage=dict(runs=len(clients),
                    reported_secs=round(sum(r['client_wall'] for r in clients), 3),
                    client_slow_but_recorded_wall_not_slow=sum(r['client_wall'] > threshold and r['wall'] <= threshold for r in clients)),
                slowest=[detail(r, threshold) for r in sorted(slow, key=lambda r: (-r['wall'], r['id']))[:limit]])


def report(root, threshold, now, days=7, limit=12):
    rows, coverage = inventory(root)
    cutoff = now - days * 86400
    recent = [r for r in rows if r['start'] is not None and cutoff <= r['start'] <= now]
    real = [r for r in recent if r['category'] == 'real-workload-candidate']
    return dict(schema_version=1, generated_at=datetime.fromtimestamp(now, timezone.utc).isoformat(),
                threshold_secs=threshold, recent_days=days, coverage=coverage,
                caveats=[
                    'All run directories counted, including missing or malformed summaries. Inventory is not an atomic snapshot.',
                    'Threshold applies strictly to recorded summary.wall, not full agent wait. Remote client_wall is separately reported where available, never substituted or added.',
                    'Aggregate wall and excess are sums across runs, not elapsed session time or guaranteed recoverable savings. Concurrent runs overlap.',
                    'Cause is the largest measured top-level phase, not a proven root cause. Phase coverage varies. Phase, unit, linker, and remote timings may overlap and are never added together.',
                    'Workload classes use path/argument heuristics. Real-workload-candidate may include experiments. Unknown dates excluded from recent; failures retained and grouped.',
                    'Paths and arguments are omitted. Unknown workspaces are hashed. Detail is bounded; all measured runs contribute to aggregates.'
                ], all=aggregate(rows, threshold, limit), recent=aggregate(recent, threshold, limit),
                recent_24h_real_workload_candidates=aggregate([r for r in rows if r['category'] == 'real-workload-candidate' and r['start'] is not None and now - 86400 <= r['start'] <= now], threshold, limit),
                recent_real_workload_candidates=aggregate(real, threshold, limit))


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--runs-dir', type=Path, default=Path.home() / '.justrust/runs')
    parser.add_argument('--threshold', type=float, default=10)
    parser.add_argument('--recent-days', type=float, default=7)
    parser.add_argument('--as-of', help='ISO timestamp, defaults to now (UTC)')
    parser.add_argument('--limit', type=int, default=12)
    parser.add_argument('--output', type=Path, help='Write report here, otherwise stdout')
    args = parser.parse_args()
    if number(args.threshold) is None or number(args.recent_days) is None or args.limit < 0:
        parser.error('threshold, recent-days and limit must be finite and nonnegative')
    now = datetime.fromisoformat(args.as_of.replace('Z', '+00:00')) if args.as_of else datetime.now(timezone.utc)
    if now.tzinfo is None:
        parser.error('--as-of must include a timezone')
    result = report(args.runs_dir, args.threshold, now.timestamp(), args.recent_days, args.limit)
    text = json.dumps(result, indent=2, allow_nan=False) + '\n'
    if args.output:
        args.output.write_text(text)
    else:
        print(text, end='')


if __name__ == '__main__':
    main()
