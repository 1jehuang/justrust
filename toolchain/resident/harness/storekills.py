#!/usr/bin/env python3
"""storekills.py: the daemon killed at random moments while it writes its store.

Each worker has a checkout of the stress workspace and a store of its own. A kill is one of:

  cold     the store is emptied, a daemon is started on it and killed before or while it
           compiles everything and writes it out
  start    the same on the store as the kill before left it
  edit     a daemon that has started is asked for the program after an edit, and killed
           before, while or after it compiles, links and writes the store
  settle   the same, with the kill around the moment the daemon answers, which is when it
           seals packs and collects garbage
  write    a daemon is started on a program the store has not seen, and killed at the
           moment the store is seen to change for the first, second or third time: an open
           pack that grows, a pack that gets its name, a merged pack on its way to its name

The moment is random, SIGKILL is the signal, and the daemons are started with sizes that make
them seal, merge and evict on nearly every build. After every kill `ra-jit cache verify` has to
find nothing in the store that is not what was written, and a new daemon on the same store
has to build a program that prints what cargo's does.

    python3 storekills.py [--kills N] [--workers N] [--seed N]
"""
import os, random, shutil, sys, threading, time

HERE = os.path.dirname(os.path.abspath(__file__))
sys.path.insert(0, HERE)
from storelib import Checkout, Daemon, files, scratch, verify


def option(name, default):
    return int(sys.argv[sys.argv.index(name) + 1]) if name in sys.argv else default


KILLS, WORKERS, SEED = option("--kills", 300), option("--workers", 4), option("--seed", int(time.time()))
MODES = ["cold", "start", "start", "edit", "edit", "settle", "write", "write"]
# What a daemon is started with: packs sealed at every build or seldom, a store with room or one
# that evicts all the time.
SIZES = [{"RA_JIT_CACHE_SEAL_AT": "1"}, {"RA_JIT_CACHE_SEAL_AT": "1", "RA_JIT_CACHE_MAX": "2M"},
         {"RA_JIT_CACHE_SEAL_AT": "64K", "RA_JIT_CACHE_MAX": "off"}, {}]


class Worker:
    def __init__(self, number, kills):
        self.rng = random.Random(SEED * 1000 + number)
        self.work = scratch(f"ra-jit-storekills-{number}-")
        self.store = os.path.join(self.work, "store")
        self.checkout = Checkout(self.work)
        self.kills, self.number = kills, number
        self.counts = {mode: 0 for mode in set(MODES)}
        self.found = {"before it was watching": 0, "before it answered": 0, "after it answered": 0,
                      "as the store changed": 0,
                      "open packs left": 0, "torn bytes left": 0, "temporary files left": 0}
        self.start_time, self.sync_time = 5.0, 0.1
        self.failure = None

    def daemon(self, wait=True):
        return Daemon(self.checkout, self.store, env=self.rng.choice(SIZES), wait=wait)

    def checked(self, what):
        """A new daemon on the store as the kill left it, after its program and the store were checked."""
        whole, numbers, out = verify(self.store)
        if not whole:
            raise SystemExit(f"FAIL {what}: the store does not verify\n{out}")
        self.found["open packs left"] += numbers.get("open packs", 0) > 0
        self.found["torn bytes left"] += numbers.get("torn bytes", 0) > 0
        self.found["temporary files left"] += bool(files(self.store, ".tmp"))
        start = time.time()
        daemon = self.daemon()
        self.start_time = time.time() - start
        self.checkout.check(daemon, what)
        whole, _, out = verify(self.store)
        if not whole:
            raise SystemExit(f"FAIL {what}: the store does not verify after the build\n{out}")
        return daemon

    def listing(self):
        """The files of the store and their sizes."""
        try:
            with os.scandir(self.store) as entries:
                return sorted((entry.name, entry.stat().st_size) for entry in entries)
        except FileNotFoundError:
            return []

    def kill_writing(self):
        """A daemon is started on a program the store has not seen, and killed a moment after
        the store is seen to be other than it was."""
        self.checkout.set(self.rng.randrange(10**6))
        before, give_up = self.listing(), time.time() + 2 * self.start_time + 1
        daemon = Daemon(self.checkout, self.store, env=self.rng.choice(SIZES[:2]), wait=False)
        changes = self.rng.randrange(1, 4)
        while time.time() < give_up and changes:
            now = self.listing()
            changes -= now != before
            before = now
        self.found["as the store changed" if not changes else "after it answered"] += 1
        time.sleep(self.rng.choice([0, 0, self.rng.uniform(0, 0.003)]))
        daemon.kill()

    def kill_starting(self, mode):
        if mode == "cold":
            shutil.rmtree(self.store, ignore_errors=True)
        daemon = self.daemon(wait=False)
        time.sleep(self.rng.uniform(0, self.start_time * 1.1))
        self.found["before it was watching" if not daemon.started() else "after it answered"] += 1
        daemon.kill()

    def kill_building(self, daemon, mode):
        # Half of the edits are to what some build before has seen, and is in the store.
        self.checkout.set(self.rng.choice([self.rng.randrange(6), self.rng.randrange(10**6)]))
        client, start = daemon.asking(), time.time()
        low = 0.7 if mode == "settle" else 0.0
        kill_at = start + self.rng.uniform(low * self.sync_time, 1.2 * self.sync_time)
        while time.time() < kill_at and client.poll() is None:
            time.sleep(0.0005)
        answered = client.poll() is not None
        if answered:
            # How long an answer takes, for the kills to come to fall inside one.
            self.sync_time = 0.7 * self.sync_time + 0.3 * (time.time() - start)
            time.sleep(self.rng.uniform(0, 0.01))
        daemon.kill()
        client.communicate()
        self.found["after it answered" if answered else "before it answered"] += 1

    def run(self):
        try:
            daemon = self.checked("before any kill")
            for kill in range(self.kills):
                mode = self.rng.choice(MODES)
                self.counts[mode] += 1
                if mode in ("cold", "start"):
                    daemon.kill()
                    self.kill_starting(mode)
                elif mode == "write":
                    daemon.kill()
                    self.kill_writing()
                else:
                    self.kill_building(daemon, mode)
                daemon = self.checked(f"worker {self.number}, kill {kill} ({mode}, seed {SEED})")
            daemon.kill()
        except (SystemExit, Exception) as failure:
            self.failure = failure


def main():
    start = time.time()
    workers = [Worker(n, KILLS // WORKERS + (n < KILLS % WORKERS)) for n in range(WORKERS)]
    threads = [threading.Thread(target=worker.run) for worker in workers]
    for thread in threads:
        thread.start()
    for thread in threads:
        thread.join()
    failures = [worker for worker in workers if worker.failure is not None]
    for worker in failures:
        print(f"{worker.failure}\n  kept: {worker.work}")
    for worker in workers:
        if worker.failure is None:
            shutil.rmtree(worker.work, ignore_errors=True)
    done = sum(sum(worker.counts.values()) for worker in workers)
    counts = {mode: sum(worker.counts[mode] for worker in workers) for mode in sorted(set(MODES))}
    found = {what: sum(worker.found[what] for worker in workers) for what in workers[0].found}
    print(f"kills: {done} {counts}")
    print(f"found: {found}")
    print(f"seed {SEED}, {time.time() - start:.0f}s")
    if failures or done != KILLS:
        raise SystemExit("FAIL")
    print(f"PASS: {done} kills, every store verified and every next build printed what cargo's does")


if __name__ == "__main__":
    main()
