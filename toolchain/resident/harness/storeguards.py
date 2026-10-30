#!/usr/bin/env python3
"""storeguards.py: whose the store is, and what of a checkout is in it.

A daemon fills a store and ends. Then:

  - the directory has to be the user's alone (0700) and every file in it too (0600);
  - no file of the store, and no file of a remote the store was pushed to, may have the
    path of the checkout in it: what one checkout kept is found by another, and read by
    whoever reads the remote;
  - a pack that other users can write to has to be removed, said so, and the program
    built from the rest has to print what cargo's does;
  - a store directory that other users can write to has to be refused by `ra-jit cache`
    with the reason, and a daemon on it has to say it is not used, build the right
    program, and neither read nor write anything in it;
  - `ra-jit cache pull` from a remote whose lists nobody signed has to be refused, with
    nothing fetched, unless RA_JIT_REMOTE_UNSIGNED=1; `ra-jit cache push` asks for nothing.

  - a crate cargo fetched from a registry is kept under `/cargo`, not under the cargo home
    of whoever built it: a second user with another cargo home has to find what the first
    kept, the program has to print what that user's cargo's prints (the place of the file
    under their own cargo home), and neither home may be in the store.

    python3 storeguards.py
"""
import os, re, shutil, stat, subprocess, sys

HERE = os.path.dirname(os.path.abspath(__file__))
sys.path.insert(0, HERE)
from storelib import Checkout, Daemon, cache, files, run, scratch, verify
from difftest import Fixture
import packs

SEALS = {"RA_JIT_CACHE_SEAL_AT": "1", "RA_JIT_CACHE_MAX": "off"}
OTHERS_WRITE = "written to by other users"


def fail(what):
    raise SystemExit(f"FAIL: {what}")


def mode(path):
    return stat.S_IMODE(os.stat(path).st_mode)


def filled(checkout, store, name):
    """Has a daemon build the program with `store`; returns what the daemon logged."""
    daemon = Daemon(checkout, store, name=name, env=SEALS)
    checkout.check(daemon, name)
    daemon.kill()
    return daemon.log()


def the_users_alone(store):
    if mode(store) != 0o700:
        fail(f"the store is {mode(store):o}")
    open_to = [(name, mode(os.path.join(store, name))) for name in os.listdir(store)]
    if not open_to or any(it != 0o600 for _, it in open_to):
        fail(f"files of the store are not 0600: {open_to}")
    print(f"  modes: the store 0700, its {len(open_to)} files 0600")


def held_in(path):
    """The bytes of a file as a store would hold them: a remote sends its packs through zstd,
    a sealed pack keeps its values in zstd blocks, and a path that is looked for is not found
    in what is compressed."""
    data = open(path, "rb").read()
    if path.endswith(".zst"):
        unpacked = subprocess.run(["zstd", "-d", "-c", "-q"], input=data, capture_output=True)
        if unpacked.returncode != 0:
            fail(f"{path} is not zstd: {unpacked.stderr.decode()[-200:]}")
        data = unpacked.stdout
    if data.startswith(packs.MAGIC):
        if packs.index(data) is None:
            fail(f"{path} is not laid out as a sealed pack")
        return b"".join(packs.blocks(data))
    return data


def names_no_checkout(checkout, directory, what):
    """Nothing below `directory` has the checkout's place, or the home of the user, in it."""
    places = [checkout.work.encode(), os.path.expanduser("~").encode()]
    read = 0
    for at, _, names in os.walk(directory):
        for name in names:
            data = held_in(os.path.join(at, name))
            read += len(data)
            for place in places:
                if place in data:
                    fail(f"{what}: {name} has {place.decode()} in it")
    if read < 10_000:
        fail(f"{what}: only {read} bytes to look through")
    print(f"  paths: none of the checkout or the home in the {read} bytes of {what}")


def writable_pack(checkout, store):
    pack = files(store, ".pack")[0]
    os.chmod(pack, 0o666)
    whole, _, out = verify(store)
    if whole or OTHERS_WRITE not in out:
        fail(f"verify did not tell of the pack others can write to: {out}")
    log = filled(checkout, store, "writable-pack")
    if OTHERS_WRITE not in log or os.path.basename(pack) not in log:
        fail(f"the daemon did not say it removed the pack: {log[-400:]}")
    whole, _, out = verify(store)
    if not whole or any(mode(it) != 0o600 for it in files(store, ".pack")):
        fail(f"the store after the pack was removed: {out}")
    print("  a pack others can write to: told of by verify, removed by the daemon, the program right")


