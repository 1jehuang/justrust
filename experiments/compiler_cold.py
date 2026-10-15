#!/usr/bin/env python3
"""Verified Linux-cache-cold comparison, requiring an authorized root supervisor.

Only cache management runs privileged. Compiler and generated executables run
as the explicitly named unprivileged guest user. No CPU/EBS cache claim.
"""
import argparse
import ctypes
import hashlib
import json
import mmap
import os
from pathlib import Path
import pwd
import resource
import subprocess
import tempfile
import time
from compiler import source_for


def cached_pages(path):
    with path.open("rb") as file:
        size = os.fstat(file.fileno()).st_size
        if not size:
            return 0, 0
        pages = (size + mmap.PAGESIZE - 1) // mmap.PAGESIZE
        mapping = mmap.mmap(file.fileno(), size, flags=mmap.MAP_PRIVATE,
                            prot=mmap.PROT_READ | mmap.PROT_WRITE)
        anchor = ctypes.c_char.from_buffer(mapping)
        vector = (ctypes.c_ubyte * pages)()
        libc = ctypes.CDLL(None, use_errno=True)
        libc.mincore.argtypes = [ctypes.c_void_p, ctypes.c_size_t, ctypes.c_void_p]
        libc.mincore.restype = ctypes.c_int
        status = libc.mincore(ctypes.addressof(anchor), size, vector)
        del anchor
        mapping.close()
        if status:
            raise OSError(ctypes.get_errno(), "mincore failed")
        return sum(bool(v & 1) for v in vector), pages


def memory():
    wanted = {"Cached", "SReclaimable", "Dirty", "Writeback", "MemAvailable"}
    result = {}
    for line in Path("/proc/meminfo").read_text().splitlines():
        key, value = line.split(":", 1)
        if key in wanted:
            result[key + "_kib"] = int(value.split()[0])
    return result


def ensure_idle():
    active = []
    for entry in Path("/proc").iterdir():
        if not entry.name.isdigit():
            continue
        try:
            command = (entry / "comm").read_text().strip()
        except (FileNotFoundError, PermissionError, ProcessLookupError):
            continue
        if command in {"cargo", "rustc", "jr-rust", "jr-prototype", "rust-lld", "ld.lld", "ld", "cc", "gcc", "clang"}:
            active.append((entry.name, command))
    if active:
        raise RuntimeError(f"refusing host cache flush during active compilation: {active}")


