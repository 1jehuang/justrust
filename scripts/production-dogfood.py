#!/usr/bin/env python3
"""Activate the service-selected published release, then verify a hosted build.

No compilation happens locally. This does not publish releases or change the
fleet selection. Run after release CI publishes the intended production tag.
"""
import argparse
import hashlib
import io
import json
import os
from pathlib import Path
import platform
import re
import shutil
import stat
import subprocess
import tarfile
import tempfile
import urllib.request

ENDPOINT = "https://build.jcode.sh/v1/release/latest"
ASSETS = "https://github.com/1jehuang/justrust/releases/download/"


def production_env():
    # Do not inherit development routing, updater URLs, compiler overrides, or
    # forced legacy transport. Retain credentials and a configured account home.
    env = {k: v for k, v in os.environ.items()
           if not k.startswith("JUSTRUST_") or k == "JUSTRUST_HOME"}
    env.update(JUSTRUST_REMOTE_REQUIRED="1", JUSTRUST_HOSTED_SESSION="1",
               JUSTRUST_NO_UPDATE="1")
    return env


def run(*args, cwd=None):
    result = subprocess.run(args, cwd=cwd, env=production_env(),
                            text=True, stdout=subprocess.PIPE)
    if result.returncode:
        print(result.stdout, end="", flush=True)
        result.check_returncode()
    return result.stdout


def fetch(url, limit):
    request = urllib.request.Request(url, headers={"User-Agent": "justrust-production-dogfood/1"})
    with urllib.request.urlopen(request, timeout=60) as response:
        data = response.read(limit + 1)
    if len(data) > limit:
        raise ValueError("download exceeded size limit")
    return data


def target():
    arch = {"x86_64": "x86_64", "aarch64": "aarch64"}.get(platform.machine())
    if platform.system() != "Linux" or not arch:
        raise ValueError("exact-byte hosted dogfood currently requires Linux x86_64/aarch64")
    return f"{arch}-unknown-linux-musl"


def release_info(expected, triple):
    if not re.fullmatch(r"v\d+\.\d+\.\d+(?:-[0-9A-Za-z.]+)?", expected):
        raise ValueError("expected release must be an exact v-prefixed version tag")
    info = json.loads(fetch(f"{ENDPOINT}?target={triple}", 65536))
    url = f"{ASSETS}{expected}/justrust-{triple}.tar.gz"
    if (info.get("tag") != expected or info.get("version") != expected[1:]
            or info.get("enabled") is not True or info.get("url") != url
            or info.get("sha256_url") != url + ".sha256"):
        raise ValueError("production service does not select the requested enabled release")
    return info


def verified_binary(info, triple):
    checksum = fetch(info["sha256_url"], 4096).decode().split()
    if not checksum or not re.fullmatch(r"[0-9a-fA-F]{64}", checksum[0]):
        raise ValueError("invalid published checksum")
    archive = fetch(info["url"], 128 * 1024 * 1024)
    if hashlib.sha256(archive).hexdigest() != checksum[0].lower():
        raise ValueError("release archive checksum mismatch")
    with tarfile.open(fileobj=io.BytesIO(archive), mode="r:gz") as bundle:
        members = [m for m in bundle.getmembers()
                   if m.name == f"justrust-{triple}/justrust"]
        if len(members) != 1 or not members[0].isfile() or members[0].size > 128 * 1024 * 1024:
            raise ValueError("release must contain one regular executable")
        # Never extract arbitrary paths, links, or archive permissions.
        return bundle.extractfile(members[0]).read()


def validate_release(binary, version):
    if run(str(binary), "--version").strip() != f"justrust {version}":
        raise ValueError("release executable version mismatch")
    status = run(str(binary), "update", "status")
    if "(release build)" not in status:
        raise ValueError("download is not marked as a release build")


def validate_run(output, cwd):
    rows = [json.loads(line) for line in output.splitlines() if line.strip()]
    matches = [row for row in rows if row.get("cwd") == str(cwd)
               and row.get("args", [None])[0:1] == ["test"]]
    if not matches or matches[-1].get("remote") != "hosted" or matches[-1].get("exit") != 0:
        raise ValueError("acceptance run was not a successful recorded hosted test")
    return matches[-1]["id"]


