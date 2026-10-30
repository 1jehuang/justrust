#!/usr/bin/env python3
"""storefirst.py: the first edit under a daemon that took code of the checkout from the store
(issue #12).

A daemon builds the stress workspace with an empty store and ends. A second one, on the same
store and checkout, builds it again: what it has of the checkout then comes from the store,
and nothing of that says where in the checkout it came from, so no patch can replace it. Once
it has answered, it compiles all of that again by itself, while nothing waits. Then an edit:

  - the second daemon has to take code of the checkout from the store, and say that it then
    compiles it again;
  - that build has to be over within a minute, with nobody asking for it;
  - the edit after it has to be spliced, not linked in full;
  - the program has to print what cargo's does after the second daemon's first build, after
    the build it did by itself, and after the edit.

    python3 storefirst.py
"""
import os, re, shutil, sys, time

HERE = os.path.dirname(os.path.abspath(__file__))
sys.path.insert(0, HERE)
from storelib import Checkout, Daemon, scratch

AGAIN = "compiling what the store had of the checkout"


def links(text):
    return re.findall(r"^link: \S+ \((.*)\)$", text, re.M)


def main():
    work, store = scratch("ra-jit-storefirst-"), scratch("ra-jit-storefirst-store-")
    daemons, passed = [], False
    try:
        checkout = Checkout(work)
        first = Daemon(checkout, store, name="first")
        daemons.append(first)
        checkout.check(first, "the first daemon's build")
        first.kill()

        second = Daemon(checkout, store, name="second")
        daemons.append(second)
        checkout.check(second, "the build from the store")
        found = [int(n) for n in re.findall(r"^entries: (\d+) found", second.log(), re.M)]
        if not found or found[0] == 0:
            raise SystemExit(f"FAIL: the second daemon found nothing in the store: {second.log_path}")
        if AGAIN not in second.log():
            raise SystemExit(f"FAIL: the second daemon took {found[0]} entries from the store "
                             f"and does not compile what it took of the checkout: {second.log_path}")
        deadline = time.monotonic() + 60
        while not links(second.log().split(AGAIN, 1)[1]):
            if time.monotonic() > deadline:
                raise SystemExit(f"FAIL: the build nobody asked for is not over after a minute: {second.log_path}")
            time.sleep(0.1)
        checkout.check(second, "the build the daemon did by itself")

        seen = len(second.log())
        checkout.set(1)
        checkout.check(second, "the first edit")
        how = links(second.log()[seen:])
        if not how or not how[-1].startswith("spliced"):
            raise SystemExit(f"FAIL: the first edit was not spliced: {how[-1] if how else 'no link'}; "
                             f"log {second.log_path}")
        print(f"PASS: {found[0]} entries from the store, what of them is the checkout's compiled again by the daemon by itself; "
              f"the first edit {how[-1]}")
        passed = True
    finally:
        for daemon in daemons:
            if daemon.process.poll() is None:
                daemon.kill()
        if passed:
            shutil.rmtree(work, ignore_errors=True)
            shutil.rmtree(store, ignore_errors=True)


if __name__ == "__main__":
    main()
