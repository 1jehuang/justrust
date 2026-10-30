#!/usr/bin/env python3
"""closure_pins.py: the nixpkgs store paths build scripts run with
(crates/ra-jit/src/cargo/closure/pins.txt): a C toolchain, a small shell userland and the system
libraries of the per-crate table, per Linux host, from one pinned nixpkgs revision.

    closure_pins.py                    say whether pins.txt is what its revision evaluates to
    closure_pins.py --write            write pins.txt for the revision it names
    closure_pins.py --write --rev REV  move the pin to nixpkgs commit REV
    closure_pins.py --write --rev latest
                                       move it to the newest commit of nixos-unstable

A tool for people moving the pin, not a part of blaze: it runs `nix eval` on the pinned revision
to name each root's store path, so it runs where nix is, an ix VM. No blaze code path runs nix,
and users need none: blaze reads the paths from pins.txt, CI fetches them from cache.nixos.org
(`ra-jit closure fetch`) into blaze's store, and users take them from blaze's remote cache.

The roots are named by attribute, `openssl.dev`, so that the per-crate table can name them. Each
root's NarHash and NarSize, and the size of its whole closure, are read from cache.nixos.org's
.narinfo files, walking their References: a revision of nixos-unstable is one Hydra built, so
every path is there. A narinfo's signature is not checked here; blaze checks every one it uses.
"""
import concurrent.futures, json, os, subprocess, sys, urllib.request

HERE = os.path.dirname(os.path.abspath(__file__))
ROOT = os.path.dirname(HERE)
PINS = os.path.join(ROOT, "crates/ra-jit/src/cargo/closure/pins.txt")
CACHE = "https://cache.nixos.org"
CHANNEL = "https://channels.nixos.org/nixos-unstable/git-revision"
HOSTS = ["x86_64-linux", "aarch64-linux"]
# The C toolchain (the wrapped clang, so that `CC` works, lld, LLVM's wrapped ar, nm and
# ranlib, a glibc sysroot), pkg-config (wrapped), the userland a build script's shell commands
# need, and the libraries of the per-crate table, each with its headers and pkg-config files
# where it has a `dev` output.
ROOTS = [
    "clang", "lld", "llvmPackages.bintools", "glibc.out", "glibc.dev", "linuxHeaders",
    "pkg-config",
    "bash", "coreutils", "gnumake", "gnused", "gawk", "gnugrep", "diffutils", "findutils",
    "cmake", "git",
    "openssl.out", "openssl.dev", "zlib.out", "zlib.dev", "libgit2.lib", "libgit2.dev",
    "libssh2.out", "libssh2.dev", "curl.out", "curl.dev", "sqlite.out", "sqlite.dev",
    "zstd.out", "zstd.dev", "xz.out", "xz.dev", "bzip2.out", "bzip2.dev",
    "oniguruma.lib", "oniguruma.dev", "libxkbcommon.out", "libxkbcommon.dev",
    "fontconfig.lib", "fontconfig.dev", "freetype.out", "freetype.dev",
    "alsa-lib.out", "alsa-lib.dev", "wayland.out", "wayland.dev",
    "libx11.out", "libx11.dev", "dbus.lib", "dbus.dev", "protobuf.out",
]


def get(url):
    with urllib.request.urlopen(url, timeout=60) as answer:
        return answer.read().decode()


def pinned_rev():
    for line in open(PINS) if os.path.exists(PINS) else []:
        if line.startswith("nixpkgs "):
            return line.split()[1]
    sys.exit(f"{os.path.relpath(PINS, ROOT)} names no nixpkgs revision: pass --rev")


def out_paths(rev, host):
    """Each root's store path, by `nix eval` of the revision for `host`."""
    fields = []
    for attr in ROOTS:
        path = ".".join(f'"{part}"' for part in attr.split("."))
        fields.append(f'"{attr}" = p.{path}.outPath;')
    expr = (f'let p = (builtins.getFlake "github:NixOS/nixpkgs/{rev}").legacyPackages.{host}; '
            f'in {{ {" ".join(fields)} }}')
    said = subprocess.run(["nix", "eval", "--json", "--expr", expr], check=True,
                          capture_output=True, text=True)
    return json.loads(said.stdout)


def narinfo(path):
    base = path.removeprefix("/nix/store/")
    fields = {}
    for line in get(f"{CACHE}/{base[:32]}.narinfo").splitlines():
        name, _, value = line.partition(": ")
        fields[name] = value
    if fields.get("StorePath") != path:
        sys.exit(f"cache.nixos.org's narinfo for {path} is of {fields.get('StorePath')}")
    refs = [f"/nix/store/{it}" for it in fields.get("References", "").split()]
    return fields["NarHash"], int(fields["NarSize"]), refs


def infos(roots):
    """The narinfo of every path in the closures of `roots`."""
    known, todo = {}, set(roots)
    with concurrent.futures.ThreadPoolExecutor(32) as pool:
        while todo:
            batch = sorted(todo)
            for path, info in zip(batch, pool.map(narinfo, batch)):
                known[path] = info
            todo = {ref for path in batch for ref in known[path][2]} - known.keys()
    return known


def closure(path, known):
    seen, todo = set(), [path]
    while todo:
        at = todo.pop()
        if at not in seen:
            seen.add(at)
            todo.extend(known[at][2])
    return seen


def pins(rev):
    lines = [
        "# The nixpkgs store paths build scripts run with: written by harness/closure_pins.py",
        "# --write, which names each root by `nix eval` of the revision and reads its NarHash,",
        "# NarSize and closure from cache.nixos.org. nixpkgs <commit>; then per host",
        "# <host> <attribute> <store path> <NarHash> <NarSize> <closure bytes> <closure paths>,",
        "# and total <host> <bytes> <paths> of the closure of all its roots.",
        f"nixpkgs {rev}",
    ]
    for host in HOSTS:
        paths = out_paths(rev, host)
        known = infos(paths.values())
        everything = set()
        for attr in ROOTS:
            path = paths[attr]
            nar_hash, nar_size, _ = known[path]
            whole = closure(path, known)
            everything |= whole
            size = sum(known[it][1] for it in whole)
            lines.append(f"{host} {attr} {path} {nar_hash} {nar_size} {size} {len(whole)}")
        size = sum(known[it][1] for it in everything)
        lines.append(f"total {host} {size} {len(everything)}")
    return "\n".join(lines) + "\n"


def main():
    rev = pinned_rev() if "--rev" not in sys.argv else sys.argv[sys.argv.index("--rev") + 1]
    if rev == "latest":
        rev = get(CHANNEL).strip()
    written = pins(rev)
    if "--write" in sys.argv:
        os.makedirs(os.path.dirname(PINS), exist_ok=True)
        with open(PINS, "w") as out:
            out.write(written)
        print(f"wrote {os.path.relpath(PINS, ROOT)} for nixpkgs {rev}")
        return
    have = open(PINS).read() if os.path.exists(PINS) else ""
    if have != written:
        sys.exit(f"{os.path.relpath(PINS, ROOT)} is not what nixpkgs {rev} evaluates to: "
                 "closure_pins.py --write writes it")
    print(f"{os.path.relpath(PINS, ROOT)} is nixpkgs {rev}")


if __name__ == "__main__":
    main()
