"""Compares the size and alignment rust-analyzer computes for types against rustc's, and for
the futures of `async fn`s where each variant keeps what it keeps.

rustc's side is `-Zprint-type-sizes`; ra-jit's is the `RA_JIT_LAYOUTS` log of every type its
backend laid out. The two print types differently, so both are reduced to a common spelling
and only types that reduce to one unambiguous rustc layout are compared. A mismatch here is a
miscompile found without running anything.
"""
import os, re, subprocess, time

DEFAULTS = (", Global", ", RandomState", ", BuildHasherDefault<DefaultHasher>")


def spell(name):
    name = re.sub(r"\b(?:[A-Za-z_]\w*::)+", "", name)
    name = re.sub(r"'(?:\{region error\}|static|_|[a-z]\w*) ?", "", name)
    name = re.sub(r"ValueConst \{ ty: \w+, value: ValTreeInterned\(Leaf\(0x([0-9a-f]+)\)\) \}",
                  lambda m: str(int(m[1], 16)), name)
    for default in DEFAULTS:
        name = name.replace(default, "")
    return name.replace(" ", "")


def rustc_layouts(workspace, crate):
    # Crate names have underscores where the package may have hyphens.
    for package in (crate, crate.replace("_", "-")):
        # cargo runs rustc again only for arguments it has not seen, and a crate it finds fresh
        # prints nothing.
        again = f"layouts_{time.time_ns()}"
        out = subprocess.run(["cargo", "rustc", "-p", package, "--lib", "--", "-Zprint-type-sizes",
                              "--cfg", again],
                             cwd=workspace, capture_output=True, text=True,
                             # The one unstable flag the harness uses, on the stable pin.
                             env={**os.environ, "RUSTC_BOOTSTRAP": "1", "CARGO_INCREMENTAL": "0"})
        if "did not match any packages" not in out.stderr:
            break
    if out.returncode != 0:
        raise SystemExit(f"cargo failed:\n{out.stderr[-3000:]}")
    layouts = {}
    for m in re.finditer(r"print-type-size type: `(.*)`: (\d+) bytes, alignment: (\d+) bytes", out.stdout):
        layouts.setdefault(spell(m[1]), set()).add((int(m[2]), int(m[3])))
    for name, layout in rustc_coroutines(out.stdout):
        layouts.setdefault(name, set()).add(layout)
    return layouts


def rustc_coroutines(text):
    """Each `async fn` future `-Zprint-type-sizes` prints, as (name, (size, align, variants)) with
    the offsets of what each variant holds. rustc prints a variant's fields by offset, counting
    from just after a one-byte discriminant and naming the offset of one that comes before it."""
    found, current, cursor, discriminant = [], None, 0, 0
    for line in text.splitlines():
        if not line.startswith("print-type-size"):
            continue
        body = line[len("print-type-size"):].strip()
        m = re.match(r"type: `(\{async fn body of .*\})`: (\d+) bytes, alignment: (\d+) bytes", body)
        if m:
            current = [spell(m[1]), int(m[2]), int(m[3]), []]
            found.append(current)
            discriminant = 0
            continue
        if body.startswith("type: "):
            current = None
            continue
        if current is None:
            continue
        if m := re.match(r"discriminant: (\d+) bytes", body):
            discriminant = int(m[1])
        elif body.startswith("variant `"):
            current[3].append([])
            cursor = discriminant
        elif m := re.match(r"padding: (\d+) bytes", body):
            cursor += int(m[1])
        elif m := re.match(r"(?:upvar|local) `.*`: (\d+) bytes(?:, offset: (\d+) bytes)?", body):
            offset = int(m[2]) if m[2] else cursor
            current[3][-1].append(offset)
            cursor = offset + int(m[1])
    return [(name + "@coroutine", (size, align, tuple(sorted(tuple(sorted(v)) for v in variants))))
            for name, size, align, variants in found]


def ra_jit_layouts(path):
    layouts = {}
    for line in open(path):
        name, size, align = line.rstrip("\n").rsplit("\t", 2)
        if " @coroutine " in name:
            name, variants = name.split(" @coroutine ")
            variants = tuple(sorted(tuple(sorted(int(it) for it in v.split(",") if it))
                                    for v in variants.split(";")))
            layouts.setdefault(spell(name) + "@coroutine", set()).add((int(size), int(align), variants))
            continue
        if any(s in name for s in ("Binder", "Closure(", "Re", "Param", "Alias", "{", "?")):
            continue
        layouts.setdefault(spell(name), set()).add((int(size), int(align)))
    return layouts


def diff(workspace, crates, log):
    theirs = {}
    for crate in crates:
        for name, sizes in rustc_layouts(workspace, crate).items():
            theirs.setdefault(name, set()).update(sizes)
    ours = ra_jit_layouts(log)
    compared, wrong = 0, []
    for name, sizes in sorted(ours.items()):
        want = theirs.get(name)
        if not want or len(want) != 1 or len(sizes) != 1:
            continue
        compared += 1
        if sizes != want:
            wrong.append((name, next(iter(want)), next(iter(sizes))))
    return compared, wrong
