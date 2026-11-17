#!/usr/bin/env python3
"""Provision ephemeral fixture Git identity from an exact pinned commit author.

No Git config is written. The shell exports apply only to the controlled runner
and runtime_context forwards only these names/emails into private HOME commands.
"""
import argparse
import re
import shlex
import subprocess
import sys


def pinned_author_env(repository, revision):
    if not re.fullmatch(r"[0-9a-f]{40}", revision):
        raise ValueError("fixture identity requires an exact full lowercase commit SHA")
    result = subprocess.run(
        ["git", "--no-replace-objects", "-C", str(repository), "show", "--no-patch", "--format=%H%x00%an%x00%ae", revision],
        capture_output=True, text=True, check=True)
    fields = result.stdout.rstrip("\n").split("\0")
    if (len(fields) != 3 or fields[0] != revision
            or any(not value.strip() or any(c in value for c in "\r\n\0") for value in fields[1:])):
        raise ValueError("pinned commit has invalid or missing fixture author identity")
    return {f"GIT_{role}_{field}": value
            for role in ("AUTHOR", "COMMITTER")
            for field, value in zip(("NAME", "EMAIL"), fields[1:])}


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("repository")
    parser.add_argument("revision")
    args = parser.parse_args()
    identity = pinned_author_env(args.repository, args.revision)
    print(f"fixture Git identity source: pinned commit {args.revision}", file=sys.stderr)
    for key, value in identity.items():
        print(f"export {key}={shlex.quote(value)}")


if __name__ == "__main__":
    main()
