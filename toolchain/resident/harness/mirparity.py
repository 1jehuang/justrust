#!/usr/bin/env python3
"""How close the MIR ra-jit compiles is to rustc's, function by function.

usage: mirparity.py [<workspace>] [--crate <package>] [--level 0|1|2] [--rustc-level 1|2]
                    [--show N] [--json FILE]

Both sides are written as rustc's `-Zunpretty=mir` writes MIR: rustc's by rustc itself,
ra-jit's by `ra-jit mir`, which prints the bodies its code generator compiles at the MIR
optimization level `--level` (`RA_JIT_MIR_OPT`). rustc's are its optimized MIR, which is what
it generates code from: at `-Copt-level=0` that is its `-Zmir-opt-level=1`, the level a debug
build gets, and with `--rustc-level 2` the MIR of `-Copt-level=2`, without inlining
(`-Zinline-mir=no`), which ra-jit leaves to the code generator, and with the debug
assertions and overflow checks of the debug build ra-jit compiles. rustc is asked for its MIR
without the checks of pointers a debug build has (`-Zub-checks=no`) unless `--ub-checks`:
ra-jit has none of them yet, and they would hide everything else in a function that has one.

A function of one is paired with the function of the other of the same path and number of
parameters. Each pair is then compared at four strictnesses, each implying the one before:

  shape      the blocks and the kinds of their terminators, in the same order
  counts     also the number of statements in each block
  identical  the same statements and terminators, up to the numbering of blocks and locals,
             the spelling of paths and the values of constants that are not scalars
  typed      also the same type for every local

and the share of pairs that pass each is printed. `identical` is the match rate. The
statements one side has in a function and the other has not are counted over all pairs, and
the most frequent are listed with an example function, which is what to work on next.

Exit status is 0 unless a side fails to build or to print. RA_JIT_BIN names the ra-jit binary.
"""
import argparse, collections, json, os, re, subprocess, sys

HERE = os.path.dirname(os.path.abspath(__file__))
ROOT = os.path.dirname(HERE)
TOOLCHAIN = open(os.path.join(HERE, "toolchain")).read().strip()
RA_JIT = os.environ.get("RA_JIT_BIN") or os.path.join(ROOT, "target/release/ra-jit")


def rustc_mir(workspace, package, level, ub_checks):
    env = dict(os.environ, RUSTC_BOOTSTRAP="1", RUSTUP_TOOLCHAIN=TOOLCHAIN, CARGO_INCREMENTAL="0")
    env.pop("RUSTFLAGS", None)
    flags = ["-Zunpretty=mir"]
    if not ub_checks:
        flags.append("-Zub-checks=no")
    if level >= 2:
        flags += ["-Copt-level=2", "-Zinline-mir=no", "-Cdebug-assertions=on", "-Coverflow-checks=on"]
    cmd = ["cargo", "rustc", "-q", "-p", package, "--lib", "--"] + flags
    out = subprocess.run(cmd, cwd=workspace, env=env, capture_output=True, text=True)
    if out.returncode != 0:
        sys.exit(f"rustc failed:\n{out.stderr[-4000:]}")
    return out.stdout


def ra_jit_mir(workspace, package, level):
    env = dict(os.environ, RUSTUP_TOOLCHAIN=TOOLCHAIN)
    cmd = [RA_JIT, "mir", workspace, "--crate", package.replace("-", "_"), "--level", str(level), "--time"]
    out = subprocess.run(cmd, env=env, capture_output=True, text=True)
    if out.returncode != 0:
        sys.exit(f"ra-jit mir failed:\n{out.stderr[-4000:]}")
    timing = [line for line in out.stderr.splitlines() if line.startswith("mir: ")]
    return out.stdout, (timing[-1] if timing else "")


# --- Reading the text ------------------------------------------------------------------------

def functions(text):
    """(name, parameter count, body lines) of every `fn` the text has."""
    found, lines, i = [], text.splitlines(), 0
    while i < len(lines):
        line = lines[i]
        if line.startswith("fn "):
            head = line[3:]
            name, params = split_head(head)
            body = []
            i += 1
            while i < len(lines) and lines[i] != "}":
                body.append(lines[i])
                i += 1
            found.append((name, params, body))
        i += 1
    return found


def split_head(head):
    """The path and parameter count of `path(_1: T, _2: U) -> R {`."""
    depth, start = 0, None
    for at, ch in enumerate(head):
        if ch in "<[{":
            depth += 1
        elif ch in ">]}":
            depth -= 1
        elif ch == "(" and depth == 0:
            start = at
            break
    name, rest = head[:start], head[start + 1:]
    depth, params = 0, rest
    for at, ch in enumerate(rest):
        if ch in "([<{":
            depth += 1
        elif ch in ")]>}":
            if depth == 0:
                params = rest[:at]
                break
            depth -= 1
    count = len(re.findall(r"(?:^|, )_\d+: ", params))
    return name, count


