#!/usr/bin/env python3
"""Append one verified platform to an existing release, checksum last.

Uses only Python's standard library and authenticated gh. Never creates releases,
replaces assets, changes immutability, or moves tags. The checksum is the consumer
readiness boundary, so it is uploaded only after the other assets are verified.
"""
import argparse
import hashlib
import json
import os
from pathlib import Path
import re
import stat
import subprocess
import sys
import tempfile
from urllib.parse import quote

TARGETS = (
    "x86_64-unknown-linux-musl", "aarch64-unknown-linux-musl",
    "x86_64-apple-darwin", "aarch64-apple-darwin", "x86_64-pc-windows-msvc",
)


class PublishError(Exception):
    pass


def gh(*args):
    result = subprocess.run(["gh", *args], capture_output=True)
    if result.returncode:
        raise PublishError("gh command failed: " + " ".join(args) + ": "
                           + result.stderr.decode("utf-8", "replace").strip())
    return result.stdout


def api(endpoint):
    return json.loads(gh("api", endpoint))


def validate_args(args):
    if (not re.fullmatch(r"v[0-9][A-Za-z0-9._+-]*", args.tag)
            or ".." in args.tag or args.tag.endswith((".", ".lock"))):
        raise PublishError("invalid release tag")
    if not re.fullmatch(r"[0-9a-f]{40}", args.sha):
        raise PublishError("sha must be a full lowercase 40-character commit SHA")
    if not re.fullmatch(r"[A-Za-z0-9][A-Za-z0-9-]*/[A-Za-z0-9][A-Za-z0-9._-]*", args.repo):
        raise PublishError("repo must be OWNER/REPO")
    if args.target not in TARGETS:
        raise PublishError("unsupported target")


def local_assets(args):
    if ".." in Path(args.dist).parts:
        raise PublishError("dist path must not contain traversal components")
    directory = Path(os.path.abspath(args.dist))
    for part in [directory, *directory.parents]:
        if part.is_symlink():
            raise PublishError("dist and its ancestors must not be symlinks")
    if not directory.is_dir():
        raise PublishError("dist must be a directory")
    archive = f"justrust-{args.target}.tar.gz"
    checksum = archive + ".sha256"
    companions = {f"compiler-{args.target}.json", f"compiler-{args.target}.txt"}
    names = {p.name for p in directory.iterdir()}
    provenance = names & companions
    if len(provenance) != 1 or names != {archive, checksum} | provenance:
        raise PublishError("dist must contain only archive, checksum, and exactly one compiler provenance companion")
    contents = {}
    for name in sorted(names):
        path = directory / name
        before = path.lstat()
        if not stat.S_ISREG(before.st_mode):
            raise PublishError(f"asset is not a regular non-symlink file: {name}")
        fd = os.open(path, os.O_RDONLY | getattr(os, "O_NOFOLLOW", 0) | getattr(os, "O_BINARY", 0))
        with os.fdopen(fd, "rb") as stream:
            opened = os.fstat(stream.fileno())
            if not stat.S_ISREG(opened.st_mode) or (before.st_dev, before.st_ino) != (opened.st_dev, opened.st_ino):
                raise PublishError(f"asset changed during validation: {name}")
            contents[name] = stream.read()
        if not contents[name]:
            raise PublishError(f"empty asset: {name}")
    expected = hashlib.sha256(contents[archive]).hexdigest()
    try:
        text = contents[checksum].decode("ascii")
    except UnicodeDecodeError as exc:
        raise PublishError("checksum must be ASCII") from exc
    if not re.fullmatch(re.escape(expected) + r" [ *]" + re.escape(archive) + r"\n?", text):
        raise PublishError("checksum must match the archive bytes and exact archive basename")
    return [archive, next(iter(provenance)), checksum], contents


def check_tag(args):
    current = api(f"repos/{args.repo}/commits/{quote('refs/tags/' + args.tag, safe='')}")
    if current.get("sha") != args.sha:
        raise PublishError("release tag moved or does not match pinned SHA")


def release(args):
    data = api(f"repos/{args.repo}/releases/tags/{quote(args.tag, safe='')}")
    if data.get("immutable") is True:
        raise PublishError("GitHub release is immutable; cannot append platform assets. No settings were changed.")
    if data.get("tag_name") != args.tag or not isinstance(data.get("draft"), bool):
        raise PublishError("invalid release metadata")
    if not isinstance(data.get("assets"), list):
        raise PublishError("release assets missing from metadata")
    return data


def existing_assets(data, contents, args):
    found = {}
    for asset in data["assets"]:
        name = asset.get("name")
        if name not in contents:
            continue
        if name in found:
            raise PublishError(f"duplicate existing asset: {name}")
        digest = asset.get("digest")
        expected = "sha256:" + hashlib.sha256(contents[name]).hexdigest()
        if isinstance(digest, str) and digest.startswith("sha256:"):
            matches = digest == expected
        else:
            asset_id = asset.get("id")
            if not isinstance(asset_id, int) or isinstance(asset_id, bool) or asset_id <= 0:
                raise PublishError(f"invalid existing asset id: {name}")
            remote = gh("api", f"repos/{args.repo}/releases/assets/{asset_id}",
                        "-H", "Accept: application/octet-stream")
            matches = remote == contents[name]
        if not matches:
            raise PublishError(f"existing asset bytes differ: {name}; left untouched")
        found[name] = asset
    return found


def publish(args):
    validate_args(args)
    order, contents = local_assets(args)
    check_tag(args)
    data = release(args)
    found = existing_assets(data, contents, args)
    with tempfile.TemporaryDirectory(prefix="justrust-release-") as temporary:
        for name in order:
            if name in found:
                continue
            # Refresh before each append to detect concurrent retries and changes.
            check_tag(args)
            found = existing_assets(release(args), contents, args)
            if name in found:
                continue
            path = Path(temporary) / name
            path.write_bytes(contents[name])
            try:
                gh("release", "upload", args.tag, str(path), "--repo", args.repo)
            except PublishError:
                # Another invocation may have won the same-name race. Never clobber.
                if name not in existing_assets(release(args), contents, args):
                    raise
            if name not in existing_assets(release(args), contents, args):
                raise PublishError(f"uploaded asset absent: {name}")
    check_tag(args)
    data = release(args)
    found = existing_assets(data, contents, args)
    if set(found) != set(contents):
        raise PublishError("platform asset set incomplete")
    if data["draft"]:
        check_tag(args)
        # Partial releases must not become global Latest: legacy installers use
        # releases/latest/download and need every target. The all-platform
        # summary owns promotion only after verifying the complete release.
        gh("release", "edit", args.tag, "--draft=false", "--latest=false", "--repo", args.repo)
    check_tag(args)
    if release(args)["draft"]:
        raise PublishError("release remains draft after publication")
    print(f"Published {args.tag} platform {args.target} (append-only, checksum ready)")


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    for name in ("tag", "sha", "target", "dist", "repo"):
        parser.add_argument("--" + name, required=True)
    args = parser.parse_args()
    try:
        publish(args)
    except (PublishError, OSError, ValueError) as exc:
        print(f"platform release failed: {exc}", file=sys.stderr)
        return 1
    return 0


if __name__ == "__main__":
    sys.exit(main())
