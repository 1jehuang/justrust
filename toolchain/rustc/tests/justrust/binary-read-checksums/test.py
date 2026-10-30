#!/usr/bin/env python3
"""Exercise the real checkout driver at the read/dep-info boundary, without installing it."""
import argparse
import hashlib
import json
import os
from pathlib import Path
import subprocess
import tempfile
import time


def checksum(data, path="payload.bin"):
    return f"# checksum:sha256={hashlib.sha256(data).hexdigest()} file_len:{len(data)} {path}"


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--rustc", required=True, type=Path)
    parser.add_argument("--output", type=Path)
    parser.add_argument("--baseline", action="store_true", help="Capture the known pre-fix mutation bug")
    checkout = Path(__file__).resolve().parents[3]
    parser.add_argument("--driver-rustc", type=Path, default=checkout / "build/host/stage0/bin/rustc")
    parser.add_argument("--driver-sysroot", type=Path, default=checkout / "build/host/stage0-sysroot")
    args = parser.parse_args()
    rustc = args.rustc.absolute()
    sysroot = rustc.parent.parent
    driver_libraries = list((sysroot / "lib").glob("librustc_driver-*.so"))
    assert len(driver_libraries) == 1, driver_libraries
    driver_library = driver_libraries[0]
    driver_metadata_dir = Path(subprocess.check_output([
        args.driver_rustc.absolute(), "--sysroot", args.driver_sysroot.absolute(),
        "--print", "target-libdir"], text=True).strip())
    driver_metadata = driver_metadata_dir / driver_library.with_suffix(".rmeta").name
    assert driver_metadata.is_file(), f"Matching full driver metadata required: {driver_metadata}"
    with driver_library.open("rb") as library, (driver_metadata_dir / driver_library.name).open("rb") as paired:
        driver_sha256 = hashlib.file_digest(library, "sha256").hexdigest()
        assert driver_sha256 == hashlib.file_digest(paired, "sha256").hexdigest()
    env = dict(os.environ)
    env["LD_LIBRARY_PATH"] = str(sysroot / "lib") + os.pathsep + env.get("LD_LIBRARY_PATH", "")
    results = {"compiler": subprocess.check_output([rustc, "-Vv"], text=True),
               "driver_library": driver_library.name,
               "driver_sha256": driver_sha256,
               "cases": []}
    source = '''fn main() {
    print!("{}|{}|{}", include_str!("payload.bin"),
        std::str::from_utf8(include_bytes!("payload.bin")).unwrap(),
        std::str::from_utf8(include_bytes!("payload.bin")).unwrap());
}
'''
    with tempfile.TemporaryDirectory(prefix="jr-binary-checksums-") as temp:
        work = Path(temp)
        driver = work / "driver"
        driver_options = ["--cfg=legacy_checksum_fixture"] if args.baseline else []
        # Bootstrap uses split full metadata and dylib stubs. Resolve both from the
        # paired sysroot, whose driver library was byte-checked against the tested one.
        subprocess.run([args.driver_rustc.absolute(), Path(__file__).with_name("driver.rs"),
                        "--sysroot", args.driver_sysroot.absolute(), "--edition=2024", "-o", driver,
                        "-Cprefer-dynamic", *driver_options], env=dict(env, RUSTC_BOOTSTRAP="1"), check=True)
        modes = ["mutate"] if args.baseline else [
            "mutate", "delete", "identical", "different", "default-off", "untracked", "unchanged"]
        for mode in modes:
            case = work / mode
            case.mkdir()
            (case / "main.rs").write_text(source)
            (case / "payload.bin").write_bytes(b"old")
            options = ["main.rs", "--sysroot", str(sysroot), "--edition=2024",
                       "--emit=link=app,dep-info=app.d",
                       "-Cincremental=incremental"]
            if mode != "default-off":
                options += ["-Zchecksum-hash-algorithm=sha256"]
            start = time.monotonic()
            subprocess.run([driver, *options], cwd=case,
                           env=dict(env, READ_BOUNDARY_MODE=mode), check=True)
            artifact = subprocess.check_output([case / "app"], text=True)
            assert artifact == "old|old|old", (mode, artifact)
            depinfo = (case / "app.d").read_text()
            assert "payload.bin:" in depinfo
            if args.baseline:
                assert checksum(b"new content") in depinfo, depinfo
                assert checksum(b"old") not in depinfo, depinfo
            elif mode in ("different", "default-off"):
                assert not any(line.startswith("# checksum:") and line.endswith(" payload.bin")
                               for line in depinfo.splitlines()), depinfo
            else:
                assert checksum(b"old") in depinfo, depinfo
                assert checksum(b"new content") not in depinfo, depinfo
            if mode == "default-off":
                assert "# checksum:" not in depinfo, depinfo
            if mode == "untracked":
                assert checksum(b"later", "untracked.bin") in depinfo, depinfo
                assert "missing.bin:" in depinfo
                assert not any(line.startswith("# checksum:") and line.endswith(" missing.bin")
                               for line in depinfo.splitlines()), depinfo
            # A subsequent ordinary compile must embed and checksum the new bytes, including
            # when reusing the incremental directory of the deliberately raced compile.
            (case / "payload.bin").write_bytes(b"new content")
            for repeat in range(2):
                subprocess.run([rustc, *options], cwd=case, env=env, check=True)
                assert subprocess.check_output([case / "app"], text=True) == "new content|new content|new content"
                new_depinfo = (case / "app.d").read_text()
                if mode != "default-off":
                    assert checksum(b"new content") in new_depinfo, new_depinfo
                    assert checksum(b"old") not in new_depinfo, new_depinfo
            result = {"mode": mode, "status": "bug reproduced" if args.baseline else "passed", "compilations": 3,
                      "artifact_at_boundary": artifact, "wall_seconds": round(time.monotonic() - start, 3)}
            result["checksum_at_boundary"] = next((line for line in depinfo.splitlines()
                if line.startswith("# checksum:") and line.endswith(" payload.bin")), None)
            results["cases"].append(result)
            print(json.dumps(result), flush=True)
    results["status"] = "bug reproduced" if args.baseline else "passed"
    if args.output:
        args.output.parent.mkdir(parents=True, exist_ok=True)
        args.output.write_text(json.dumps(results, indent=2) + "\n")


if __name__ == "__main__":
    main()
