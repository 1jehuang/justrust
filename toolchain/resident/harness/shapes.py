#!/usr/bin/env python3
"""How much compiled code is the same function at different types.

usage: shapes.py [crate[,crate...]]

Builds the crates of bench2 once with RA_JIT_SHAPES set and folds the functions the way a linker's
identical-code folding does: two functions are the same when their bytes and relocations are, and
what they refer to is the same after folding. Reports how many functions, bytes and compile
seconds are left, by kind of function and for the derives.
"""
import os, subprocess, sys
from collections import Counter, defaultdict

HERE = os.path.dirname(os.path.abspath(__file__))
os.environ.setdefault("RUSTUP_TOOLCHAIN", open(os.path.join(HERE, "toolchain")).read().strip())
from difftest import Fixture, RA_JIT, sysroot_rlibs


def load(path):
    # Each round of the build writes every function again, and only the first compiles it.
    origins, codes, times = {}, {}, {}
    for line in open(path):
        tag, *rest = line.rstrip("\n").split("\t")
        if tag == "R":
            # A round starts over, and only the round that compiles a function takes time.
            times, origins, codes = {o["key"]: o["micros"] for o in origins.values()} | times, {}, {}
            continue
        symbol, *rest = rest
        if tag == "O":
            kind, origin, name, generic, derive, micros, key = rest
            micros = max(int(micros), times.get(key, 0))
            origins[symbol] = dict(kind=kind, origin=origin, name=name, generic=generic == "1",
                                   derive=derive == "1", micros=micros, key=key)
        else:
            size, code, targets = rest
            codes[symbol] = (int(size), code, targets.split())
    kept = {o["key"] for o in origins.values()}
    dropped = [t for key, t in times.items() if key not in kept]
    print(f"{len(dropped)} functions compiled in an earlier round and left to rustc in the last, "
          f"{sum(dropped) / 1e6:.2f} s")
    return origins, codes


def fold(codes, by_shape):
    """Classes of identical functions. `by_shape` ignores what the relocations point to."""
    cls = {s: code for s, (_, code, _) in codes.items()}
    if by_shape:
        return cls
    count = len(set(cls.values()))
    while True:
        # A target outside the build, or data, stands for itself.
        cls = {s: hash((code, tuple(cls.get(t, t) if t != "self" else "self" for t in targets)))
               for s, (_, code, targets) in codes.items()}
        now = len(set(cls.values()))
        if now == count:
            return cls
        count = now


def main():
    crates = (sys.argv[1] if len(sys.argv) > 1 else "regex_automata").split(",")
    fx = Fixture(os.path.join(HERE, "bench2"), crates, ["-p", "bench2"])
    fx.rustc_build()
    log = os.path.join(fx.work, "shapes.tsv")
    symbols = [x for rlib in fx.rlibs + sysroot_rlibs() for x in ("--symbols", rlib)]
    args = [x for name in crates for x in ("--crate", name)]
    subprocess.run([RA_JIT, fx.ws, *args, "--out", os.path.join(fx.work, "ra-jit.o"), *symbols],
                   env={**os.environ, "RA_JIT_SHAPES": log}, check=True, capture_output=True)
    origins, codes = load(log)
    total = len(codes)
    size = sum(c[0] for c in codes.values())
    secs = sum(o["micros"] for o in origins.values()) / 1e6
    print(f"{total} functions, {size / 1e6:.2f} MB of code, {secs:.2f} s in the compile queries\n")

    def left(cls, keep=lambda s: True):
        seen, n, b, t = set(), 0, 0, 0
        for s in codes:
            if keep(s) and cls[s] not in seen:
                seen.add(cls[s])
                n, b, t = n + 1, b + codes[s][0], t + origins.get(s, {}).get("micros", 0)
        return n, b, t / 1e6

    exact, shape = fold(codes, False), fold(codes, True)
    print("| what | functions | code | compile |\n|---|---:|---:|---:|")
    print(f"| today | {total} | {size / 1e6:.2f} MB | {secs:.2f} s |")
    for name, cls in (("identical after folding", exact), ("identical but for what they refer to", shape)):
        n, b, t = left(cls)
        print(f"| {name} | {n} ({n / total:.0%}) | {b / 1e6:.2f} MB ({b / size:.0%}) | {t:.2f} s ({t / secs:.0%}) |")

    print("\n| kind | functions | code | compile | after folding | by shape |\n|---|---:|---:|---:|---:|---:|")
    kinds = defaultdict(list)
    for s in codes:
        o = origins.get(s)
        kind = "?" if o is None else "derive" if o["derive"] else \
            ("generic " if o["generic"] else "") + o["kind"]
        kinds[kind].append(s)
    for kind, syms in sorted(kinds.items(), key=lambda kv: -len(kv[1])):
        members = set(syms)
        b = sum(codes[s][0] for s in syms)
        t = sum(origins.get(s, {}).get("micros", 0) for s in syms) / 1e6
        e, sh = (len({cls[s] for s in syms}) for cls in (exact, shape))
        print(f"| {kind} | {len(syms)} | {b / 1e3:.0f} kB | {t:.2f} s | {e} | {sh} |")

    print("\n| derived method | functions | code | compile | after folding | by shape |\n|---|---:|---:|---:|---:|---:|")
    derives = defaultdict(list)
    for s in kinds["derive"]:
        derives[origins[s]["name"]].append(s)
    for name, syms in sorted(derives.items(), key=lambda kv: -len(kv[1])):
        b = sum(codes[s][0] for s in syms)
        t = sum(origins[s]["micros"] for s in syms) / 1e6
        e, sh = (len({cls[s] for s in syms}) for cls in (exact, shape))
        print(f"| {name} | {len(syms)} | {b / 1e3:.0f} kB | {t:.2f} s | {e} | {sh} |")

    print("\nGeneric functions with the most instances (instances, after folding, by shape, kB):")
    by_origin = defaultdict(list)
    for s in codes:
        o = origins.get(s)
        if o and o["generic"]:
            by_origin[(o["origin"], o["name"])].append(s)
    print(f"{len(by_origin)} generic origins, {sum(map(len, by_origin.values()))} instances")
    for (origin, name), syms in sorted(by_origin.items(), key=lambda kv: -len(kv[1]))[:25]:
        e, sh = (len({cls[s] for s in syms}) for cls in (exact, shape))
        print(f"  {len(syms):5} {e:5} {sh:5} {sum(codes[s][0] for s in syms) / 1e3:7.1f}  {name or origin}")
    sizes = Counter(len(v) for v in by_origin.values())
    print("instances per origin:", sorted(sizes.items())[:12])
    print("log:", log)


if __name__ == "__main__":
    main()
