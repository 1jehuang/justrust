#!/usr/bin/env python3
"""Validate and archive native bounded-CGU Desktop A/B receipts, not release provenance.

Input directory: native.log (JSONL: i=0 warmup, 1..6 edits, 7 full suite),
cold.log (i=8..11 fresh UI unit with warm dependencies), and matching
native-{control,candidate}-7.log Cargo output. Same-cap, immutable-package
acceptance runs can separately use reuse-cgu-desktop.py --compiler-pair.
"""
import argparse
import hashlib
import json
from pathlib import Path
import re
from statistics import median


def check(condition, message):
    if not condition:
        raise ValueError(message)


def evaluate(rows, full):
    arms = ('control', 'candidate')
    samples = {(r['arm'], r['i']): r for r in rows}
    check(len(rows) == len(samples) == 24, 'missing or duplicate samples')
    for i in range(12):
        pair = [samples[a, i] for a in arms]
        check(pair[0]['source'] == pair[1]['source'] and re.fullmatch('[a-f0-9]{64}', pair[0]['source']),
              f'round {i}: different source inputs')
        for r in pair:
            check(r['units'] == 1 and 0 < sum(r['cgu'].values()) <= 256, f'round {i}: not one capped unit')
            check(all(r[k] > 0 for k in ('compile', 'link', 'rss_mb', 'wall')), f'round {i}: missing metrics')
            if i == 0 or i >= 8:
                check(set(r['cgu']) == {'new'}, f'round {i}: reused a cold CGU')
        if 1 <= i <= 6:
            check(pair[0]['cgu'].get('new', 0) > 0, f'round {i}: missing control churn')
            check(pair[1]['cgu'].get('new', 0) == 0 and pair[1]['cgu'].get('loaded', 0) > 0,
                  f'round {i}: unstable candidate')
    check(full[0] == full[1] and full[0][0] > 0 and full[0][1] == 0 and full[0][3] == 0,
          'full-suite counts differ, failed, or filtered')
    result = {}
    for mode, rounds in [('warm', range(1, 7)), ('cold', range(8, 12))]:
        result[mode] = {a: {k: median(samples[a, i][k] for i in rounds)
                            for k in ('wall', 'compile', 'link', 'rss_mb')} for a in arms}
    wins = sum(samples['candidate', i]['wall'] < samples['control', i]['wall'] for i in range(1, 7))
    check(wins >= 5 and result['warm']['candidate']['wall'] < result['warm']['control']['wall'],
          'no reliable foreground win')
    for metric in ('wall', 'rss_mb'):
        check(result['cold']['candidate'][metric] <= 1.05 * result['cold']['control'][metric],
              f'cold {metric} regressed by more than 5%')
    return dict(medians=result, wins=wins, suite=full[0])


def digest(path):
    with path.open('rb') as f:
        return hashlib.file_digest(f, 'sha256').hexdigest()


def main():
    ap = argparse.ArgumentParser(description=__doc__)
    ap.add_argument('directory', type=Path)
    ap.add_argument('control_driver', type=Path)
    ap.add_argument('candidate_driver', type=Path)
    ap.add_argument('output', type=Path)
    a = ap.parse_args()
    rows = [json.loads(line) for f in ('native.log', 'cold.log')
            for line in (a.directory / f).read_text().splitlines()]
    counts = []
    for arm in ('control', 'candidate'):
        text = (a.directory / f'native-{arm}-7.log').read_text()
        match, = re.findall(r'test result: ok\. (\d+) passed; (\d+) failed; (\d+) ignored; 0 measured; (\d+) filtered out;', text)
        counts.append(tuple(map(int, match)))
    report = evaluate(rows, counts)
    drivers = list(map(digest, (a.control_driver, a.candidate_driver)))
    check(drivers[0] != drivers[1], 'identical compiler binaries')
    with a.output.open('x') as out:
        json.dump(dict(schema=1, scope='isolated hosted Desktop UI target unit', drivers=drivers,
                       samples=rows, **report), out)
    print(json.dumps(report))


if __name__ == '__main__':
    main()