def drop_caches():
    os.sync()
    with open("/proc/sys/vm/drop_caches", "w") as control:
        control.write("3\n")


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--binary", required=True, type=Path)
    parser.add_argument("--rustc", required=True, type=Path)
    parser.add_argument("--output", required=True, type=Path)
    parser.add_argument("--user", default="ubuntu")
    args = parser.parse_args()
    if os.geteuid() != 0:
        parser.error("requires root solely for authorized OS cache eviction")
    account = pwd.getpwnam(args.user)
    if account.pw_uid == 0:
        parser.error("compiler processes must run unprivileged")
    environment = dict(os.environ, HOME=account.pw_dir,
                       CARGO_HOME=account.pw_dir + "/.cargo",
                       RUSTUP_HOME=account.pw_dir + "/.rustup")
    def run(command):
        result = subprocess.run(command, capture_output=True, timeout=120,
                                user=account.pw_uid, group=account.pw_gid,
                                extra_groups=(), env=environment)
        if result.returncode:
            raise RuntimeError(f"{command!r}\n{result.stdout!r}\n{result.stderr!r}")
        return result
    binary = args.binary.resolve(strict=True)
    sysroot = Path(run([str(args.rustc), "--print", "sysroot"]).stdout.decode().strip())
    runtime_libs = sysroot / "lib" / "rustlib" / "x86_64-unknown-linux-gnu" / "lib"
    probes = [binary, sysroot / "bin" / "rustc"]
    probes.extend(sorted((sysroot / "lib").glob("*.so*")))
    probes.extend(sorted(runtime_libs.glob("*.rlib")))
    # Resolve aliases once, before any timed cache evictions.
    probes = list(dict.fromkeys(path.resolve(strict=True) for path in probes))
    version = run([str(args.rustc), "-Vv"]).stdout.decode()
    binary_digest = hashlib.sha256(binary.read_bytes()).hexdigest()
    with args.output.open("x") as receipt, tempfile.TemporaryDirectory(prefix="jr-cold-") as temporary:
        directory = Path(temporary)
        os.chown(directory, account.pw_uid, account.pw_gid)
        def record(row):
            receipt.write(json.dumps(row, sort_keys=True) + "\n")
            receipt.flush()
        record({"record":"environment", "rustc":version,
                "binary_sha256":binary_digest,
                "revision":run(["git","rev-parse","HEAD"]).stdout.decode().strip(),
                "instance_id":Path("/sys/devices/virtual/dmi/id/board_asset_tag").read_text().strip(),
                "instance_type":Path("/sys/devices/virtual/dmi/id/product_name").read_text().strip(),
                "topology":run(["lscpu"]).stdout.decode(),
                "measurement":"Linux-cache-cold/fresh-process/fresh-output/contended-uncontrolled",
                "flush":"sync then drop_caches=3 before verification and again immediately before timing",
                "probe_files":len(probes), "probe_bytes":sum(p.stat().st_size for p in probes),
                "repeats":7,"warmups":0,"compiler_uid":account.pw_uid,
                "cargo":False,"incremental":False,
                "excluded":"CPU caches, already-mapped shared pages, EBS/provider caches, reboot effects"})
        for count in (4,256):
            text = source_for(count)
            source = directory / "input.rs"
            source.write_text(text)
            for repeat in range(7):
                for tool in (("rustc","prototype") if repeat % 2 else ("prototype","rustc")):
                    ensure_idle()
                    before = memory()
                    started = time.perf_counter_ns()
                    drop_caches()
                    residency = []
                    for path in probes + [source]:
                        resident, pages = cached_pages(path)
                        residency.append({"file":str(path.relative_to(sysroot)) if path.is_relative_to(sysroot) else ("source.rs" if path == source else "prototype"),
                                          "resident_pages":resident,"total_pages":pages})
                    if any(row["resident_pages"] for row in residency):
                        record({"record":"failed_cold_verification","tool":tool,"residency":residency})
                        raise RuntimeError("compiler or sysroot artifact pages remain resident after eviction")
                    # Probing warms metadata, so evict a second time and do not
                    # open any compiler/source/sysroot file before exec.
                    ensure_idle()
                    drop_caches()
                    flush_ns = time.perf_counter_ns() - started
                    after = memory()
                    output = directory / f"out-{count}-{repeat}-{tool}"
                    if tool == "rustc":
                        command = [str(args.rustc),"--edition=2024","--crate-name","reference","-C","opt-level=0","-C","overflow-checks=yes",str(source),"-o",str(output)]
                    else:
                        command = [str(binary),str(source),"--workers","1","-o",str(output)]
                    usage_before = resource.getrusage(resource.RUSAGE_CHILDREN)
                    begin = time.perf_counter_ns()
                    compiled = run(command)
                    elapsed = time.perf_counter_ns() - begin
                    usage_after = resource.getrusage(resource.RUSAGE_CHILDREN)
                    execution = run([str(output)])
                    if execution.stdout != b"compiled correctly\n" or execution.stderr:
                        raise RuntimeError("wrong native program result")
                    row = {"record":"sample","functions":count+2,"source_bytes":len(text.encode()),
                           "source_sha256":hashlib.sha256(text.encode()).hexdigest(),
                           "tool":tool,"repeat":repeat,"process_wall_ns":elapsed,
                           "flush_and_probe_ns":flush_ns,"cache_before":before,"cache_after":after,
                           "residency":residency,"all_probed_pages_cold":True,
                           "major_faults":usage_after.ru_majflt - usage_before.ru_majflt,
                           "block_inputs":usage_after.ru_inblock - usage_before.ru_inblock,
                           "executable_bytes":output.stat().st_size,"executable_checked":True}
                    if tool == "prototype":
                        detail = json.loads(compiled.stdout)
                        if not detail["capture_complete"] or detail["failures"] or detail["events_dropped"]:
                            raise RuntimeError("incomplete prototype capture")
                        row["in_process_ns"] = detail["total_ns"]
                    record(row)
                    print(json.dumps({k:v for k,v in row.items() if k != "residency"}),flush=True)


if __name__ == "__main__":
    main()