def smoke(binary, directory):
    directory.mkdir()
    (directory / "src").mkdir()
    (directory / "Cargo.toml").write_text(
        '[package]\nname = "production-dogfood-smoke"\nversion = "0.0.0"\nedition = "2021"\n')
    (directory / "src/lib.rs").write_text(
        '#[test]\nfn production_smoke() { assert_eq!(2 + 2, 4); }\n')
    # Explicitly select the real hosted backend. Durable sessions pin the exact
    # checksum-verified release executable, rather than an old mutable launcher.
    print(run(str(binary), "remote", "use", "hosted"), end="")
    print(run(str(binary), "test", "--remote", cwd=directory), end="")
    receipt = run(str(binary), "runs", "--here", "--json", "-n", "10", cwd=directory)
    return validate_run(receipt, directory)


def activate(binary, data):
    backup_dir = Path(os.environ.get("JUSTRUST_HOME", Path.home() / ".justrust")) / "production-backups"
    backup_dir.mkdir(parents=True, exist_ok=True, mode=0o700)
    old = binary.read_bytes()
    digest = hashlib.sha256(old).hexdigest()
    backup = backup_dir / f"justrust-{digest}"
    if not backup.exists():
        with backup.open("xb") as out:
            out.write(old)
        backup.chmod(0o700)
    fd, name = tempfile.mkstemp(prefix=".justrust-production-", dir=binary.parent)
    try:
        with os.fdopen(fd, "wb") as out:
            out.write(data)
            out.flush()
            os.fsync(out.fileno())
        os.chmod(name, stat.S_IMODE(binary.stat().st_mode) | stat.S_IXUSR)
        os.replace(name, binary)
    finally:
        if os.path.exists(name):
            os.unlink(name)
    return backup


def stale_proxies(binary, path=None):
    """`cargo` links on PATH that proxy to a different justrust executable.

    Plain `cargo` then runs another client build than `justrust`. Each build
    installs itself as the hosted agent and starts its own sync daemon, so the
    two keep replacing each other (seen 2026-11-12: 23 daemon restarts and 17
    agent reinstalls in three hours, alternating between the two builds).
    """
    path = os.environ.get("PATH", "") if path is None else path
    stale = []
    for directory in dict.fromkeys(filter(None, path.split(os.pathsep))):
        link = Path(directory) / "cargo"
        if not link.is_symlink():
            continue
        target = link.resolve()
        if target.name == "justrust" and target != binary:
            stale.append(link)
    return stale


def repoint(link, binary):
    fd, name = tempfile.mkstemp(prefix=".cargo-proxy-", dir=link.parent)
    os.close(fd)
    os.unlink(name)
    try:
        os.symlink(binary, name)
        os.replace(name, link)
    finally:
        if os.path.lexists(name):
            os.unlink(name)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--expect", required=True, help="exact published production tag")
    parser.add_argument("--activate", action="store_true", help="replace the PATH-selected client after verification")
    args = parser.parse_args()
    triple = target()
    info = release_info(args.expect, triple)
    selected = shutil.which("justrust")
    if not selected:
        raise ValueError("justrust must already be installed and signed in")
    binary = Path(selected).resolve()
    if not args.activate:
        validate_release(binary, args.expect[1:])
        if binary.read_bytes() != verified_binary(info, triple):
            raise ValueError("installed executable differs from production artifact, use --activate")
        stale = stale_proxies(binary)
        if stale:
            raise ValueError("cargo proxy runs a different justrust than PATH selects, use --activate: "
                             + ", ".join(f"{link} -> {link.resolve()}" for link in stale))
        print(f"Verified production artifact: {binary} ({args.expect})")
        return
    data = verified_binary(info, triple)
    scratch = os.environ.get("JCODE_SCRATCH_DIR")
    with tempfile.TemporaryDirectory(prefix="justrust-production-", dir=scratch) as work:
        candidate = Path(work) / "justrust"
        candidate.write_bytes(data)
        candidate.chmod(0o700)
        validate_release(candidate, args.expect[1:])
        smoke(candidate, Path(work) / "before-activation")
        release_info(args.expect, triple)
        backup = activate(binary, data)
        print(f"Activated {binary}. Previous executable retained at {backup}")
        for link in stale_proxies(binary):
            previous = link.resolve()
            repoint(link, binary)
            print(f"Repointed cargo proxy {link} from {previous} to {binary}")
        # Do not silently roll back after failure: leave the failure visible and
        # the backup available, with no claim that production acceptance passed.
        validate_release(binary, args.expect[1:])
        receipt = smoke(binary, Path(work) / "after-activation")
        release_info(args.expect, triple)
        print(f"Production dogfood passed: {args.expect}, sha256={hashlib.sha256(data).hexdigest()}, run={receipt}")
        print("Next: run real project workloads with this installed client.")


if __name__ == "__main__":
    main()
