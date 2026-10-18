#!/usr/bin/env python3
"""Freeze and independently classify leading-zero sources using stock rustc.

Usage: python3 experiments/leading_zeros_oracle.py --output-dir NEW_DIRECTORY
Optional --freeze-dir NEW_DIRECTORY publishes the classified source corpus.
All output directories are exclusive. No native compiler is invoked.
"""
import argparse
import hashlib
import json
from pathlib import Path
import subprocess

PROVIDER_HASH = "2d109735cfff1317afd02e093012ee4d4c3b5bf39431da45f6afe99002b0f9b5"


def digest(data):
    return hashlib.sha256(data).hexdigest()


def corpus():
    cases = []
    for signed in (False, True):
        for width_name in ("8", "16", "32", "64", "128", "size"):
            width = 64 if width_name == "size" else int(width_name)
            ty = ("i" if signed else "u") + width_name
            maximum = (1 << (width - int(signed))) - 1
            values = [0, 1, 2, maximum, (1 << (width - 2))]
            values += [-1, -(1 << (width - 1))] if signed else [1 << (width - 1)]
            if width == 128:
                values += [(1 << 63), (1 << 64), (1 << 65), (1 << 96) + 1]
            values = list(dict.fromkeys(values))
            functions = []
            for index, value in enumerate(values):
                bits = value & ((1 << width) - 1)
                expected = width - bits.bit_length()
                functions.append(f"fn c{index}(x:{ty})->bool{{let n:u32=x.leading_zeros();n=={expected}u32}}")
            condition = " && ".join(f"c{i}({v}{ty})" for i, v in enumerate(values))
            source = "\n".join(functions) + f'\nfn main(){{if {condition}{{println!("ok");}}}}\n'
            cases.append((ty, "valid", source, "ok\n"))
    cases += [
        ("once", "valid", 'fn next(p:&mut i32)->u64{*p=*p+1;1u64}\nfn main(){let mut n=0;let z=next(&mut n).leading_zeros();if n==1 && z==63u32{println!("ok");}}\n', "ok\n"),
        ("nominal", "valid", 'struct S{n:i32}\nimpl S{fn leading_zeros(self)->i32{self.n+2}}\nfn main(){let s=S{n:5};if s.leading_zeros()==7{println!("ok");}}\n', "ok\n"),
        ("divergent", "valid", 'fn f()->u32{({return 7u32;1u64}).leading_zeros()}\nfn main(){if f()==7u32{println!("ok");}}\n', "ok\n"),
        ("nested", "valid", 'fn nested(x:u128)->u32{x.leading_zeros().leading_zeros()}\nfn main(){if nested(1u128)==25u32 && 0u8.leading_zeros().leading_zeros()==28u32{println!("ok");}}\n', "ok\n"),
        ("u128_positions", "valid", 'fn verify()->bool{let mut x=1u128;let mut i=0u32;while i<128u32{if x.leading_zeros()!=127u32-i{return false;}if i<127u32{x=x<<1u8;}i=i+1u32;}0u128.leading_zeros()==128u32}\nfn main(){if verify(){println!("ok");}}\n', "ok\n"),
        ("bytes_exhaustive", "valid", 'fn verify()->bool{let mut x=0u16;while x<256u16{let mut t=x;let mut expected=8u32;while t!=0u16{t=t>>1u8;expected=expected-1u32;}let unsigned=x as u8;let signed=x as i8;if unsigned.leading_zeros()!=expected || signed.leading_zeros()!=expected{return false;}x=x+1u16;}true}\nfn main(){if verify(){println!("ok");}}\n', "ok\n"),
        ("float", "invalid", 'fn main(){let n=1.0f64.leading_zeros();}\n', ""),
        ("bool", "invalid", 'fn main(){let n=true.leading_zeros();}\n', ""),
        ("char", "invalid", "fn main(){let n='a'.leading_zeros();}\n", ""),
        ("generic", "invalid", 'fn f<T>(x:T)->u32{x.leading_zeros()} fn main(){}\n', ""),
        ("arity", "invalid", 'fn main(){let n=1u32.leading_zeros(2u32);}\n', ""),
        ("turbofish", "invalid", 'fn main(){let n=1u32.leading_zeros::<u32>();}\n', ""),
        ("result", "invalid", 'fn main(){let n:i32=1u32.leading_zeros();}\n', ""),
        ("dead", "invalid", 'fn unused()->u32{false.leading_zeros()} fn main(){}\n', ""),
        ("ufcs", "legal-unsupported", 'fn main(){if u32::leading_zeros(1u32)==31u32{println!("ok");}}\n', "ok\n"),
        ("reference", "legal-unsupported", 'fn main(){let n=1u32;let r=&n;if r.leading_zeros()==31u32{println!("ok");}}\n', "ok\n"),
        ("unsuffixed", "valid", 'fn main(){let n:u32=1;if n.leading_zeros()==31u32{println!("ok");}}\n', "ok\n"),
    ]
    pairs = [(1, 0), (1, -100), (1, 100), (2, 0), (3, -1), (1 << 31, -31), (1 << 32, 10), (1 << 63, -100), ((1 << 64) - 1, 100)]
    funcs = []
    for i, (mant, exp) in enumerate(pairs):
        # Independent exact integer arithmetic, not a leading_zeros call.
        expected = (((mant - 1).bit_length() + exp) * 1292913986) // (1 << 32)
        funcs.append(f"fn c{i}()->bool{{estimator::estimate_scaling_factor({mant}u64,{exp}i16)=={expected}i16}}")
    funcs.append('fn sweep(mant:u64,bits:i64)->bool{let mut exp=-1074i16;while exp<=1024i16{let expected=(((bits+exp as i64)*1292913986i64)>>32) as i16;if estimator::estimate_scaling_factor(mant,exp)!=expected{return false;}exp=exp+1i16;}true}')
    conditions = [f'c{i}()' for i in range(len(pairs))]
    conditions += [f'sweep({mant}u64,{(mant - 1).bit_length()}i64)' for mant in dict.fromkeys(m for m, _ in pairs)]
    source = 'mod estimator;\n' + '\n'.join(funcs) + '\nfn main(){if ' + ' && '.join(conditions) + '{println!("ok");}}\n'
    cases.append(("estimator", "valid", source, "ok\n"))
    return cases


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--output-dir", type=Path, required=True)
    parser.add_argument("--freeze-dir", type=Path)
    parser.add_argument("--rustc", default="/usr/bin/rustc")
    args = parser.parse_args()
    args.output_dir.mkdir(parents=True, exist_ok=False)
    if args.freeze_dir:
        args.freeze_dir.mkdir(parents=True, exist_ok=False)
    records = []

    def run(command, directory):
        timed_out = False
        try:
            result = subprocess.run(command, cwd=directory, capture_output=True, timeout=60, check=False)
        except subprocess.TimeoutExpired as error:
            timed_out = True
            result = subprocess.CompletedProcess(command, 124, error.stdout or b"", error.stderr or b"")
        except OSError as error:
            result = subprocess.CompletedProcess(command, 127, b"", str(error).encode())
        record = {"command": command, "cwd": str(directory), "status": result.returncode, "timed_out": timed_out,
                  "stdout": result.stdout.decode(errors="replace"), "stderr": result.stderr.decode(errors="replace"),
                  "stdout_sha256": digest(result.stdout), "stderr_sha256": digest(result.stderr)}
        records.append(record)
        return result

    manifest = []
    failures = []
    try:
        version = run([args.rustc, "-Vv"], args.output_dir)
        if version.returncode != 0:
            raise RuntimeError("stock rustc -Vv failed")
        if "host: x86_64-unknown-linux-gnu" not in version.stdout.decode():
            raise RuntimeError("corpus assumes 64-bit Linux")
        root = Path(__file__).resolve().parents[1]
        provider = (root / "libraries/probes/estimator/estimator.rs").read_bytes()
        if digest(provider) != PROVIDER_HASH:
            raise RuntimeError("original estimator bytes changed")
        for name, category, source, expected in corpus():
            directory = args.output_dir / name
            directory.mkdir()
            data = source.encode()
            (directory / "main.rs").write_bytes(data)
            if name == "estimator":
                (directory / "estimator.rs").write_bytes(provider)
            check = run([args.rustc, "--edition=2024", "--emit=metadata", "main.rs", "-o", "check.rmeta"], directory)
            build = run([args.rustc, "--edition=2024", "main.rs", "-o", "program"], directory)
            valid = category != "invalid"
            if check.returncode == build.returncode == 0:
                actual = "accepted"
            elif check.returncode == build.returncode == 1:
                actual = "rejected"
            else:
                actual = "inconclusive"
                failures.append(name + ": compiler failure or inconsistent check/build")
            if (check.returncode == 0) != valid or (build.returncode == 0) != valid:
                failures.append(name + ": classification mismatch")
            if build.returncode == 0:
                execution = run([str((directory / "program").resolve())], directory)
                if execution.returncode != 0 or execution.stdout != expected.encode() or execution.stderr:
                    failures.append(name + ": execution mismatch")
            manifest.append({"name": name, "category": category, "stock": actual, "source_sha256": digest(data), "expected_stdout": expected})
            if args.freeze_dir:
                (args.freeze_dir / (name + ".rs")).write_bytes(data)
                (args.freeze_dir / (name + ".stdout")).write_text(expected)
        if args.freeze_dir:
            (args.freeze_dir / "manifest.tsv").write_text("".join(f'{c["name"]}\t{c["category"]}\t{c["stock"]}\t{c["source_sha256"]}\n' for c in manifest))
    except BaseException as error:
        failures.append(f"{type(error).__name__}: {error}")
        raise
    finally:
        receipt = {"schema": 1, "provider_sha256": PROVIDER_HASH, "cases": manifest, "commands": records, "failures": failures}
        serialized = json.dumps(receipt, indent=2, sort_keys=True) + "\n"
        (args.output_dir / "receipt.json").write_text(serialized)
        if args.freeze_dir:
            # Preserve commands and complete outputs without machine-private paths.
            portable = serialized.replace(str(args.output_dir.resolve()), "$OUTPUT")
            portable = portable.replace(str(args.output_dir), "$OUTPUT")
            (args.freeze_dir / "stock-receipt.json").write_text(portable)
    print(json.dumps({"receipt": str(args.output_dir / "receipt.json"), "receipt_sha256": digest((args.output_dir / "receipt.json").read_bytes()), "cases": len(manifest), "failures": failures}, sort_keys=True))
    if failures:
        raise SystemExit(1)


if __name__ == "__main__":
    main()
