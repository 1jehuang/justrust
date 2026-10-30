"""linkline.py: a link line with ra-jit's objects ahead of what rustc built of the same crates.

Both define the same names, and ra-jit's have to be the ones that stay. An ELF linker keeps the
first definition when told to, and so does a linker of the `link.exe` kind. Apple's linker has no flag for it, so there rustc's objects and
archives are replaced by copies whose definitions are weak (`ra-jit weaken`): a weak definition
gives way to one that is not, and to a weak one before it on the line. On ELF they are replaced
by the copies the daemon links (`ra-jit weaken --shared`), whose statics private to an object
ra-jit's code can refer to.
"""
import os, shlex, subprocess, sys

MACHO = sys.platform == "darwin"
WINDOWS = os.name == "nt"
THEIRS = (".rlib", ".rcgu.o")


def first_of_rustc(link):
    """Where rustc's own objects start in a link line."""
    return next(i for i, a in enumerate(link) if a.endswith(THEIRS))


def msvc(link):
    """Whether the line is one of a linker of the `link.exe` kind."""
    return "/NOLOGO" in link


def output_of(link):
    """The file the line links."""
    if msvc(link):
        return next(a for a in link if a.upper().startswith("/OUT:"))[len("/OUT:"):]
    return link[link.index("-o") + 1]


def with_output(link, path):
    """The line with the linked file written to `path`."""
    if msvc(link):
        # A library's import library goes with it, not over the one of the line's own link.
        moved = {"/OUT:": f"/OUT:{path}", "/IMPLIB:": f"/IMPLIB:{path}.lib"}
        return [next((to for flag, to in moved.items() if a.upper().startswith(flag)), a) for a in link]
    link = list(link)
    link[link.index("-o") + 1] = path
    return link


def join(link):
    """The line as `--link-cmd` takes it: what the platform's shell would run."""
    return subprocess.list2cmdline(link) if WINDOWS else shlex.join(link)


def wasm(link):
    """Whether the line is one of `wasm-ld`, which rustc runs itself rather than through `cc`."""
    return "wasm" in link[:3]


def allowing_duplicates(wasm):
    """What an ELF or wasm link line gets after ra-jit's objects. A wasm module that edits are
    spliced into has to keep its symbol table, as it names nothing but its exports otherwise,
    and keeps what nothing calls yet, as an edit may. `RA_JIT_LINK_ARGS` is what it gets
    instead of those."""
    spliced = "--emit-relocs --no-gc-sections" if wasm and os.environ.get("RA_JIT_SPLICE") else ""
    allow = "--allow-multiple-definition" if wasm else "-Wl,--allow-multiple-definition"
    return [allow, *shlex.split(os.environ.get("RA_JIT_LINK_ARGS", spliced))]


def ahead(link, objects, ra_jit):
    """`link` with `objects` before rustc's, and the linker told that the first definition stays."""
    link = list(link)
    first = first_of_rustc(link)
    if msvc(link):
        link[first:first] = [*objects, "/FORCE:MULTIPLE", "/IGNORE:4006"]
        return link
    if wasm(link):
        link[first:first] = [*objects, *allowing_duplicates(True)]
        return link
    # The library of the toolchain defines nothing a workspace crate does.
    sysroot = subprocess.check_output(["rustc", "--print", "sysroot"], text=True).strip()
    theirs = [i for i, a in enumerate(link) if a.endswith(THEIRS) and not a.startswith(sysroot)]
    # On ELF, copies whose statics private to an object are ones ra-jit's code can refer to,
    # as the daemon links them (`weaken::shared`); ra-jit takes rustc's copy to be linkable.
    weaken = [ra_jit, "weaken"] if MACHO else [ra_jit, "weaken", "--shared"]
    copies = subprocess.check_output([*weaken, *(link[i] for i in theirs)], text=True).splitlines()
    for i, copy in zip(theirs, copies):
        link[i] = copy
    if not MACHO:
        link[first:first] = [*objects, *allowing_duplicates(False)]
        return link
    # A splice adds two segments to the header, and the unwind table behind it, which the
    # linker leaves no room for. It reads and writes the loader's fixups as tables, which
    # the linker gives as chains for a deployment target of macOS 12 or later.
    link[first:first] = [*objects, "-Wl,-headerpad,0x100000", "-Wl,-no_fixup_chains"]
    return link
