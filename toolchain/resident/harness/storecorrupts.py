#!/usr/bin/env python3
"""storecorrupts.py: packs of the store damaged on disk, and stores of other formats.

A daemon fills a store and ends. Then, many times over, one file of the store is damaged (a
bit flipped, the file cut short, bytes added to it, part of it zeroed or the whole file
replaced) while no daemon runs or under a daemon that has already read it. Each time:

  - `ra-jit cache verify` has to say so when a sealed pack was damaged;
  - a daemon on the store has to build a program that prints what cargo's does, for a state
    of the workspace whose code was in the damaged store;
  - the daemon has to say it removed the pack;
  - `ra-jit cache verify` has to find nothing wrong afterwards.

Then the same for a store that says it is of an older format, of none, and of a newer one.

    python3 storecorrupts.py [--rounds N] [--seed N]
"""
import os, random, re, shutil, sys, time

HERE = os.path.dirname(os.path.abspath(__file__))
sys.path.insert(0, HERE)
from storelib import Checkout, Daemon, cache, files, scratch, stats, verify


def option(name, default):
    return int(sys.argv[sys.argv.index(name) + 1]) if name in sys.argv else default


ROUNDS, SEED = option("--rounds", 40), option("--seed", int(time.time()))
RNG = random.Random(SEED)
SEALS = {"RA_JIT_CACHE_SEAL_AT": "1", "RA_JIT_CACHE_MAX": "off"}
# The states of the workspace the store is filled with.
STATES = [0, 1, 2, 3]


def fail(what):
    raise SystemExit(f"FAIL (seed {SEED}): {what}")


def flip(data):
    at = RNG.randrange(len(data))
    data[at] ^= 1 << RNG.randrange(8)


def cut(data):
    del data[RNG.randrange(len(data)):]


def grow(data):
    data += RNG.randbytes(RNG.randrange(1, 100))


def zero(data):
    at = RNG.randrange(len(data))
    end = min(len(data), at + RNG.randrange(1, 4096))
    if not any(data[at:end]):
        data[at] = 1
    else:
        data[at:end] = bytes(end - at)


def replace(data):
    data[:] = RNG.randbytes(RNG.randrange(0, 200))


DAMAGES = [flip, flip, cut, cut, grow, zero, replace]


def damage(path):
    with open(path, "rb") as file:
        data = bytearray(file.read())
    how = RNG.choice(DAMAGES)
    how(data)
    with open(path, "wb") as file:
        file.write(data)
    return how.__name__


def fill(checkout, store):
    """A store with the code of every state in sealed packs; returns how many packs."""
    # A daemon writes the store when it builds everything, which is when it starts.
    for state in STATES:
        checkout.set(state)
        daemon = Daemon(checkout, store, env=SEALS)
        checkout.check(daemon, f"filling the store, state {state}")
        daemon.kill()
    # What the daemon left open is sealed by whoever comes next.
    status, out = cache(store, "gc", env=SEALS)
    if status != 0:
        fail(f"gc: {out}")
    whole, numbers, out = verify(store)
    # However many packs: a collection makes one of them once they are many and small.
    if not whole or numbers["packs"] < 1 or numbers["open packs"] != 0:
        fail(f"the filled store: {out}")
    return numbers["packs"]


def removed(daemon, pack):
    """Whether a daemon said it removed the pack. The same code compiled again is sealed under
    the same name, so a pack of that name may well be there afterwards."""
    with open(daemon.log_path, errors="replace") as log:
        return any(os.path.basename(pack) in line and line.rstrip().endswith("; removed") for line in log)


def clean(store, what):
    whole, numbers, out = verify(store)
    if not whole:
        fail(f"{what}: the store does not verify afterwards\n{out}")
    return numbers


def while_stopped(checkout, store, round):
    """Damages a sealed pack, then starts a daemon on the store."""
    pack = RNG.choice(files(store, ".pack"))
    how = damage(pack)
    what = f"round {round}: {how} of {os.path.basename(pack)} with no daemon"
    whole, _, out = verify(store)
    if whole:
        fail(f"{what}: verify finds nothing wrong\n{out}")
    checkout.set(RNG.choice(STATES))
    daemon = Daemon(checkout, store, env=SEALS)
    checkout.check(daemon, what)
    checkout.set(RNG.choice(STATES))
    checkout.check(daemon, what + ", after an edit")
    daemon.kill()
    if not removed(daemon, pack):
        fail(f"{what}: the daemon does not say it removed the pack: {daemon.log_path}")
    clean(store, what)


