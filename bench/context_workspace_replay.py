"""Offline paired workspace-probe replay, excluding JSON decode and reverse I/O."""
import argparse
import json
from pathlib import Path
from time import perf_counter


def workspace(cwd):
    path = Path(cwd)
    try:
        path = path.resolve(strict=True)
    except (OSError, RuntimeError):
        pass
    nearest = None
    for parent in (path, *path.parents):
        try:
            manifest = (parent / 'Cargo.toml').read_text()
        except (OSError, UnicodeError):
            continue
        if any(line.strip() == '[workspace]' for line in manifest.splitlines()):
            return parent
        nearest = nearest or parent
    return nearest or path


def replay(rows, cwd, session, bounded):
    root = workspace(cwd)
    probes = 1
    misses = set()
    result = None
    for rid, directory, agent in rows:
        if session is not None and session != agent:
            continue
        if bounded and directory in misses:
            continue
        if bounded and directory == cwd:
            result = rid
            break
        probes += 1
        if workspace(directory) == root:
            result = rid
            break
        if bounded:
            if len(misses) == 128:
                misses.clear()
            misses.add(directory)
    if not bounded:
        workspace(cwd)  # eager diagnostic
        probes += 1
    return result, probes


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('index', type=Path)
    args = parser.parse_args()
    rows = []
    for line in args.index.read_bytes().splitlines():
        try:
            row = json.loads(line)
            rows.append((row['id'], row['cwd'], row.get('agent_session')))
        except (ValueError, KeyError):
            continue
    rows.reverse()
    # Choose the cwd whose last literal occurrence is oldest.
    last = {}
    for i, (_, directory, _) in enumerate(rows):
        last.setdefault(directory, i)
    active = next(row for row in rows if row[2] and row[2] != rows[0][2])
    for name, cwd, session in [('old', max(last, key=last.get), None),
                               ('missing', '/nonexistent-jr-lookup-replay', None),
                               ('active', active[1], active[2]), ('newest', rows[0][1], None)]:
        results = []
        for bounded in (False, True):
            start = perf_counter()
            rid, probes = replay(rows, cwd, session, bounded)
            print(name, 'bounded' if bounded else 'baseline', probes, 'resolver calls',
                  round(1000 * (perf_counter() - start), 3), 'ms', flush=True)
            results.append(rid)
        assert results[0] == results[1]


if __name__ == '__main__':
    main()
