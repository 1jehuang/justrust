#!/usr/bin/env python3
"""How complete `cargo blaze doc` is next to rustdoc's `cargo doc`, crate by crate.

usage: docdiff.py <workspace> [<cargo doc flags>...] [--show N] [--json FILE] [--keep]

Runs stock `cargo +<toolchain> doc` (the oracle, into `target/doc-oracle`) and `ra-jit cargo
doc` (into `target/doc`) on the workspace with the same flags, times both, and compares what
they wrote, for every crate both documented:

  pages      the item pages (`struct.Foo.html`, `mod/index.html`), redirects left out
  anchors    on pages both have, the members and impls anchored by id: `method.new`,
             `structfield.x`, `variant.A`, `tymethod.f`, `associatedtype.T`, `impl-Debug-for-Foo`
  links      on pages both have, the intra-doc links of the docs: each `<a title="kind path">`
             in a docblock, rustdoc's mark of a link it resolved, by its title
  warnings   the `unresolved link to` warnings each printed

and prints, per crate, what each side has and what blaze misses or adds, with up to `--show`
examples of each. Exit status is 0 unless a side fails. RA_JIT_BIN names the ra-jit binary.
"""
import argparse, collections, html, json, os, re, shutil, subprocess, sys, time

HERE = os.path.dirname(os.path.abspath(__file__))
ROOT = os.path.dirname(HERE)
TOOLCHAIN = open(os.path.join(HERE, "toolchain")).read().strip()
RA_JIT = os.environ.get("RA_JIT_BIN") or os.path.join(ROOT, "target/release/ra-jit")

NOT_CRATES = {"src", "static.files", "search-index", "search.index", "trait.impl", "type.impl",
              "implementors", "settings.html", "help.html"}
ANCHOR = re.compile(
    r'id="((?:method|tymethod|structfield|variant|associatedtype|associatedconstant|impl)[.-][^"]*)"')
TITLED = re.compile(r'<a [^>]*?title="([^"]+)"[^>]*>')
DOCBLOCK = re.compile(r"""<div class=["']docblock[^"']*["']>(.*?)</div>""", re.S)
UNRESOLVED = re.compile(r"warning: unresolved link to `([^`]*)`")


def run(command, cwd, env=None):
    started = time.monotonic()
    done = subprocess.run(command, cwd=cwd, env=env, capture_output=True, text=True)
    return done, time.monotonic() - started


def crates(doc):
    if not os.path.isdir(doc):
        return []
    return sorted(it for it in os.listdir(doc)
                  if it not in NOT_CRATES and os.path.isfile(os.path.join(doc, it, "index.html")))


def pages(doc, krate):
    """Each item page of the crate, from the crate's directory, with its text."""
    out = {}
    base = os.path.join(doc, krate)
    for dirpath, _, files in os.walk(base):
        for name in files:
            if not name.endswith(".html") or name == "all.html":
                continue
            path = os.path.join(dirpath, name)
            text = open(path, encoding="utf-8", errors="replace").read()
            if 'http-equiv="refresh"' in text[:2000]:
                continue
            out[os.path.relpath(path, base)] = text
    return out


def anchors(text):
    # rustdoc numbers a second id of a name (`method.fmt-1`): the names are compared.
    return collections.Counter(re.sub(r"-\d+$", "", it) for it in ANCHOR.findall(text))


def links(text):
    out = collections.Counter()
    for block in DOCBLOCK.findall(text):
        for title in TITLED.findall(block):
            out[html.unescape(title)] += 1
    return out


def missing_extra(oracle, blaze):
    return sorted((oracle - blaze).elements()), sorted((blaze - oracle).elements())


def compare(oracle_doc, blaze_doc, show):
    report = {}
    both = sorted(set(crates(oracle_doc)) & set(crates(blaze_doc)))
    for krate in both:
        theirs, ours = pages(oracle_doc, krate), pages(blaze_doc, krate)
        common = sorted(set(theirs) & set(ours))
        row = {
            "pages": [len(theirs), len(ours), sorted(set(theirs) - set(ours)), sorted(set(ours) - set(theirs))],
        }
        for what, read in (("anchors", anchors), ("links", links)):
            total_o = total_b = 0
            missing, extra = [], []
            for page in common:
                o, b = read(theirs[page]), read(ours[page])
                total_o += sum(o.values())
                total_b += sum(b.values())
                m, e = missing_extra(o, b)
                missing += [f"{page}: {it}" for it in m]
                extra += [f"{page}: {it}" for it in e]
            row[what] = [total_o, total_b, missing, extra]
        report[krate] = row
    only = {"oracle": sorted(set(crates(oracle_doc)) - set(crates(blaze_doc))),
            "blaze": sorted(set(crates(blaze_doc)) - set(crates(oracle_doc)))}
    return report, only


