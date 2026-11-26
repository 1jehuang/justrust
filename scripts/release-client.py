#!/usr/bin/env python3
"""Publish an exact committed client tag, then activate its production bytes.

No client is compiled here. Full post-publication regression jobs are not on
this command's critical path. Without --execute this prints the release plan.
"""
import argparse
import importlib.util
import json
from pathlib import Path
import re
import subprocess
import sys
import time
import tomllib

ROOT = Path(__file__).resolve().parents[1]
REPO = "1jehuang/justrust"
TAG = re.compile(r"v\d+\.\d+\.\d+(?:-[0-9A-Za-z.]+)?\Z")


def run(*args):
    result = subprocess.run(args, cwd=ROOT, text=True, stdout=subprocess.PIPE)
    if result.returncode:
        print(result.stdout, end="", flush=True)
        result.check_returncode()
    return result.stdout.strip()


def candidate(tag, ref):
    if not TAG.fullmatch(tag):
        raise ValueError("tag must be an exact v-prefixed version")
    sha = run("git", "rev-parse", "--verify", "--end-of-options", ref + "^{commit}")
    run("git", "merge-base", "--is-ancestor", sha, "origin/stable")
    manifest = tomllib.loads(run("git", "show", sha + ":Cargo.toml"))
    if tag != "v" + manifest["package"]["version"]:
        raise ValueError("tag must match the committed Cargo.toml version")
    return sha


def remote_tag(tag):
    rows = run("git", "ls-remote", "--tags", "origin", "refs/tags/" + tag,
               "refs/tags/" + tag + "^{}")
    refs = dict((ref, sha) for sha, ref in (line.split() for line in rows.splitlines()))
    return refs.get("refs/tags/" + tag + "^{}", refs.get("refs/tags/" + tag))


def ensure_tag(tag, sha):
    remote = remote_tag(tag)
    if remote:
        if remote != sha:
            raise ValueError("remote tag already names a different commit, refusing to move it")
        print("Existing remote tag is unchanged. Waiting for its publication.", flush=True)
        return
    local = subprocess.run(["git", "rev-parse", "--verify", "refs/tags/" + tag],
                           cwd=ROOT, text=True, stdout=subprocess.PIPE, stderr=subprocess.DEVNULL)
    if local.returncode == 0:
        object_id = local.stdout.strip()
    else:
        run("git", "tag", tag, sha)
        object_id = run('git', 'rev-parse', '--verify', 'refs/tags/' + tag)
    if object_id != sha and run('git', 'rev-parse', '--verify', object_id + '^{commit}') != sha:
        raise ValueError("local tag already names a different commit, refusing to move it")
    # Push the verified object, not a local tag that could move while queued.
    # The immutable object also preserves an existing annotated tag.
    run("git", "push", "origin", object_id + ":refs/tags/" + tag)


def release_runs(tag, sha):
    rows = json.loads(run("gh", "run", "list", "--repo", REPO, "--workflow", "release.yml",
                          "--event", "push", "--commit", sha, "--limit", "30", "--json",
                          "databaseId,headBranch,headSha,status,conclusion,url"))
    return [row for row in rows if row["headBranch"] == tag and row["headSha"] == sha]


def platform_failed(release_run, triple):
    """Only this platform or shared preflight can block local publication."""
    result = json.loads(run("gh", "run", "view", str(release_run["databaseId"]),
                            "--repo", REPO, "--json", "jobs"))
    critical = {"preflight", "build " + triple}
    return any(job["name"] in critical and job["status"] == "completed"
               and job["conclusion"] != "success" for job in result["jobs"])


def production_module():
    spec = importlib.util.spec_from_file_location("production_dogfood", ROOT / "scripts/production-dogfood.py")
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module