def writable_store(checkout, store):
    before = sorted(os.listdir(store))
    for open_to in (0o770, 0o707):
        os.chmod(store, open_to)
        for command in (["stats"], ["verify"], ["gc"]):
            status, out = cache(store, *command)
            if status == 0 or OTHERS_WRITE not in out:
                fail(f"cache {command[0]} on a {open_to:o} store: {out}")
        log = filled(checkout, store, f"writable-{open_to:o}")
        if "store: not used" not in log or OTHERS_WRITE not in log:
            fail(f"the daemon did not say the {open_to:o} store is not used: {log[-400:]}")
        if sorted(os.listdir(store)) != before:
            fail(f"the {open_to:o} store was changed: {sorted(os.listdir(store))}")
    os.chmod(store, 0o700)
    print("  a store others can write to: refused by cache stats, verify and gc; not used and "
          "not changed by the daemon, the program right")


def unsigned(checkout, store, work):
    remote, second = os.path.join(work, "remote"), os.path.join(work, "second")
    nothing_asked = {"RA_JIT_REMOTE_UNSIGNED": ""}
    status, out = cache(store, "push", remote, env=nothing_asked)
    if status != 0:
        fail(f"push: {out}")
    names_no_checkout(checkout, remote, "the remote")
    for value in ("", "0", "true"):
        status, out = cache(second, "pull", remote, env={"RA_JIT_REMOTE_UNSIGNED": value})
        if status == 0 or "BLAZE_REMOTE_UNSIGNED=1" not in out or files(second, ".pack"):
            fail(f"pull with RA_JIT_REMOTE_UNSIGNED={value!r} was not refused: {out}")
    status, out = cache(second, "pull", remote, env={"RA_JIT_REMOTE_UNSIGNED": "1"})
    whole, _, verified = verify(second)
    if status != 0 or not whole or len(files(second, ".pack")) != len(files(store, ".pack")):
        fail(f"pull with RA_JIT_REMOTE_UNSIGNED=1: {out}\n{verified}")
    the_users_alone(second)
    print(f"  unsigned lists: push asks for nothing; pull refused unless RA_JIT_REMOTE_UNSIGNED=1, "
          f"then {len(files(second, '.pack'))} packs fetched")


def write(path, text):
    os.makedirs(os.path.dirname(path), exist_ok=True)
    open(path, "w").write(text)


def fetched_workspace(top):
    """A workspace whose program is built on a crate of a registry of this machine's own: a
    local-registry the `local` registry's source is replaced with, as blaze reads no git index."""
    registry, ws = os.path.join(top, "registry"), os.path.join(top, "ws")
    write(f"{top}/stage/dep-0.1.0/Cargo.toml", '[package]\nname = "dep"\nversion = "0.1.0"\nedition = "2021"\n')
    write(f"{top}/stage/dep-0.1.0/src/lib.rs",
          'pub fn place() -> &\'static str {\n    file!()\n}\n\n'
          'pub fn third(of: &[u32]) -> u32 {\n    of[2]\n}\n\n'
          'pub fn first<T: Copy>(of: &[T]) -> T {\n    of[0]\n}\n')
    os.makedirs(registry)
    crate = f"{registry}/dep-0.1.0.crate"
    subprocess.run(["tar", "-czf", crate, "-C", f"{top}/stage", "dep-0.1.0"], check=True)
    import hashlib
    cksum = hashlib.sha256(open(crate, "rb").read()).hexdigest()
    write(f"{registry}/index/3/d/dep",
          '{"name":"dep","vers":"0.1.0","deps":[],"cksum":"%s","features":{},"yanked":false}\n' % cksum)
    index = f"file://{registry}/index"
    write(f"{ws}/.cargo/config.toml",
          f'[registries.local]\nindex = "{index}"\n\n[source.local-index]\nregistry = "{index}"\n'
          f'replace-with = "local-files"\n\n[source.local-files]\nlocal-registry = "{registry}"\n')
    write(f"{ws}/Cargo.toml", '[workspace]\nmembers = ["app"]\nresolver = "2"\n')
    write(f"{ws}/app/Cargo.toml", '[package]\nname = "app"\nversion = "0.1.0"\nedition = "2021"\n\n'
          '[dependencies]\ndep = { version = "0.1", registry = "local" }\n')
    write(f"{ws}/app/src/main.rs",
          'fn main() {\n    println!("{} {}", dep::place(), dep::first(&[4u8]));\n'
          '    let short = std::env::args().map(|it| it.len() as u32).collect::<Vec<_>>();\n'
          '    let caught = std::panic::catch_unwind(|| dep::third(&short)).unwrap_err();\n'
          '    println!("{}", caught.downcast_ref::<String>().map_or("?", |it| it));\n}\n')
    return ws


