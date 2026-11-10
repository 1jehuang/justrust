#!/usr/bin/env python3
"""Targeted checks and serialized validation/publication of committed Rust trees."""

import argparse
import contextlib
import fcntl
import os
from pathlib import Path
import re
import shutil
import stat
import subprocess
import sys
import tarfile
import tempfile

DEFAULT_LOCK = "/home/ec2-user/.jcode/scratch/fish-milestones/build.lock"
MAX_SNAPSHOT_BYTES = 256 * 1024 * 1024
MAX_SNAPSHOT_FILE_BYTES = 32 * 1024 * 1024
EXCLUDED_PARTS = {".git", "target", "__pycache__", ".cache", ".pytest_cache",
                  ".mypy_cache", ".ruff_cache", "node_modules"}


def snapshot(repo, destination):
    """Copy listed working-tree inputs, never the index or Git object contents.

    Capture happens before the first build-lock acquisition. Deleted tracked
    files stay deleted, dirty and nonignored untracked inputs stay dirty.
    Generated history and caches are not compiler inputs. Unexpected large
    inputs fail explicitly instead of silently creating an incomplete tree.
    """
    names = subprocess.check_output(
        ["git", "ls-files", "-z", "--cached", "--others", "--exclude-standard"], cwd=repo)
    paths = sorted({Path(os.fsdecode(name)) for name in names.split(b"\0") if name})
    total = 0
    links = []
    for relative in paths:
        if relative.is_absolute() or ".." in relative.parts:
            raise ValueError(f"unsafe source path: {relative}")
        if EXCLUDED_PARTS.intersection(relative.parts) or relative.parts[:2] in (
                ("data", "runs"), ("experiments", "results")):
            continue
        source = repo / relative
        try:
            info = source.lstat()
        except FileNotFoundError:
            continue
        # Reject external symlinks, including changed symlink parent paths.
        if not source.resolve().is_relative_to(repo):
            raise ValueError(f"source symlink escapes repository: {relative}")
        if stat.S_ISLNK(info.st_mode):
            target = os.readlink(source)
            if Path(target).is_absolute():
                raise ValueError(f"absolute source symlink is not relocatable: {relative}")
            links.append((relative, target))
            continue
        if not stat.S_ISREG(info.st_mode):
            raise ValueError(f"source input is not a regular file: {relative}")
        if info.st_size > MAX_SNAPSHOT_FILE_BYTES or total + info.st_size > MAX_SNAPSHOT_BYTES:
            raise ValueError(f"source snapshot exceeds size budget at {relative}. Ignore generated artifacts first")
        with source.open("rb") as stream:
            contents = stream.read(min(MAX_SNAPSHOT_FILE_BYTES, MAX_SNAPSHOT_BYTES - total) + 1)
        total += len(contents)
        if len(contents) > MAX_SNAPSHOT_FILE_BYTES or total > MAX_SNAPSHOT_BYTES:
            raise ValueError(f"source input grew past snapshot size budget: {relative}")
        output = destination / relative
        output.parent.mkdir(parents=True, exist_ok=True)
        output.write_bytes(contents)
        shutil.copystat(source, output)
    # Create links last, so copying a later file can never traverse a link.
    for relative, target in links:
        output = destination / relative
        output.parent.mkdir(parents=True, exist_ok=True)
        output.symlink_to(target)
    print(f"Captured target inputs before build queue ({total} bytes)", flush=True)


@contextlib.contextmanager
def locked(path):
    path = Path(path).expanduser().resolve()
    path.parent.mkdir(parents=True, exist_ok=True)
    # The inode is shared by all clients. Never unlink or replace this file.
    with path.open("a") as stream:
        fcntl.flock(stream, fcntl.LOCK_EX)
        try:
            yield
        finally:
            fcntl.flock(stream, fcntl.LOCK_UN)


def run(command, cwd, capture=False):
    print("+ " + " ".join(map(str, command)), flush=True)
    result = subprocess.run(command, cwd=cwd, check=True, text=True,
                            stdout=subprocess.PIPE if capture else None)
    return result.stdout.strip() if capture else None


def hosted(command, cwd, lock):
    with locked(lock):
        run(command, cwd)


def target_commands(args, filtered=True):
    for package in args.package or [None]:
        prefix = ["justrust", "test"]
        if package:
            prefix += ["--package", package]
        selectors = [["--test", target] for target in args.test]
        if args.unit:
            selectors.append(["--lib"])
        for selector in selectors:
            command = prefix + selector
            if filtered and args.filter:
                command.append(args.filter)
            yield command


def targets(args, cwd):
    for command in target_commands(args):
        hosted(command, cwd, args.lock)


