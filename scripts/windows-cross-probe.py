#!/usr/bin/env python3
"""Opt-in Linux MSVC probe/bootstrap. No downloads, installs, uploads or signing."""
import argparse
import json
import os
from pathlib import Path
import platform
import shutil
import struct
import subprocess
import sys
import time

TARGETS = {"x86_64-pc-windows-msvc": ("x86_64", 0x8664),
           "aarch64-pc-windows-msvc": ("aarch64", 0xAA64)}


def machine(path):
    data = Path(path).read_bytes()
    if data[:2] != b"MZ" or len(data) < 64:
        raise ValueError("not a PE executable")
    offset = struct.unpack_from("<I", data, 60)[0]
    if offset > len(data) - 6 or data[offset:offset + 4] != b"PE\0\0":
        raise ValueError("missing PE signature")
    return struct.unpack_from("<H", data, offset + 4)[0]


def clean_env():
    # Do not propagate signing, cloud, SSH, Cargo registry or user Rust flags.
    return {k: os.environ[k] for k in ("PATH", "HOME", "LANG", "TMPDIR")
            if k in os.environ}


def run(args, cwd, records):
    start = time.monotonic()
    env = clean_env()
    if str(args[0]) == "justrust":
        # Match the rustc queried above, never fetch a pinned toolchain.
        env.update(JUSTRUST_TOOLCHAIN="system", JUSTRUST_TOOLCHAIN_AUTO_INSTALL="0",
                   JUSTRUST_NO_UPDATE="1", JUSTRUST_PREBUILT="off",
                   JUSTRUST_DEPCACHE="off", JUSTRUST_PEER_CACHE="off",
                   JUSTRUST_LINKER="off", JUSTRUST_CARGO_FORK="off")
    result = subprocess.run([str(a) for a in args], cwd=cwd, env=env,
                            text=True, stdout=subprocess.PIPE, stderr=subprocess.PIPE,
                            timeout=180)
    records.append({"command": [str(a) for a in args], "seconds": time.monotonic() - start,
                    "exit_code": result.returncode, "stdout": result.stdout,
                    "stderr": result.stderr})
    if result.returncode:
        raise RuntimeError(result.stdout + result.stderr)
    return result.stdout.strip()


def bootstrap(out, sdk, target, clang, linker):
    arch, _ = TARGETS[target]
    includes = [sdk / p for p in ("crt/include", "sdk/include/ucrt",
                                  "sdk/include/um", "sdk/include/shared")]
    libs = [sdk / p / arch for p in ("crt/lib", "sdk/lib/ucrt", "sdk/lib/um")]
    for p in includes + libs:
        if not p.is_dir():
            raise ValueError(f"missing SDK directory: {p}")
    # JSON string syntax is also TOML basic string syntax for these paths.
    q = json.dumps
    import shlex
    wrapper = out / "clang-cl"
    command = [clang, "--driver-mode=cl", f"--target={target}"]
    for p in includes:
        command += ["/imsvc", str(p)]
    wrapper.write_text("#!/bin/sh\nexec " + shlex.join(command) + ' "$@"\n')
    wrapper.chmod(0o700)
    config = out / ".cargo"
    config.mkdir()
    flags = [f"-Clink-arg=/libpath:{p}" for p in libs]
    (config / "config.toml").write_text(
        f"[target.{target}]\nlinker = {q(linker)}\nrustflags = {q(flags)}\n"
        f"[env]\nCC_{target.replace('-', '_')} = {q(str(wrapper))}\n")
    (out / "Cargo.toml").write_text('[package]\nname = "windows-cross-probe"\n'
                                   'version = "0.0.0"\nedition = "2021"\n[workspace]\n')
    (out / "src").mkdir()
    (out / "src/main.rs").write_text('fn main() { println!("Windows cross probe"); }\n')
    return wrapper, includes, libs


def main(argv=None):
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--opt-in", action="store_true", help="authorize fresh output directory creation")
    parser.add_argument("--out", type=Path, required=True, help="must not exist")
    parser.add_argument("--target", choices=TARGETS, default="x86_64-pc-windows-msvc")
    parser.add_argument("--sdk", type=Path, help="pre-provisioned, licensed, case-correct crt/ and sdk/ tree")
    args = parser.parse_args(argv)
    if not args.opt_in:
        parser.error("--opt-in required, nothing changed")
    if platform.system() != "Linux":
        parser.error("this probe requires Linux")
    out = args.out.absolute()
    out.mkdir(mode=0o700, parents=False, exist_ok=False)
    records = []
    report = {"target": args.target, "scope": "local cross-toolchain only, not hosted or runtime validation",
              "commands": records, "blockers": []}
    try:
        clang = shutil.which("clang")
        linker = shutil.which("lld-link")
        if not clang or not linker:
            raise ValueError("installed clang and lld-link are required, no auto-install")
        for tool in (clang, linker, "rustc"):
            run([tool, "--version"], out, records)
        source = out / "freestanding.c"
        source.write_text("void mainCRTStartup(void) {}\n")
        run([clang, f"--target={args.target}", "-ffreestanding", "-fno-stack-protector",
             "-c", source, "-o", out / "probe.obj"], out, records)
        run([linker, "/entry:mainCRTStartup", "/subsystem:console", "/nodefaultlib",
             f"/out:{out / 'probe.exe'}", out / "probe.obj"], out, records)
        if machine(out / "probe.exe") != TARGETS[args.target][1]:
            raise ValueError("PE machine does not match requested target")
        report["freestanding_pe"] = "passed (no CRT, SDK, Rust std or runtime verified)"
        libdir = Path(run(["rustc", "--print", "target-libdir", "--target", args.target], out, records))
        if not list(libdir.glob("libstd-*.rlib")):
            report["blockers"].append("matching Rust Windows std is not installed")
        if not args.sdk:
            report["blockers"].append("no pre-provisioned Windows SDK/CRT supplied")
        else:
            wrapper, _, libs = bootstrap(out, args.sdk.resolve(), args.target, clang, linker)
            native = out / "sdk.c"
            native.write_text('#include <windows.h>\n#include <stdio.h>\nint main(void) { puts("SDK probe"); return GetCurrentProcessId() == 0; }\n')
            run([wrapper, "/c", "/MD", native, f"/Fo{out / 'sdk.obj'}"], out, records)
            run([linker, "/subsystem:console", f"/out:{out / 'sdk.exe'}", out / "sdk.obj",
                 "kernel32.lib", *[f"/libpath:{p}" for p in libs]], out, records)
            if machine(out / "sdk.exe") != TARGETS[args.target][1]:
                raise ValueError("SDK PE machine mismatch")
            if not report["blockers"]:
                # Local-only requirement: target SDK paths are local and hosted support is unverified.
                run(["justrust", "build", "--local", "--offline", "--target", args.target,
                     "--target-dir", out / "target"], out, records)
                if machine(out / "target" / args.target / "debug/windows-cross-probe.exe") != TARGETS[args.target][1]:
                    raise ValueError("Rust PE machine mismatch")
                report["rust_pe"] = "passed (not executed)"
    except (ValueError, RuntimeError, OSError, subprocess.TimeoutExpired) as error:
        report["blockers"].append(str(error))
    report["verdict"] = "blocked" if report["blockers"] else "cross-linked, runtime unverified"
    (out / "report.json").write_text(json.dumps(report, indent=2) + "\n")
    print(json.dumps(report, indent=2))
    return 2 if report["blockers"] else 0


if __name__ == "__main__":
    sys.exit(main())
