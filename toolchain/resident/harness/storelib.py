"""storelib.py: a checkout of the stress workspace under a daemon that keeps a store.

What storekills.py, storecorrupts.py, storefull.py and storeshared.py have in common: the
checkout, cargo's build of it to compare with, the daemon and its store, and `ra-jit cache`
on that store.
"""
import os, shutil, subprocess, sys, tempfile, time

HERE = os.path.dirname(os.path.abspath(__file__))
sys.path.insert(0, HERE)
import linkline
from difftest import Fixture, RA_JIT, sysroot_rlibs, RUNNER, TARGET_ARGS, built

CRATES = ["sapp", "sbase", "smid"]
MID, APP = "mid/src/lib.rs", "app/src/main.rs"
AREA = "sbase::square!(self.side)"


def scratch(prefix):
    # cargo names a checkout by its real path, and the temporary directory of macOS is a link.
    # That directory is also too long for the path of a daemon's socket in it (104 bytes).
    return os.path.realpath(tempfile.mkdtemp(prefix=prefix, dir="/tmp" if sys.platform == "darwin" else None))


def cache(store, *args, env=None):
    """Runs `ra-jit cache ARGS` on the store; returns (exit status, what it printed)."""
    ran = subprocess.run([RA_JIT, "cache", *args], capture_output=True, text=True,
                         env={**os.environ, **(env or {}), "RA_JIT_CACHE": store})
    return ran.returncode, ran.stdout + ran.stderr


def numbers(out):
    """The `name: number` lines of what a `ra-jit cache` command printed, as a dictionary."""
    found = {}
    for line in out.splitlines():
        name, _, value = line.partition(":")
        first = value.split()[0] if value.split() else ""
        if first.isdigit():
            found[name.strip()] = int(first)
    return found


def stats(store, env=None):
    status, out = cache(store, "stats", env=env)
    assert status == 0, out
    return numbers(out)


def verify(store):
    """`ra-jit cache verify`: (whether everything in the store is what was written, its numbers, its output)."""
    status, out = cache(store, "verify")
    return status == 0, numbers(out), out


def files(store, suffix):
    """The files of the store whose names end so; a store that is not there has none."""
    if not os.path.isdir(store):
        return []
    return sorted(os.path.join(store, name) for name in os.listdir(store) if name.endswith(suffix))


class Checkout:
    """One copy of the stress workspace, with what cargo makes of it as the reference."""

    def __init__(self, work, name="checkout"):
        self.work, self.ws = work, os.path.join(work, name)
        shutil.copytree(os.path.join(HERE, "stress"), self.ws, ignore=shutil.ignore_patterns("target*"))
        fx = Fixture(self.ws, CRATES, ["-p", "sapp"])
        fx.rustc_build()
        self.obj, self.patch, self.bin = (os.path.join(work, f"{name}.{n}") for n in ("o", "patch.o", "bin"))
        link = linkline.with_output(linkline.ahead(list(fx.link_args), [self.patch, self.obj], RA_JIT), self.bin)
        self.link = linkline.join(link)
        self.symbols = [x for rlib in fx.rlibs + sysroot_rlibs() for x in ("--symbols", rlib)]
        self.reference = os.path.join(work, f"{name}.reference")
        self.value = 0
        self.outputs = {}

    def set(self, value):
        """Makes the program print what it prints with `value` added to an area; 0 is the workspace as it came."""
        path = os.path.join(self.ws, MID)
        text = open(path).read()
        old = AREA + (f" + {self.value}" if self.value else "")
        assert old in text, old
        open(path, "w").write(text.replace(old, AREA + (f" + {value}" if value else ""), 1))
        self.value = value

    def expected(self):
        """What cargo's build of the checkout as it is now prints."""
        if self.value not in self.outputs:
            cargo = subprocess.run(["cargo", "build", "-q", "-p", "sapp", *TARGET_ARGS], cwd=self.ws,
                                   capture_output=True, text=True,
                                   env={**os.environ, "CARGO_TARGET_DIR": self.reference})
            assert cargo.returncode == 0, cargo.stderr[-2000:]
            self.outputs[self.value] = run(built(self.reference, "sapp"))
        return self.outputs[self.value]

    def check(self, daemon, what):
        """Asks the daemon for the program and compares what it prints with cargo's; returns the reply."""
        reply = daemon.sync()
        got, want = run(self.bin), self.expected()
        if got != want:
            raise SystemExit(f"FAIL {what}: the program prints\n{got}\nand cargo's\n{want}\n"
                             f"reply: {reply}\nlog: {daemon.log_path}")
        return reply


def run(path):
    try:
        ran = subprocess.run([*RUNNER, path], capture_output=True, text=True, timeout=60)
    except Exception as error:
        return f"did not run: {error}"
    return ran.stdout + (f"[exit {ran.returncode}]" if ran.returncode else "")


class Daemon:
    """A daemon on a checkout, with `store` as its store."""

    def __init__(self, checkout, store, name="daemon", env=None, wait=True):
        self.checkout, self.store = checkout, store
        self.socket = os.path.join(checkout.work, f"{name}.sock")
        self.log_path = os.path.join(checkout.work, f"{name}.log")
        self.seen = os.path.getsize(self.log_path) if os.path.exists(self.log_path) else 0
        out = open(self.log_path, "a")
        c = checkout
        self.process = subprocess.Popen(
            [RA_JIT, c.ws, *TARGET_ARGS, *[x for k in CRATES for x in ("--crate", k)], "--out", c.obj,
             "--socket", self.socket, "--patch", c.patch, "--link-cmd", c.link, "--binary", c.bin, *c.symbols],
            stderr=out, stdout=out, env={**os.environ, **(env or {}), "RA_JIT_CACHE": store})
        if wait:
            self.wait()

    def log(self):
        """What this daemon has logged, without what the ones before it in the same file did."""
        with open(self.log_path, errors="replace") as log:
            log.seek(self.seen)
            return log.read()

    def started(self):
        return "watching" in self.log()

    def wait(self):
        while not self.started():
            if self.process.poll() is not None:
                raise SystemExit(f"the daemon did not start: {self.log_path}")
            time.sleep(0.02)

    def asking(self):
        """Asks for the program without waiting for the reply; returns the client."""
        c = self.checkout
        return subprocess.Popen(
            [RA_JIT, "sync", self.socket, "--project", c.ws, "--crate", "sapp", "--out", c.obj,
             "--patch", c.patch, "--link-cmd", c.link, "--binary", c.bin, *c.symbols],
            stdout=subprocess.PIPE, stderr=subprocess.STDOUT, text=True)

    def sync(self, timeout=180):
        """Asks for the program; returns the daemon's reply."""
        return self.asking().communicate(timeout=timeout)[0].strip()

    def kill(self):
        self.process.kill()
        self.process.wait()