def publish(args, repo):
    publication_lock = args.publication_lock or str(Path(args.lock).resolve()) + ".publication"
    if Path(publication_lock).expanduser().resolve() == Path(args.lock).expanduser().resolve():
        raise ValueError("publication and build locks must be separate")
    with locked(publication_lock):
        candidate = run(["git", "rev-parse", "--verify", args.candidate + "^{commit}"], repo, True)
        if candidate != args.candidate.lower():
            raise ValueError("candidate must name the exact commit, not a tag or tree")
        head = run(["git", "rev-parse", "HEAD"], repo, True)
        print(f"Validating committed candidate {candidate} (HEAD {head})", flush=True)
        if head != candidate:
            print("NOTE: candidate differs from HEAD. Only the candidate will be validated/pushed.", flush=True)
        # Refresh the remote baseline before spending time on validation. A later
        # race is handled by the normal non-forced push, never by moving HEAD.
        run(["git", "fetch", "--no-tags", "origin", "refs/heads/main"], repo)
        remote = run(["git", "rev-parse", "FETCH_HEAD"], repo, True)
        run(["git", "merge-base", "--is-ancestor", remote, candidate], repo)
        with tempfile.TemporaryDirectory(prefix="justrust-publish-", dir=args.scratch_dir) as temporary:
            temporary = Path(temporary)
            archive = temporary / "candidate.tar"
            tree = temporary / "tree"
            tree.mkdir()
            run(["git", "archive", "--format=tar", "--output=" + str(archive), candidate], repo)
            with tarfile.open(archive) as source:
                source.extractall(tree, filter="data")
            targets(args, tree)
            commands = [
                ["cargo", "fmt", "--all", "--check"],
                ["justrust", "test", "--test", "graph_integrity"],
                ["justrust", "test", "--test", "full_programs"],
                ["justrust", "test", "--workspace", "--no-run"],
            ]
            # Full touched suites must pass even when targeted filters passed.
            commands.extend(target_commands(args, filtered=False))
            if args.lib or args.unit:
                commands.append(["justrust", "test", "--workspace", "--lib"])
            commands.append(["justrust", "clippy", "--workspace", "--all-targets", "--", "-D", "warnings"])
            for command in commands:
                hosted(command, tree, args.lock)
            if args.push:
                run(["git", "push", "origin", candidate + ":refs/heads/main"], repo)
                observed = run(["git", "ls-remote", "--exit-code", "origin", "refs/heads/main"], repo, True)
                if observed.split() != [candidate, "refs/heads/main"]:
                    raise ValueError(f"remote main differs from pushed candidate {candidate}: {observed}")
            else:
                print(f"Validation only: {candidate}. No push requested.", flush=True)


def main(argv=None):
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("mode", choices=["target", "publish"])
    parser.add_argument("--repo", default=".")
    parser.add_argument("--lock", default=DEFAULT_LOCK)
    parser.add_argument("--publication-lock")
    parser.add_argument("--scratch-dir", help="parent for temporary committed-tree extraction")
    parser.add_argument("--test", action="append", default=[], help="touched integration test target, repeatable")
    parser.add_argument("--package", action="append", default=[], help="bounded Cargo package name, repeatable; defaults to root package")
    parser.add_argument("--unit", action="store_true", help="target package library unit tests; publish also requires all workspace library tests")
    parser.add_argument("--filter", help="single bounded Rust test-name filter")
    parser.add_argument("--candidate", help="full immutable commit hash, required for publish")
    parser.add_argument("--lib", action="store_true", help="also gate shared-type/library changes with --lib")
    parser.add_argument("--push", action="store_true", help="explicitly publish validated hash to origin/main")
    args = parser.parse_args(argv)
    if not args.test and not args.unit:
        parser.error("provide --test or --unit")
    if len(args.test) > 32 or any(not re.fullmatch(r"[A-Za-z0-9_][A-Za-z0-9_-]{0,127}", name) for name in args.test):
        parser.error("provide at most 32 bounded integration target names")
    args.test = list(dict.fromkeys(args.test))
    if len(args.package) > 32 or any(not re.fullmatch(r"[A-Za-z0-9_][A-Za-z0-9_-]{0,127}", name) for name in args.package):
        parser.error("provide at most 32 bounded package names")
    args.package = list(dict.fromkeys(args.package))
    if args.filter is not None and (not args.filter or len(args.filter) > 256 or args.filter.startswith("-") or any(c.isspace() for c in args.filter)):
        parser.error("filter must be a non-option test-name substring of at most 256 characters")
    if args.mode == "publish" and not re.fullmatch(r"[a-fA-F0-9]{40}|[a-fA-F0-9]{64}", args.candidate or ""):
        parser.error("publish requires --candidate with a full commit hash")
    if args.mode == "target" and (args.push or args.candidate or args.lib):
        parser.error("--push, --candidate and --lib are publication-only")
    try:
        repo = Path(run(["git", "rev-parse", "--show-toplevel"], args.repo, True))
        if args.mode == "target":
            with tempfile.TemporaryDirectory(prefix="justrust-target-", dir=args.scratch_dir) as temporary:
                tree = Path(temporary)
                snapshot(repo, tree)
                targets(args, tree)
        else:
            publish(args, repo)
    except (subprocess.CalledProcessError, OSError, ValueError, RuntimeError, tarfile.TarError) as error:
        print(f"Workflow stopped: {error}", file=sys.stderr)
        return 1
    return 0


if __name__ == "__main__":
    sys.exit(main())
