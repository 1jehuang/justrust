#!/usr/bin/env python3
"""Materialize optional upstream sources, never the owned compiler/Cargo trees.

Only git-archive content is extracted. Originals are read-only inputs and no Git
metadata, ignored build products, or nested contribution policies are imported.
"""
import argparse
import json
import os
from pathlib import Path, PurePosixPath
import re
import subprocess
import tarfile
import tempfile

ROOT = Path(__file__).resolve().parents[1]
MANIFEST = ROOT / "toolchain/sources.json"
POLICY_NAMES = {
    "agents.md", "claude.md", "justrust.md", "contributing.md",
    "code_of_conduct.md", "ai_policy.md", "pull_request_template.md",
    ".gitmodules",
}


def is_policy(path):
    return PurePosixPath(path).name.lower() in POLICY_NAMES


def git(repo, *args):
    return subprocess.check_output(["git", "-C", str(repo), *args], text=True).strip()


def materialize(spec, root=ROOT, from_repo=None, cache=None):
    """Export an exact commit into a previously absent or empty directory."""
    revision = spec["commit"]
    if not re.fullmatch(r"[0-9a-f]{40}", revision):
        raise ValueError("source commit must be a full lowercase Git SHA")
    root = Path(root).resolve()
    destination = root / spec["path"]
    if not destination.resolve().is_relative_to(root / "toolchain/rustc"):
        raise ValueError("optional source path escapes toolchain/rustc")
    if destination.is_symlink() or (destination.exists() and any(destination.iterdir())):
        raise ValueError(f"refusing to overwrite nonempty source tree: {destination}")
    cache = Path(cache or os.environ.get("JCODE_SCRATCH_DIR") or
                 Path(os.environ.get("JUSTRUST_HOME", Path.home() / ".justrust")) /
                 "toolchain-source-cache")
    cache.mkdir(parents=True, exist_ok=True)
    with tempfile.TemporaryDirectory(prefix="source-", dir=cache) as temporary:
        temporary = Path(temporary)
        repository = Path(from_repo).resolve() if from_repo else temporary / "git"
        if from_repo is None:
            subprocess.run(["git", "init", "--bare", "--quiet", str(repository)], check=True)
            subprocess.run(["git", "-C", str(repository), "fetch", "--depth=1", "--no-tags",
                            spec["origin"], revision], check=True)
        actual = git(repository, "rev-parse", f"{revision}^{{commit}}")
        if actual != revision:
            raise ValueError(f"expected {revision}, got {actual}")
        # Nested Git dependencies must not silently become empty source directories.
        links = [line for line in git(repository, "ls-tree", "-r", revision).splitlines()
                 if line.startswith("160000 ")]
        if links:
            raise ValueError("optional source contains nested gitlinks requiring explicit handling: "
                             + ", ".join(line.split("\t", 1)[1] for line in links))
        archive = temporary / "source.tar"
        with archive.open("wb") as output:
            subprocess.run(["git", "-C", str(repository), "archive", revision],
                           stdout=output, check=True)
        staged = temporary / "source"
        staged.mkdir()
        with tarfile.open(archive) as tar:
            for member in tar:
                if not is_policy(member.name):
                    tar.extract(member, staged, filter="data")
        info = git(repository, "show", "-s", "--format=%H%n%h%n%cs", "--abbrev=9", revision)
        (staged / "git-commit-info").write_text(info + "\n")
        destination.parent.mkdir(parents=True, exist_ok=True)
        if destination.exists():
            destination.rmdir()
        # Atomic on the destination filesystem, even if the download cache is elsewhere.
        with tempfile.TemporaryDirectory(prefix=".source-", dir=destination.parent) as local:
            import shutil
            complete = Path(local) / "complete"
            shutil.copytree(staged, complete, symlinks=True)
            complete.rename(destination)
    return destination


def main():
    manifest = json.loads(MANIFEST.read_text())
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("source", choices=[item["name"] for item in manifest["optional"]] + ["list"])
    parser.add_argument("--from-repo", type=Path, help="read an existing repository instead of fetching")
    args = parser.parse_args()
    if args.source == "list":
        for spec in manifest["optional"]:
            present = (ROOT / spec["path"] / "git-commit-info").is_file()
            print(f'{spec["name"]:16} {spec["commit"]} {"present" if present else "absent"}: {spec["purpose"]}')
        return
    spec = next(item for item in manifest["optional"] if item["name"] == args.source)
    print(materialize(spec, from_repo=args.from_repo))


if __name__ == "__main__":
    main()
