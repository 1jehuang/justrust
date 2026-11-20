#!/usr/bin/env python3
"""Rank executed proc macros from a single rustc self-profile (not cache queries).

Pass a .mm_profdata captured with -Zself-profile-events=generic-activity,args
or a crox chrome_profiler.json. Capture the test unit, not its dependencies.
Self time excludes the union of recorded same-thread child intervals. It still
includes uninstrumented bridge, allocator and macro code; it is not CPU time.
"""
import argparse
from collections import defaultdict
import json
from pathlib import Path
import subprocess
import tempfile


def rank(events):
    threads = defaultdict(list)
    for e in events:
        if e['ph'] == 'X' and e['dur'] > 0:
            threads[e['pid'], e['tid']].append(e)
    groups = defaultdict(lambda: dict(calls=0, inclusive_us=0, self_us=0))
    # Keep zero-duration calls in counts (crox rounds to microseconds).
    for e in events:
        if e['ph'] == 'X' and e['name'] == 'expand_proc_macro':
            name = (e.get('args') or {}).get('arg0')
            if not name:
                raise ValueError('missing macro name: capture with self-profile event `args`')
            groups[name]['calls'] += 1
            groups[name]['inclusive_us'] += e['dur']
    if not groups:
        raise ValueError('profile contains no executed proc macros')
    for thread in threads.values():
        # crox emits complete events in completion order, including children
        # before their parent when both have identical rounded endpoints.
        stack = []
        for _, e in sorted(enumerate(thread), key=lambda p: (p[1]['ts'], -p[1]['dur'], -p[0])):
            start, end = e['ts'], e['ts'] + e['dur']
            while stack and start >= stack[-1][0]:
                stack.pop()
            if stack and end > stack[-1][0]:
                # crox independently rounds timestamps and durations to us.
                if end > stack[-1][0] + 1:
                    raise ValueError('crossing same-thread intervals: cannot attribute self time')
                end = stack[-1][0]
            if stack and stack[-1][1] is not None:
                groups[stack[-1][1]]['self_us'] -= end - start
            macro = e['args']['arg0'] if e['name'] == 'expand_proc_macro' else None
            if macro is not None:
                groups[macro]['self_us'] += end - start
            stack.append((end, macro))
    return [dict(macro=k, **v) for k, v in sorted(groups.items(), key=lambda p: (-p[1]['self_us'], p[0]))]


def main():
    ap = argparse.ArgumentParser(description=__doc__)
    ap.add_argument('profile', type=Path)
    args = ap.parse_args()
    with tempfile.TemporaryDirectory() as tmp:
        path = args.profile.resolve()
        if path.suffix == '.mm_profdata':
            subprocess.run(['crox', str(path)], cwd=tmp, check=True)
            path = Path(tmp, 'chrome_profiler.json')
        print(json.dumps(rank(json.loads(path.read_text())), indent=2))


if __name__ == '__main__':
    main()
