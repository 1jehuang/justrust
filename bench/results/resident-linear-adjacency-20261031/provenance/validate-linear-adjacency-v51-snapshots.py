#!/usr/bin/env python3
"""Visible worker-log snapshot work only, not fresh phase attribution. Diagnostic timings invalid."""
import collections
import json
import re
from pathlib import Path
ROOT = Path(__file__).resolve().parent
OUT = ROOT / 'linear-adjacency-v51-diagnostic'
state = json.loads((OUT / 'state.json').read_text())
assert state['status'] == 'passed' and state['unchanged']
pattern = re.compile(r'native-adjacency-dual-v51 timings_invalid=true selector_calls=(\d+) yielded=(\d+) visits=(\d+) raw_lengths=(\d+) max_raw_length=(\d+) fanout=\[([0-9, ]+)\] exhausted=(\d+) aborted=(\d+) candidate_inspections=(\d+) original_inspections=(\d+)')
previous = collections.Counter()
rows = []
for index, row in enumerate(state['rows']):
    assert row['accepted'] and row['input_unchanged'] and not row['failures']
    logs = list(OUT.glob(f'{index:02}-{row["phase"]}-worker-*.log'))
    assert logs, 'positive worker log capture required'
    current = collections.Counter()
    for log in logs:
        for line in log.read_text(errors='strict').splitlines():
            if 'native-adjacency-dual-v51' in line:
                match = pattern.search(line)
                assert match, 'malformed diagnostic marker'
                current[match.group(0)] += 1
    reset_or_replaced = bool(previous-current)
    repeated_visible_counts = current == previous
    delta = current
    metrics = collections.Counter()
    hist = None
    for text, count in delta.items():
        match = pattern.fullmatch(text)
        calls, yielded, visits, lengths, maximum = map(int, match.groups()[:5])
        fanout = list(map(int, match.group(6).split(',')))
        exhausted, aborted, candidate, original = map(int, match.groups()[6:])
        assert visits > 0 and sum(fanout) == visits
        assert exhausted + aborted == visits and calls == yielded + exhausted
        assert candidate <= lengths and candidate <= original
        if hist is None:
            hist = [0] * len(fanout)
        assert len(hist) == len(fanout)
        hist = [a + count*b for a,b in zip(hist, fanout)]
        for key,value in dict(selector_calls=calls,yielded=yielded,visits=visits,raw_lengths=lengths,exhausted=exhausted,aborted=aborted,candidate_inspections=candidate,original_inspections=original).items():
            metrics[key] += count*value
        metrics['max_raw_length'] = max(metrics['max_raw_length'],maximum)
    rows.append(dict(captured_after=row['phase'],report_count=sum(delta.values()),snapshot_metrics=dict(metrics),worker_snapshot_reset_or_replaced=reset_or_replaced,repeats_previous_visible_report_counts=repeated_visible_counts,fanout=hist,original_over_candidate=(metrics['original_inspections']/metrics['candidate_inspections'] if metrics['candidate_inspections'] else None)))
    previous = current
assert sum(row['report_count'] for row in rows) > 0, 'positive native paired selector activation required'
print(json.dumps(dict(status='passed',scope=__doc__,native_rows=len(rows),reports_in_final_snapshot=sum(previous.values()),warning='Worker logs reset or are replaced, do not sum snapshots or infer fresh phase work. Zero visible reports are not independent proof of zero work.',rows=rows),indent=2))