def built_with_home(ws, home, store):
    """Builds the program with `home` as cargo's; returns what ra-jit said of the store."""
    were = {name: os.environ.get(name) for name in ("CARGO_HOME", "RA_JIT_CACHE", "PATH")}
    cargo_bin = os.path.dirname(shutil.which("cargo"))
    os.environ.update(CARGO_HOME=home, RA_JIT_CACHE=store, PATH=cargo_bin + os.pathsep + os.environ["PATH"],
                      **SEALS)
    try:
        fx = Fixture(ws, ["dep", "app"], ["-p", "app"])
        fx.rustc_build()
        fx.ra_jit_build()
    finally:
        for name, value in were.items():
            os.environ.pop(name, None) if value is None else os.environ.__setitem__(name, value)
    want, got = run(fx.reference), run(fx.binary)
    if home not in want or got != want:
        fail(f"with {home} as cargo's home the program prints\n{got}\nand cargo's\n{want}")
    said = open(os.path.join(fx.work, "app.stderr")).read()
    counted = re.search(r"entries: (\d+) found, (\d+) added", said)
    if not counted:
        fail(f"ra-jit said nothing of the entries of the crate of the registry:\n{said[-2000:]}")
    return int(counted[1]), int(counted[2])


def another_cargo_home(work):
    top = os.path.join(work, "fetched")
    ws, store = fetched_workspace(top), os.path.join(top, "store")
    homes = [os.path.join(top, name, "cargo") for name in ("one", "another-user")]
    found, added = built_with_home(ws, homes[0], store)
    if found or not added:
        fail(f"the first build found {found} entries and added {added}")
    found_again, added_again = built_with_home(ws, homes[1], store)
    if found_again != added or added_again:
        fail(f"under another cargo home {found_again} of {added} entries were found and {added_again} added")
    remote = os.path.join(top, "remote")
    status, out = cache(store, "push", remote)
    if status != 0:
        fail(f"push: {out}")
    stored = b""
    for what, directory in (("the store", store), ("the remote", remote)):
        held = b"".join(held_in(os.path.join(at, name))
                        for at, _, names in os.walk(directory) for name in sorted(names))
        for home in homes:
            if home.encode() in held:
                fail(f"{what} has the cargo home {home} in it")
        if b"/cargo/registry/src/" not in held:
            fail(f"{what} has no file of the registry in it by the name every machine has for it")
        stored = stored or held
    print(f"  cargo home: {added} entries, some of a crate of a registry, kept under one cargo home, all found "
          f"under another; neither home in the {len(stored)} bytes of the store or in the remote it was pushed to; the program prints "
          f"the place under the home it was built with")


def main():
    if os.name != "posix":
        raise SystemExit("storeguards.py: the modes of files are checked on Unix only")
    work = scratch("ra-jit-storeguards-")
    checkout = Checkout(work)
    store = os.path.join(work, "store")
    filled(checkout, store, "fill")
    checkout.set(1)
    filled(checkout, store, "fill")
    the_users_alone(store)
    names_no_checkout(checkout, store, "the store")
    unsigned(checkout, store, work)
    another_cargo_home(work)
    writable_pack(checkout, store)
    writable_store(checkout, store)
    shutil.rmtree(work, ignore_errors=True)
    print("storeguards: all right")

if __name__ == "__main__":
    main()