def path_key(name):
    """The segments of a path, with an `impl` named by its file and line only."""
    name = re.sub(r"<impl at (?:[^>]*/)?([^/:>]+):(\d+)[^>]*>", r"<impl \1:\2>", name)
    segments, depth, current = [], 0, ""
    for ch in name:
        if ch == "<":
            depth += 1
        elif ch == ">":
            depth -= 1
        if ch == ":" and depth == 0:
            if current.endswith(":"):
                segments.append(current[:-1])
                current = ""
                continue
        current += ch
    segments.append(current)
    return tuple(s for s in segments if s)


def locals_of(body):
    """Local -> type, from the `let` lines; parameters come from the head."""
    types = {}
    for line in body:
        m = re.match(r"\s*let (?:mut )?(_\d+): (.*);$", line)
        if m:
            types[m.group(1)] = m.group(2)
    return types


def blocks_of(body):
    """[(label, is_cleanup, [statement], terminator)] in written order."""
    blocks, current = [], None
    for line in body:
        m = re.match(r"    (bb\d+)( \(cleanup\))?: \{$", line)
        if m:
            current = [m.group(1), bool(m.group(2)), []]
            continue
        if current is None:
            continue
        if line == "    }":
            stmts = [s for s in current[2] if s]
            term = stmts.pop() if stmts else ""
            blocks.append((current[0], current[1], stmts, term))
            current = None
            continue
        text = line.strip()
        text = re.sub(r"\s*//.*$", "", text)
        current[2].append(text)
    return blocks


# --- Normalizing ------------------------------------------------------------------------------

SCALAR = re.compile(r"^(-?[0-9][0-9_]*_(?:[iu](?:8|16|32|64|128|size))|-?[0-9.]+(?:e[+-]?[0-9]+)?_f(?:16|32|64|128)|true|false|'(?:[^'\\]|\\.[^']*)'|\(\))$")


def matching(text, at):
    """The index of the bracket that closes the one at `at`."""
    pairs = {"(": ")", "[": "]", "{": "}", "<": ">"}
    stack = [pairs[text[at]]]
    i = at + 1
    while i < len(text) and stack:
        ch = text[i]
        if ch == '"':
            i += 1
            while i < len(text) and text[i] != '"':
                i += 2 if text[i] == "\\" else 1
        elif ch in "([{":
            stack.append(pairs[ch])
        elif ch == "<" and stack[-1] == ">":
            stack.append(">")
        elif ch == stack[-1]:
            stack.pop()
        i += 1
    return i - 1


def strip_generics(text):
    """`a::<T>::b` and `a<T>` in paths without what is in the angle brackets."""
    out, i = [], 0
    while i < len(text):
        if text.startswith("::<", i):
            i = matching(text, i + 2) + 1
            continue
        out.append(text[i])
        i += 1
    return "".join(out)


def constant_end(text, at):
    """Where the constant starting at `at` ends: the next `,`, `)` or `;` outside brackets."""
    depth, i = 0, at
    while i < len(text):
        ch = text[i]
        if ch == '"':
            i += 1
            while i < len(text) and text[i] != '"':
                i += 2 if text[i] == "\\" else 1
        elif ch in "([{<":
            depth += 1
        elif ch in ")]}>":
            if depth == 0:
                return i
            depth -= 1
        elif ch in ",;" and depth == 0:
            return i
        elif text.startswith(" -> ", i) and depth == 0:
            return i
        i += 1
    return i


def normalize_constants(text):
    out, i = [], 0
    while True:
        at = text.find("const ", i)
        if at < 0 or (at > 0 and (text[at - 1].isalnum() or text[at - 1] == "_")):
            if at < 0:
                out.append(text[i:])
                break
            out.append(text[i:at + 6])
            i = at + 6
            continue
        out.append(text[i:at + 6])
        end = constant_end(text, at + 6)
        value = text[at + 6:end].strip()
        out.append(value if SCALAR.match(value) else "_")
        i = end
    return "".join(out)


def strip_field_types(text):
    """`(_1.0: T)` is `_1.0`; `(*_1)` and `(_1 as V)` are left."""
    out, i = [], 0
    while i < len(text):
        if text[i] == "(" and (text.startswith("(_", i) or text.startswith("((", i) or text.startswith("(*", i)):
            end = matching(text, i)
            inner = text[i + 1:end]
            depth, split = 0, None
            for k, ch in enumerate(inner):
                if ch in "([{<":
                    depth += 1
                elif ch in ")]}>":
                    depth -= 1
                elif inner.startswith(": ", k) and depth == 0:
                    split = k
                    break
            if split is not None and re.match(r"^[(_*]", inner):
                out.append(strip_field_types(inner[:split]))
            else:
                out.append("(" + strip_field_types(inner) + ")")
            i = end + 1
            continue
        out.append(text[i])
        i += 1
    return "".join(out)


