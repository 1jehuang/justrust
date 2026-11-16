#!/usr/bin/env python3
"""Promote only complete releases for legacy GitHub-latest installers.

Per-platform clients use the target-aware service independently. This operation
changes only latest metadata, never tags or published assets.
"""
import argparse
import json
import re
import subprocess
from urllib.parse import quote

TARGETS = ("x86_64-unknown-linux-musl", "aarch64-unknown-linux-musl",
           "x86_64-apple-darwin", "aarch64-apple-darwin", "x86_64-pc-windows-msvc")


def command(*args):
    return subprocess.check_output(["gh", *args], text=True)


def version(tag):
    match = re.fullmatch(r"v(\d+)\.(\d+)\.(\d+)(?:-([0-9A-Za-z.]+))?", tag)
    if not match:
        raise ValueError("invalid release version")
    prerelease = match[4]
    identifiers = tuple((0, int(part)) if part.isdigit() else (1, part)
                        for part in prerelease.split(".")) if prerelease else ()
    return tuple(map(int, match.group(1, 2, 3))), prerelease is None, identifiers


def promote(tag, sha, repo):
    version(tag)
    if not re.fullmatch(r"[0-9a-f]{40}", sha) or not re.fullmatch(r"[A-Za-z0-9_-]+/[A-Za-z0-9_.-]+", repo):
        raise ValueError("invalid source SHA or repository")
    ref = quote("refs/tags/" + tag, safe="")
    def check_tag():
        actual = json.loads(command("api", f"repos/{repo}/commits/{ref}"))
        if actual.get("sha") != sha:
            raise ValueError("release tag moved")
    check_tag()
    release = json.loads(command("api", f"repos/{repo}/releases/tags/{quote(tag, safe='')}"))
    if release.get("draft") is not False or release.get("tag_name") != tag:
        raise ValueError("release must be published")
    assets = {asset.get("name") for asset in release.get("assets", []) if asset.get("state") == "uploaded"}
    for target in TARGETS:
        archive = f"justrust-{target}.tar.gz"
        if not {archive, archive + ".sha256"} <= assets:
            raise ValueError("missing platform: " + target)
        if not any(f"compiler-{target}.{ext}" in assets for ext in ("json", "txt")):
            raise ValueError("missing compiler provenance: " + target)
    latest = json.loads(command("api", f"repos/{repo}/releases/latest"))
    if version(latest["tag_name"]) >= version(tag):
        print("Global latest already names this or a newer version; no promotion needed.")
        return
    check_tag()
    command("release", "edit", tag, "--latest", "--repo", repo)
    print("All platforms ready; promoted legacy installer latest to " + tag)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    for name in ("tag", "sha", "repo"):
        parser.add_argument("--" + name, required=True)
    args = parser.parse_args()
    promote(args.tag, args.sha, args.repo)


if __name__ == "__main__":
    main()
