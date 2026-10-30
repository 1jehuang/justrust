#!/usr/bin/env python3
"""storeshared.py: several daemons and `ra-jit cache` commands on one store at once.

Every daemon has a checkout of its own and all of them have the same store. They are edited
and asked for their programs at the same time, with states of the workspace that the others
build too, and killed and started again every few builds, so that they add the same keys,
seal packs side by side and read each other's.

First with room for everything: afterwards a daemon that has compiled nothing has to find
nearly all of its code in what the others sealed. Then with a store that is far too small,
while `ra-jit cache gc` evicts and merges in a loop and `ra-jit cache verify` reads the store
in a loop: every program has to print what cargo's does, and verify must never find a pack
that is not what was written.

    python3 storeshared.py [--daemons N] [--rounds N] [--seed N]
"""
import os, random, re, shutil, sys, threading, time

HERE = os.path.dirname(os.path.abspath(__file__))
sys.path.insert(0, HERE)
from storelib import Checkout, Daemon, cache, scratch, stats, verify


def option(name, default):
    return int(sys.argv[sys.argv.index(name) + 1]) if name in sys.argv else default


DAEMONS, ROUNDS, SEED = option("--daemons", 3), option("--rounds", 30), option("--seed", int(time.time()))
ROOMY = {"RA_JIT_CACHE_SEAL_AT": "1", "RA_JIT_CACHE_MAX": "off"}
TIGHT = {"RA_JIT_CACHE_SEAL_AT": "1", "RA_JIT_CACHE_MAX": "1500K"}
STATES = 6
FAILURES = []


def guarded(work):
    def run(*args):
        try:
            work(*args)
        except (SystemExit, Exception) as failure:
            FAILURES.append(failure)
    return run


def together(works):
    threads = [threading.Thread(target=guarded(work), args=args) for work, *args in works]
    for thread in threads:
        thread.start()
    for thread in threads:
        thread.join()
    if FAILURES:
        raise SystemExit("\n".join(f"FAIL (seed {SEED}): {failure}" for failure in FAILURES))


def edits(checkout, store, number, env):
    """Edits a checkout and asks its daemon for the program, ROUNDS times. A daemon writes the
    store when it builds everything, so each is killed and started again every few rounds."""
    rng = random.Random(SEED * 100 + number)
    daemon = Daemon(checkout, store, name=f"daemon{number}", env=env)
    for round in range(ROUNDS):
        if round % 3 == 2:
            daemon.kill()
            daemon = Daemon(checkout, store, name=f"daemon{number}", env=env)
        checkout.set(rng.randrange(STATES))
        checkout.check(daemon, f"daemon {number}, round {round}")
    daemon.kill()


def collecting(store, done, counts):
    rng = random.Random(SEED)
    while not done.is_set():
        status, out = cache(store, "gc", "--max", rng.choice(["0", "400K", "1M", "3M"]))
        # Two collections at once: the other is told that one is running, which is no failure.
        if status != 0 and "another process is collecting" not in out:
            raise SystemExit(f"gc: {out}")
        counts["collections"] += 1
        counts["evicted"] += int(re.search(r"evicted packs: (\d+)", out).group(1)) if status == 0 else 0
        time.sleep(rng.uniform(0, 0.3))


def verifying(store, done, counts):
    while not done.is_set():
        whole, _, out = verify(store)
        if not whole:
            raise SystemExit(f"verify, while the daemons run:\n{out}")
        counts["verifications"] += 1


def found(daemon):
    """(functions found in the store, functions compiled) by the first build of a daemon."""
    line = re.search(r"store: (\d+) functions found, (\d+) compiled and added", daemon.log())
    return (int(line.group(1)), int(line.group(2))) if line else (0, 0)


def main():
    start = time.time()
    work = scratch("ra-jit-storeshared-")
    store = os.path.join(work, "store")
    checkouts = [Checkout(work, f"checkout{n}") for n in range(DAEMONS + 1)]
    # cargo's builds of every state, before anything runs at once.
    for checkout in checkouts:
        for state in list(range(STATES)) + [0]:
            checkout.set(state)
            checkout.expected()

    together([(edits, checkout, store, n, ROOMY) for n, checkout in enumerate(checkouts[:DAEMONS])])
    whole, numbers, out = verify(store)
    if not whole:
        raise SystemExit(f"FAIL (seed {SEED}): the store after the daemons shared it\n{out}")
    late = Daemon(checkouts[DAEMONS], store, name="late", env=ROOMY)
    checkouts[DAEMONS].check(late, "a daemon that compiled nothing")
    late.kill()
    hits, misses = found(late)
    if hits == 0 or misses * 20 > hits:
        raise SystemExit(f"FAIL: a new daemon found {hits} functions in the store and compiled {misses}: {late.log_path}")
    print(f"roomy: {DAEMONS} daemons, {ROUNDS} builds each; {numbers['packs']} packs, {numbers['records']} records; "
          f"a new daemon found {hits} functions and compiled {misses}")

    done, counts = threading.Event(), {"collections": 0, "evicted": 0, "verifications": 0}
    others = [threading.Thread(target=guarded(work), args=(store, done, counts)) for work in (collecting, collecting, verifying)]
    for thread in others:
        thread.start()
    try:
        together([(edits, checkout, store, n, TIGHT) for n, checkout in enumerate(checkouts[:DAEMONS])])
    finally:
        done.set()
        for thread in others:
            thread.join()
    if FAILURES:
        raise SystemExit("\n".join(f"FAIL (seed {SEED}): {failure}" for failure in FAILURES))
    whole, numbers, out = verify(store)
    if not whole:
        raise SystemExit(f"FAIL (seed {SEED}): the store after the daemons and the collections\n{out}")
    if counts["evicted"] == 0 or counts["verifications"] == 0:
        raise SystemExit(f"FAIL: nothing was evicted or verified while the daemons ran: {counts}")
    shutil.rmtree(work, ignore_errors=True)
    print(f"tight: {DAEMONS} daemons, {ROUNDS} builds each, {counts['collections']} collections that evicted "
          f"{counts['evicted']} packs, {counts['verifications']} verifications")
    print(f"PASS: {DAEMONS * ROUNDS * 2} builds on a shared store (seed {SEED}, {time.time() - start:.0f}s)")


if __name__ == "__main__":
    main()
