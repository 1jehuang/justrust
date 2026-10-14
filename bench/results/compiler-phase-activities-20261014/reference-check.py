#!/usr/bin/env python3
"""Independent discrete-time reference, not a copy of the stack algorithm."""
from collections import Counter
from pathlib import Path
import random
import sys

sys.path.insert(0, str(Path(__file__).resolve().parents[2]))
import query_activities as activities

rng = random.Random(19417)
for trial in range(250):
    records = []

    def branch(start, end, depth, thread):
        if end <= start:
            return
        records.append(dict(record_type="interval", kind=rng.choice([
            "Query", "GenericActivity", "Unknown"]),
            label=rng.choice(["alpha", "beta", None]), thread=thread,
            start_ns=start, end_ns=end))
        if depth and end - start >= 4:
            cut = rng.randrange(start + 1, end)
            branch(start, cut, depth - 1, thread)
            branch(cut, end, depth - 1, thread)

    for thread in range(3):
        branch(0, 32, 4, thread)
    rng.shuffle(records)
    expected = Counter()
    coverage = 0
    for thread in range(3):
        for tick in range(32):
            candidates = [(e["end_ns"] - e["start_ns"], -ordinal, e)
                          for ordinal, e in enumerate(records)
                          if e["thread"] == thread and e["start_ns"] <= tick < e["end_ns"]]
            if candidates:
                selected = min(candidates, key=lambda item: item[:2])[2]
                expected[selected["kind"], selected["label"]] += 1
                coverage += 1
    actual = activities.summarize_events(iter(records))
    observed = Counter({(row["kind"], row["label"]): row["exclusive_elapsed_ns"]
                        for row in actual["rows"]})
    assert observed == expected, trial
    assert sum(observed.values()) == coverage == actual["covered_elapsed_ns"], trial
    assert actual["cpu_work_ns"] is None and actual["execution_span_ns"] is None
print("PASS: 250 fixed-seed shuffled nested forests, 3 threads, exhaustive per-time-cell attribution, covered/exclusive equality and CPU/span null")