def share(have, of):
    return f"{100.0 * have / of:5.1f}%" if of else "  -   "


def main():
    parser = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    parser.add_argument("workspace")
    parser.add_argument("--show", type=int, default=5)
    parser.add_argument("--json")
    parser.add_argument("--keep", action="store_true", help="keep the oracle's docs from an earlier run")
    asked, flags = parser.parse_known_args()
    ws = os.path.abspath(asked.workspace)
    oracle_doc = os.path.join(ws, "target/doc-oracle/doc")
    blaze_doc = os.path.join(ws, "target/doc")

    env = dict(os.environ)
    if asked.keep and os.path.isdir(oracle_doc):
        oracle_wall, oracle_err = None, open(os.path.join(ws, "target/doc-oracle/stderr")).read()
    else:
        shutil.rmtree(os.path.join(ws, "target/doc-oracle"), ignore_errors=True)
        done, oracle_wall = run(["cargo", f"+{TOOLCHAIN}", "doc", "--target-dir", "target/doc-oracle", *flags], ws, env)
        oracle_err = done.stderr
        os.makedirs(os.path.join(ws, "target/doc-oracle"), exist_ok=True)
        open(os.path.join(ws, "target/doc-oracle/stderr"), "w").write(oracle_err)
        if done.returncode != 0:
            sys.exit(f"cargo doc failed:\n{done.stderr[-4000:]}")
    shutil.rmtree(blaze_doc, ignore_errors=True)
    done, blaze_wall = run([RA_JIT, "cargo", "doc", *flags], ws, env)
    blaze_err = done.stderr
    if done.returncode != 0:
        # blaze's error is the first line, what a build script said follows it.
        said = done.stderr if len(done.stderr) <= 6000 else f"{done.stderr[:3000]}\n...\n{done.stderr[-3000:]}"
        sys.exit(f"cargo blaze doc failed:\n{said}")

    report, only = compare(oracle_doc, blaze_doc, asked.show)
    print(f"{os.path.basename(ws)}: cargo doc {oracle_wall and f'{oracle_wall:.1f}s' or '(kept)'}, "
          f"cargo blaze doc {blaze_wall:.1f}s")
    if only["oracle"]:
        print(f"  crates only rustdoc documented: {', '.join(only['oracle'])}")
    if only["blaze"]:
        print(f"  crates only blaze documented: {', '.join(only['blaze'])}")
    totals = collections.Counter()
    print(f"  {'crate':24} {'pages':>17} {'anchors':>19} {'links':>17}")
    for krate, row in report.items():
        cells = []
        for what in ("pages", "anchors", "links"):
            o, b, missing, extra = row[what]
            have = o - len(missing)
            totals[what + "_o"] += o
            totals[what + "_have"] += have
            totals[what + "_extra"] += len(extra)
            cells.append(f"{have:>5}/{o:<5} {share(have, o)}")
        print(f"  {krate:24} {cells[0]:>17} {cells[1]:>19} {cells[2]:>17}")
    cells = [f"{totals[w + '_have']:>5}/{totals[w + '_o']:<5} {share(totals[w + '_have'], totals[w + '_o'])}"
             for w in ("pages", "anchors", "links")]
    print(f"  {'all':24} {cells[0]:>17} {cells[1]:>19} {cells[2]:>17}")
    print(f"  extra in blaze: {totals['pages_extra']} pages, {totals['anchors_extra']} anchors, "
          f"{totals['links_extra']} links")
    theirs = collections.Counter(UNRESOLVED.findall(oracle_err))
    ours = collections.Counter(UNRESOLVED.findall(blaze_err))
    m, e = missing_extra(theirs, ours)
    print(f"  unresolved-link warnings: rustdoc {sum(theirs.values())}, blaze {sum(ours.values())}, "
          f"blaze misses {len(m)}, adds {len(e)}")
    if asked.show:
        for krate, row in report.items():
            for what in ("pages", "anchors", "links"):
                _, _, missing, extra = row[what]
                if missing:
                    print(f"  {krate} {what} missing: {missing[:asked.show]}")
                if extra:
                    print(f"  {krate} {what} extra: {extra[:asked.show]}")
        if m:
            print(f"  warnings missing: {m[:asked.show]}")
        if e:
            print(f"  warnings extra: {e[:asked.show]}")
    if asked.json:
        json.dump({"workspace": ws, "oracle_wall": oracle_wall, "blaze_wall": blaze_wall, "crates": report,
                   "only": only, "warnings": {"oracle": theirs, "blaze": ours}}, open(asked.json, "w"), indent=1)


if __name__ == "__main__":
    main()