def short_paths(text):
    """Every `a::b::C` is `C`, `<A as B>::c` is `c`, a closure is `{closure}`."""
    text = re.sub(r"\{closure@[^}]*\}", "{closure}", text)
    text = re.sub(r"\{coroutine@[^}]*\}", "{coroutine}", text)
    text = re.sub(r"<impl at [^>]*>", "impl", text)
    text = strip_generics(text)
    # `<T as Trait>::method` and `<[T]>::len`.
    while True:
        m = re.search(r"<[^<>]*>::", text)
        if not m:
            break
        text = text[:m.start()] + text[m.end():]
    return re.sub(r"\b(?:[A-Za-z_][A-Za-z0-9_]*::)+([A-Za-z_][A-Za-z0-9_]*|\{closure#\d+\})", r"\1", text)


def normalize_line(text):
    # How rustc marks a copy that is not retagged under its aliasing model; nothing here.
    text = text.replace("no_retag ", "")
    text = normalize_constants(text)
    text = strip_field_types(text)
    text = short_paths(text)
    # Aggregates by field name are by position; `[k of N]` has its length on one side only.
    text = re.sub(r"\b[a-z_][a-z0-9_]*: (?=(?:move|copy|const) )", "", text)
    text = re.sub(r" of \d+\]", " of ?]", text)
    text = re.sub(r"\s+", " ", text)
    return text


def renumbered(blocks, params):
    """The blocks in depth-first order from the first, their labels and locals numbered in
    the order they are reached and mentioned; the return slot and parameters keep theirs."""
    by_label = {b[0]: b for b in blocks}
    if not blocks:
        return [], {}
    order, seen, stack = [], set(), [blocks[0][0]]
    while stack:
        label = stack.pop()
        if label in seen or label not in by_label:
            continue
        seen.add(label)
        order.append(label)
        targets = re.findall(r"\bbb\d+\b", by_label[label][3])
        stack.extend(reversed(targets))
    labels = {label: f"bb{n}" for n, label in enumerate(order)}
    locals_map = {f"_{n}": f"_{n}" for n in range(params + 1)}

    def local(m):
        name = m.group(0)
        if name not in locals_map:
            locals_map[name] = f"_{len(locals_map)}"
        return locals_map[name]

    out = []
    for label in order:
        _, cleanup, stmts, term = by_label[label]
        lines = []
        for text in stmts + [term]:
            text = re.sub(r"\bbb\d+\b", lambda m: labels.get(m.group(0), "bb?"), text)
            text = re.sub(r"(?<![\w.])_\d+\b", local, text)
            lines.append(normalize_line(text))
        out.append((labels[label], cleanup, lines[:-1], lines[-1]))
    return out, locals_map


def terminator_kind(text):
    m = re.match(r"(?:\S+ = )?([A-Za-z_]+)", text)
    if not m:
        return text
    word = m.group(1)
    if " = " in text.split(" -> ")[0] and "(" in text:
        return "call"
    return word


def statement_shape(text):
    """A statement without its locals, blocks and constants, for counting kinds of difference."""
    text = re.sub(r"(?<![\w.])_\d+\b", "_", text)
    text = re.sub(r"\bbb\d+\b", "bb", text)
    text = re.sub(r"const [^,;)\]]+", "const", text)
    return text


# --- Comparing -------------------------------------------------------------------------------

def compare(rustc, ours):
    def by_key(found):
        table = collections.defaultdict(list)
        for name, params, body in found:
            table[(path_key(name), params)].append((name, params, body))
        return table

    theirs = functions(rustc)
    mine = functions(ours)
    mine_by_last = collections.defaultdict(list)
    for name, params, body in mine:
        key = path_key(name)
        mine_by_last[(key[-1], params)].append((key, name, body))
    used = set()
    pairs, unpaired_rustc = [], []
    for name, params, body in theirs:
        key = path_key(name)
        candidates = [c for c in mine_by_last[(key[-1], params)]
                      if c[0][-len(key):] == key and id(c) not in used]
        if not candidates:
            unpaired_rustc.append(name)
            continue
        chosen = candidates[0]
        used.add(id(chosen))
        pairs.append((name, params, body, chosen[1], chosen[2]))
    unpaired_ours = [c[1] for group in mine_by_last.values() for c in group if id(c) not in used]
    return pairs, unpaired_rustc, unpaired_ours