def while_running(checkout, store, round):
    """Damages a sealed pack that a daemon has read, then asks it for code that was in it."""
    checkout.set(RNG.choice(STATES))
    daemon = Daemon(checkout, store, env=SEALS)
    checkout.check(daemon, f"round {round}: before the damage")
    packs = files(store, ".pack")
    damaged = RNG.sample(packs, RNG.randrange(1, len(packs) + 1))
    how = [damage(pack) for pack in damaged]
    what = f"round {round}: {'/'.join(how)} of {len(damaged)} packs under a daemon"
    for state in RNG.sample(STATES, len(STATES)):
        checkout.set(state)
        checkout.check(daemon, f"{what}, state {state}")
    daemon.kill()
    # What the daemon did not need to read again is found by the next that reads it.
    daemon = Daemon(checkout, store, env=SEALS)
    checkout.check(daemon, what + ", the next daemon")
    daemon.kill()
    left = [pack for pack in damaged if not removed(daemon, pack)]
    if left:
        fail(f"{what}: the daemons do not say they removed {len(left)} of the packs: {daemon.log_path}")
    clean(store, what)


def open_pack(checkout, store, round):
    """Damages the pack a killed daemon left open."""
    checkout.set(RNG.randrange(100, 10**6))
    daemon = Daemon(checkout, store, env={"RA_JIT_CACHE_MAX": "off"})
    checkout.check(daemon, f"round {round}: before the damage")
    daemon.kill()
    left = files(store, ".open")
    if len(left) != 1:
        fail(f"round {round}: {len(left)} open packs left by one daemon")
    how = damage(left[0])
    what = f"round {round}: {how} of an open pack"
    # An open pack is not sealed yet, so whatever is in it is no error: only what is whole is kept.
    clean(store, what + ", before it is sealed")
    daemon = Daemon(checkout, store, env=SEALS)
    checkout.check(daemon, what)
    daemon.kill()
    if files(store, ".open") != []:
        fail(f"{what}: it was not sealed or removed")
    clean(store, what)


def formats(checkout, store):
    """A store that says it is of another format than this ra-jit writes."""
    version = os.path.join(store, "version")
    fill(checkout, store)
    current = open(version).read()

    for text, name in [("ra-jit store 1\n", "an older format"), ("", "an empty version"),
                       ("ra-jit store two\n", "a version that is no number"), (None, "no version at all")]:
        fill(checkout, store)
        os.remove(version) if text is None else open(version, "w").write(text)
        before = set(files(store, ".pack"))
        open(os.path.join(store, "notes.txt"), "w").write("not the store's\n")
        daemon = Daemon(checkout, store, env=SEALS)
        checkout.check(daemon, name)
        daemon.kill()
        said = re.search(r"store: (\d+) files of another format discarded", daemon.log())
        if not said:
            fail(f"{name}: the daemon does not say it discarded the store: {daemon.log_path}")
        # The same code compiled again is sealed under the same name, so the names say nothing.
        if int(said[1]) < len(before):
            fail(f"{name}: {said[1]} files discarded of a store of {len(before)} packs: {daemon.log_path}")
        if open(version).read() != current or not os.path.exists(os.path.join(store, "notes.txt")):
            fail(f"{name}: the version is not this one's, or a file that is not the store's is gone")
        clean(store, name)

    packs = fill(checkout, store)
    open(version, "w").write("ra-jit store 999\n")
    before = sorted(os.listdir(store))
    daemon = Daemon(checkout, store, env=SEALS)
    checkout.set(RNG.randrange(100, 10**6))
    checkout.check(daemon, "a newer format")
    daemon.kill()
    if "store: not used" not in daemon.log() or "format 999" not in daemon.log():
        fail(f"a newer format: the daemon does not say it leaves the store alone: {daemon.log_path}")
    if sorted(os.listdir(store)) != before:
        fail("a newer format: the store was changed")
    status, out = cache(store, "verify")
    if status == 0 or "999" not in out:
        fail(f"a newer format: verify does not say so\n{out}")
    status, out = cache(store, "gc")
    if status == 0 or sorted(os.listdir(store)) != before:
        fail(f"a newer format: gc did not refuse\n{out}")
    return packs


def main():
    start = time.time()
    work = scratch("ra-jit-storecorrupts-")
    store = os.path.join(work, "store")
    checkout = Checkout(work)
    counts = {"with no daemon": 0, "under a daemon": 0, "an open pack": 0}
    for round in range(ROUNDS):
        if round % 8 == 0 or not files(store, ".pack"):
            shutil.rmtree(store, ignore_errors=True)
            fill(checkout, store)
        kind = RNG.choice([while_stopped, while_stopped, while_running, open_pack])
        kind(checkout, store, round)
        counts[{while_stopped: "with no daemon", while_running: "under a daemon", open_pack: "an open pack"}[kind]] += 1
    shutil.rmtree(store, ignore_errors=True)
    formats(checkout, store)
    shutil.rmtree(work, ignore_errors=True)
    print(f"damaged: {counts}; formats: 4 older or unreadable discarded, 1 newer refused")
    print(f"PASS: {ROUNDS} damaged stores detected, discarded and rebuilt (seed {SEED}, {time.time() - start:.0f}s)")


if __name__ == "__main__":
    main()
