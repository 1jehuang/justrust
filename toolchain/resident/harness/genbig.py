#!/usr/bin/env python3
"""genbig.py DIR [N] [FILES]: a workspace whose binary has N functions (default 500) in FILES
files (default 1).

Used by `fnincr.py` to measure what an edit inside one function of a big file costs, against
the same functions spread over many small files. Every function is reached from `main`; each
has a loop, a match, a closure, a nested block, a `panic!` and a call into its neighbour, like
the code of a large real file. Function `f{i}` holds the line `let edit_{i} = {i}u64;`, which
is what the edits change. With one file it is `src/big.rs`; with more, `src/big/m{k}.rs`, and
`f{i}` is in `m{i * FILES // N}`.
"""
import os, sys


def function(i, n, path):
    nxt = (i + 1) % n
    return f"""
/// Function {i}.
pub fn f{i}(x: u64) -> u64 {{
    let edit_{i} = {i}u64;
    let mut acc = x.wrapping_add(edit_{i});
    for k in 0..(x % 7) {{
        acc = acc.wrapping_mul(31).wrapping_add(k);
    }}
    let pick = match acc % 4 {{
        0 => acc / 3,
        1 => acc ^ 0x5a5a,
        2 => {{
            let t = acc.rotate_left(5);
            t.wrapping_sub(1)
        }}
        _ => acc.wrapping_add(17),
    }};
    let scale = |v: u64| v.wrapping_mul({i % 13 + 1});
    let words: Vec<u64> = (0..3u64).map(|w| scale(w + pick)).collect();
    if words.len() > 5 {{
        panic!("f{i}: {{}} words", words.len());
    }}
    if x > 1 << 40 {{
        return {path(nxt)}f{nxt}(x >> 1);
    }}
    words.iter().sum::<u64>() ^ pick
}}
"""


def main():
    out = sys.argv[1]
    n = int(sys.argv[2]) if len(sys.argv) > 2 else 500
    files = int(sys.argv[3]) if len(sys.argv) > 3 else 1
    os.makedirs(os.path.join(out, "src"), exist_ok=True)
    open(os.path.join(out, "Cargo.toml"), "w").write(
        '[package]\nname = "big"\nversion = "0.1.0"\nedition = "2021"\n\n[workspace]\n')
    module = lambda i: i * files // n
    if files == 1:
        path = lambda i: ""
        call = lambda i: f"big::f{i}"
    else:
        path = lambda i: f"super::m{module(i)}::"
        call = lambda i: f"big::m{module(i)}::f{i}"
    calls = "\n".join(f"    sum = sum.wrapping_add({call(i)}(n + {i}));" for i in range(n))
    open(os.path.join(out, "src/main.rs"), "w").write(
        "mod big;\n\nfn main() {\n    let n: u64 = std::env::args().count() as u64;\n"
        f"    let mut sum = 0u64;\n{calls}\n    println!(\"{{sum}}\");\n}}\n")
    if files == 1:
        open(os.path.join(out, "src/big.rs"), "w").write(
            "//! One big file.\n" + "".join(function(i, n, path) for i in range(n)))
        return
    os.makedirs(os.path.join(out, "src/big"), exist_ok=True)
    open(os.path.join(out, "src/big.rs"), "w").write(
        "".join(f"pub mod m{k};\n" for k in range(files)))
    for k in range(files):
        body = "".join(function(i, n, path) for i in range(n) if module(i) == k)
        open(os.path.join(out, f"src/big/m{k}.rs"), "w").write(f"//! Module {k}.\n" + body)


if __name__ == "__main__":
    main()