def evaluate(pairs):
    levels = collections.Counter()
    only_rustc, only_ours = collections.Counter(), collections.Counter()
    example = {}
    failures = []
    for name, params, rbody, oname, obody in pairs:
        rblocks, rlocals = renumbered(blocks_of(rbody), params)
        oblocks, olocals = renumbered(blocks_of(obody), params)
        shape = [(b[1], terminator_kind(b[3])) for b in rblocks] == [(b[1], terminator_kind(b[3])) for b in oblocks]
        counts = shape and [len(b[2]) for b in rblocks] == [len(b[2]) for b in oblocks]
        rl = [l for b in rblocks for l in b[2] + [b[3]]]
        ol = [l for b in oblocks for l in b[2] + [b[3]]]
        identical = counts and rblocks == oblocks
        typed = False
        if identical:
            rtypes = {rlocals[k]: short_paths(v) for k, v in locals_of(rbody).items() if k in rlocals}
            otypes = {olocals[k]: short_paths(v) for k, v in locals_of(obody).items() if k in olocals}
            typed = all(otypes.get(k) == v for k, v in rtypes.items())
        for level, ok in (("shape", shape), ("counts", counts), ("identical", identical), ("typed", typed)):
            levels[level] += ok
        if not identical:
            failures.append((name, rblocks, oblocks))
            rs = collections.Counter(statement_shape(l) for l in rl)
            os_ = collections.Counter(statement_shape(l) for l in ol)
            for line, n in (rs - os_).items():
                only_rustc[line] += 1
                example.setdefault(("rustc", line), name)
            for line, n in (os_ - rs).items():
                only_ours[line] += 1
                example.setdefault(("ours", line), name)
    return levels, only_rustc, only_ours, example, failures


def show_diff(name, rblocks, oblocks):
    import difflib
    def text(blocks):
        out = []
        for label, cleanup, stmts, term in blocks:
            out.append(f"{label}{' (cleanup)' if cleanup else ''}:")
            out += ["    " + s for s in stmts] + ["    " + term]
        return out
    print(f"\n--- rustc / +++ ra-jit: {name}")
    for line in difflib.unified_diff(text(rblocks), text(oblocks), lineterm="", n=1):
        if not line.startswith(("---", "+++")):
            print(line)


def main():
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument("workspace", nargs="?", default=os.path.join(HERE, "corpus"))
    ap.add_argument("--crate", default="cases")
    ap.add_argument("--level", type=int, default=0, help="ra-jit's MIR optimization level")
    ap.add_argument("--rustc-level", type=int, default=1, help="rustc's -Zmir-opt-level: 1 or 2")
    ap.add_argument("--ub-checks", action="store_true",
                    help="keep the checks of pointers rustc puts in a debug build, which ra-jit has not")
    ap.add_argument("--show", type=int, default=0, help="print this many differing functions")
    ap.add_argument("--top", type=int, default=12, help="list this many kinds of difference")
    ap.add_argument("--json", help="write the numbers here")
    args = ap.parse_args()
    workspace = os.path.abspath(args.workspace)
    rustc = rustc_mir(workspace, args.crate, args.rustc_level, args.ub_checks)
    ours, timing = ra_jit_mir(workspace, args.crate, args.level)
    pairs, unpaired_rustc, unpaired_ours = compare(rustc, ours)
    levels, only_rustc, only_ours, example, failures = evaluate(pairs)
    n = len(pairs)
    print(f"ra-jit level {args.level} against rustc -Zmir-opt-level={args.rustc_level}, crate {args.crate}")
    print(f"functions: {len(functions(rustc))} of rustc's, {len(functions(ours))} of ra-jit's, {n} paired")
    if timing:
        print(timing)
    for level in ("shape", "counts", "identical", "typed"):
        print(f"  {level:<10} {levels[level]:>5} / {n}  {100 * levels[level] / max(n, 1):5.1f}%")
    print(f"\nstatements rustc has and ra-jit has not, in how many functions (top {args.top}):")
    for line, count in only_rustc.most_common(args.top):
        print(f"  {count:>5}  {line[:110]}   e.g. {example[('rustc', line)]}")
    print(f"\nstatements ra-jit has and rustc has not (top {args.top}):")
    for line, count in only_ours.most_common(args.top):
        print(f"  {count:>5}  {line[:110]}   e.g. {example[('ours', line)]}")
    if unpaired_rustc:
        print(f"\n{len(unpaired_rustc)} of rustc's functions not paired, e.g. {', '.join(unpaired_rustc[:6])}")
    if unpaired_ours:
        print(f"{len(unpaired_ours)} of ra-jit's functions not paired, e.g. {', '.join(unpaired_ours[:6])}")
    for name, rblocks, oblocks in failures[:args.show]:
        show_diff(name, rblocks, oblocks)
    if args.json:
        with open(args.json, "w") as f:
            json.dump({"level": args.level, "rustc_level": args.rustc_level, "paired": n,
                       "rustc_functions": len(functions(rustc)), "ra_jit_functions": len(functions(ours)),
                       "levels": dict(levels), "timing": timing}, f, indent=1)


if __name__ == "__main__":
    main()
