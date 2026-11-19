#!/usr/bin/env python3
"""Replay watcher lock policy over real paths, without running a build.

Run from the repository root. The mirror and generations are modeled. A real
canary file distinguishes scan generations. Foregrounds arrive after the canary
is read and before the rest of the metadata walk on each specified scan.
"""

import itertools
import json
import os
import tempfile
import threading
import time
from pathlib import Path


def replay(paths, budget, actions, canary):
    lock = threading.Lock()
    epoch, link, scans = 0, 1, 0
    mirror, waits, applied = {}, [], []
    threads = []
    canary.write_text('0')

    def scan(locked=False):
        nonlocal scans
        index = scans
        scans += 1
        value = canary.read_text()
        if index < len(actions):
            action = actions[index]
            started = threading.Event()

            def foreground():
                nonlocal epoch, link
                start = time.perf_counter()
                started.set()
                with lock:
                    waits.append(round((time.perf_counter() - start) * 1000, 3))
                    epoch += 1
                    if action == 'replace':
                        link += 1
                        mirror.clear()
                    elif action == 'disconnect':
                        link = 0
                        mirror.clear()
                    canary.write_text(str(index + 1))

            thread = threading.Thread(target=foreground)
            threads.append(thread)
            thread.start()
            started.wait()
        for path in paths:
            os.lstat(path)
        if not locked and index < len(actions):
            thread.join()
        return value

    start = time.monotonic()
    while link:
        retries = 0
        while True:
            with lock:
                token = (epoch, link)
            stamp = scan()
            with lock:
                if not link:
                    break
                if token != (epoch, link):
                    if retries < budget:
                        retries += 1
                        continue
                    stamp = scan(locked=True)
                assert stamp == canary.read_text()
                mirror[link] = stamp
                applied.append((link, stamp))
                break
        for thread in threads:
            thread.join()
        if not link or mirror.get(link) == canary.read_text():
            break
        # An edit after the locked commit generates a subsequent watch event.
    assert mirror == ({link: canary.read_text()} if link else {})
    assert len(waits) == len(actions)
    return dict(retries=budget, actions=actions, wait_ms=waits, scans=scans,
                elapsed_ms=round((time.monotonic() - start) * 1000), applied=applied)


def main():
    entries = (os.path.join(d, f) for d, _, files in os.walk('toolchain') for f in files)
    paths = list(itertools.islice(entries, 95000))
    assert len(paths) == 95000, 'run from the justrust stable checkout'
    for path in paths:
        os.lstat(path)
    with tempfile.TemporaryDirectory() as directory:
        for actions in [('edit', 'edit'), ('edit', 'replace', 'edit'), ('edit', 'disconnect')]:
            for budget in (0, 1):
                print(json.dumps(replay(paths, budget, actions, Path(directory) / 'canary')))


if __name__ == '__main__':
    main()