def wait_for_publication(tag, sha, timeout, poll, production, clock=time.monotonic, sleep=time.sleep):
    deadline = clock() + timeout
    triple = production.target()
    while True:
        # Recheck tag identity throughout the wait, not just before publication.
        try:
            observed = remote_tag(tag)
        except subprocess.CalledProcessError as error:
            remaining = deadline - clock()
            if remaining <= 0:
                raise TimeoutError("could not verify remote release tag before publication deadline") from error
            print("Remote tag lookup unavailable; retrying without activating.", flush=True)
            sleep(min(poll, remaining))
            continue
        if observed != sha:
            raise ValueError("release tag moved or disappeared while waiting")
        try:
            production.release_info(tag, triple)
            print("Exact published release is selected by production.", flush=True)
            return
        except (ValueError, OSError) as error:
            message = str(error)
        # The workflow is asked only so a failed build ends the wait early.
        # Publication is decided above, so an unavailable API (v0.1.65: one
        # HTTP 502 from GitHub ended the wait before activation) is retried.
        try:
            runs = release_runs(tag, sha)
            # A workflow's full acceptance can continue after publication. We never
            # wait for overall success before accepting service-selected bytes.
            failed = (runs and runs[0]["status"] == "completed" and runs[0]["conclusion"] != "success"
                      and platform_failed(runs[0], triple))
        except subprocess.CalledProcessError:
            print("Release workflow status unavailable; still waiting for publication.", flush=True)
            failed = False
        if failed:
            raise RuntimeError("release preflight or local platform failed before production selection: " + runs[0]["url"])
        if clock() >= deadline:
            raise TimeoutError("publication/production selection timed out: " + message)
        print("Waiting for published production client: " + message, flush=True)
        sleep(min(poll, max(0, deadline - clock())))


def main(argv=None):
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--tag", required=True, help="exact version matching committed Cargo.toml")
    parser.add_argument("--ref", default="HEAD", help="committed source to release, default HEAD")
    parser.add_argument('--test-exact', action='store_true', help='test a full --ref SHA on hosted, even without --execute; never tag on gate failure')
    parser.add_argument("--execute", action="store_true", help="push immutable tag, wait, and activate published bytes")
    parser.add_argument("--timeout", type=float, default=1800, help="publication deadline in seconds")
    parser.add_argument("--poll", type=float, default=10, help="publication polling interval in seconds")
    args = parser.parse_args(argv)
    if not 0 < args.timeout <= 7200 or not 0 < args.poll <= 300:
        parser.error("timeout must be 0..7200 seconds and poll 0..300 seconds")
    if args.test_exact and not re.fullmatch('[0-9a-f]{40}', args.ref):
        parser.error('--test-exact requires --ref with a full lowercase commit SHA, not a moving ref')
    sha = candidate(args.tag, args.ref)
    if args.test_exact:
        from exact_client_gate import gate
        if sha != args.ref or gate(ROOT, sha) != sha:
            raise ValueError('exact gate did not verify requested release SHA')
        if not args.execute:
            print('Gate only. No tag pushed or executable activated.')
            return
    print(f"Release plan: {args.tag} from {sha}")
    print("Only committed source is released. Uncommitted edits are never packaged.")
    print("Tag push -> existing Release builds/sanity checks -> shared publication -> exact-byte local activation.")
    if not args.execute:
        print("Dry run. Add --execute to publish and activate. No tag or executable changed.")
        return
    origin = run("git", "remote", "get-url", "--push", "origin")
    if origin not in {"https://github.com/1jehuang/justrust.git", "https://github.com/1jehuang/justrust",
                       "git@github.com:1jehuang/justrust.git", "ssh://git@github.com/1jehuang/justrust.git"}:
        raise ValueError("origin must be the official justrust repository before publishing")
    run("gh", "auth", "status")
    production = production_module()
    production.target()  # Refuse unsupported local activation before pushing a tag.
    ensure_tag(args.tag, sha)
    wait_for_publication(args.tag, sha, args.timeout, args.poll, production)
    # Reuse checksum validation, pre-activation hosted smoke, backup and atomic
    # activation. Never install a source build or rebuild after publication.
    print(run(sys.executable, str(ROOT / "scripts/production-dogfood.py"), "--expect", args.tag, "--activate"), flush=True)
    print(f"Published production client {args.tag} is now active locally.")
    print("Full published-client acceptance continues in GitHub Actions, independently.")


if __name__ == "__main__":
    main()
