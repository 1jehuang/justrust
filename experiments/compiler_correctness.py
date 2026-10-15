#!/usr/bin/env python3
"""Seeded native executable differential checks against bootstrap Rust."""
import argparse
import hashlib
import json
from pathlib import Path
import random
import subprocess
import tempfile


def process(argv, timeout=30):
    return subprocess.run(argv, capture_output=True, timeout=timeout)


def expression(rng, depth):
    if depth == 0:
        return rng.choice(["x", "y", str(rng.randint(-9, 9))])
    a, b = expression(rng, depth - 1), expression(rng, depth - 1)
    return f"({a} {rng.choice(['+', '-', '*'])} {b})"


def source(rng):
    expr = expression(rng, 3)
    a, b = rng.randint(-7, 7), rng.randint(-7, 7)
    # A single input appears in both implementations. Generated values stay far
    # inside i32, while predicates make differences observable without formatting.
    return f'''fn value(x:i32,y:i32)->i32 {{ {expr} }}
fn recurse(n:i32)->i32 {{ if n < 1 {{ 1 }} else {{ n * recurse(n-1) }} }}
fn first()->i32 {{ println!("first"); {a} }}
fn second()->i32 {{ println!("second"); {b} }}
fn pick(a:i32,b:i32)->i32 {{ a-b }}
fn main() {{
 let x = value({a},{b});
 if x < 0 {{ println!("negative"); }} else {{ println!("nonnegative"); }}
 if x % 7 == 0 {{ println!("divisible"); }}
 let z = if x > 0 {{ x / 3 }} else {{ x / -3 }};
 if z > 10 {{ println!("large"); }}
 let mut i=0; let mut sum=0;
 while i < {rng.randint(1, 7)} {{ sum=sum+i; i=i+1; }}
 if sum % 2 == 0 {{ println!("even"); }}
 if recurse(5) == 120 && !(x == 0) {{ println!("recursive"); }}
 if pick(first(),second()) < 0 {{ println!("ordered"); }}
 {{ let x=true; if x || false && false {{ println!("scoped"); }} }}
 println!("done");
}}'''


def main():
    p = argparse.ArgumentParser(description=__doc__)
    p.add_argument("--binary", required=True, type=Path)
    p.add_argument("--rustc", required=True, type=Path)
    p.add_argument("--output", required=True, type=Path)
    p.add_argument("--cases", type=int, default=64)
    p.add_argument("--seed", type=int, default=1)
    args = p.parse_args()
    if not 1 <= args.cases <= 10000:
        p.error("cases must be 1..10000")
    binary = args.binary.resolve(strict=True)
    rustc = args.rustc.absolute()
    rng = random.Random(args.seed)
    with args.output.open("x") as out, tempfile.TemporaryDirectory(prefix="jr-differential-") as temporary:
        directory = Path(temporary)
        def record(row):
            out.write(json.dumps(row, sort_keys=True) + "\n")
            out.flush()
        version = process([str(rustc), "-Vv"])
        assert version.returncode == 0, version.stderr
        record({"record":"environment", "seed":args.seed, "cases":args.cases,
                "reference":version.stdout.decode(),
                "revision":process(["git","rev-parse","HEAD"]).stdout.decode().strip(),
                "binary_sha256":hashlib.sha256(binary.read_bytes()).hexdigest()})
        for index in range(args.cases):
            text = source(rng)
            input_path = directory / f"input{index}.rs"
            input_path.write_text(text)
            ours, reference = directory / f"ours{index}", directory / f"stock{index}"
            workers = (1, 8, 96)[index % 3]
            a = process([str(binary),str(input_path),"-o",str(ours),"--workers",str(workers)])
            b = process([str(rustc),"--edition=2024","--crate-name","reference","-C","overflow-checks=yes",str(input_path),"-o",str(reference)])
            row = {"record":"case", "case":index,"workers":workers,"source":text,
                   "source_sha256":hashlib.sha256(text.encode()).hexdigest(),
                   "our_compile_exit":a.returncode,"stock_compile_exit":b.returncode}
            if a.returncode == b.returncode == 0:
                x, y = process([str(ours)], 5), process([str(reference)], 5)
                row.update(our_exit=x.returncode,stock_exit=y.returncode,
                           stdout=x.stdout.decode(),matched=x.returncode == y.returncode and x.stdout == y.stdout and x.stderr == y.stderr)
            else:
                row.update(matched=False,our_diagnostic=a.stderr.decode(),stock_diagnostic=b.stderr.decode())
            record(row)
            if not row["matched"]:
                raise RuntimeError(f"differential failure case {index}, retained source and diagnostics in {args.output}")
            print(f"differential {index+1}/{args.cases}: matched",flush=True)


if __name__ == "__main__":
    main()
