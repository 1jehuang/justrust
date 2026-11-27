#!/usr/bin/env python3
"""Keep slow optional platform runners from holding up client releases.

`wait-linux` lets post-publication tests start once this run's Linux builds
have published, without waiting for macOS or Windows runners.

`supersede` cancels older Release runs that are doing nothing but waiting in
a runner queue for optional platforms. It never cancels a run with work in
progress, and it changes no tag, asset, or latest pointer.
"""
import argparse
import json
import re
import subprocess
import time

LINUX = ("x86_64-unknown-linux-musl", "aarch64-unknown-linux-musl")
OPTIONAL = ("x86_64-apple-darwin", "aarch64-apple-darwin", "x86_64-pc-windows-msvc")
ACTIVE = ("queued", "in_progress", "waiting", "pending", "requested")


def gh(*args):
    return subprocess.check_output(["gh", *args], text=True)


def version(tag):
    match = re.fullmatch(r"v(\d+)\.(\d+)\.(\d+)", tag or "")
    return tuple(map(int, match.groups())) if match else None


def linux_state(jobs, expected):
    """Return "ready", "wait", or a failure message for the expected builds."""
    by_name = {job["name"]: job for job in jobs}
    waiting = False
    for target in expected:
        job = by_name.get("build " + target)
        if job is None or job["status"] != "completed":
            waiting = True
        elif job["conclusion"] != "success":
            return f"build {target} concluded {job['conclusion']}"
    return "wait" if waiting else "ready"


def only_queued_optional(jobs):
    """True when every unfinished job is an optional build still in a queue."""
    unfinished = [job for job in jobs if job["status"] != "completed"]
    optional = {"build " + target for target in OPTIONAL}
    return bool(unfinished) and all(
        job["name"] in optional and job["status"] == "queued" for job in unfinished)


def superseded(runs, tag, run_id, jobs_of):
    """Older Release runs that hold nothing but queued optional builds."""
    mine = version(tag)
    if mine is None:
        return []
    result = []
    for run in runs:
        theirs = version(run.get("headBranch"))
        if run["databaseId"] == run_id or theirs is None or theirs >= mine:
            continue
        if run["status"] in ACTIVE and only_queued_optional(jobs_of(run["databaseId"])):
            result.append(run["databaseId"])
    return result


def run_jobs(repo, run_id):
    text = gh("api", "--paginate", "--slurp",
              f"repos/{repo}/actions/runs/{run_id}/jobs?per_page=100")
    return [job for page in json.loads(text) for job in page["jobs"]]


def wait_linux(repo, run_id, expected, timeout, poll, clock=time.monotonic, sleep=time.sleep):
    deadline = clock() + timeout
    while True:
        try:
            state = linux_state(run_jobs(repo, run_id), expected)
        except subprocess.CalledProcessError as error:
            state = "wait"
            print(f"job listing failed ({error.returncode}); retrying")
        if state == "ready":
            print("Linux builds published: " + ", ".join(expected))
            return
        if state != "wait":
            raise SystemExit(state)
        if clock() >= deadline:
            raise SystemExit("timed out waiting for the Linux builds")
        sleep(poll)


def supersede(repo, tag, run_id):
    runs = json.loads(gh("run", "list", "--repo", repo, "--workflow", "release.yml", "--limit", "50",
                         "--json", "databaseId,headBranch,status"))
    for old in superseded(runs, tag, run_id, lambda identifier: run_jobs(repo, identifier)):
        print(f"cancelling Release run {old}: only queued optional platform builds remain")
        subprocess.run(["gh", "run", "cancel", str(old), "--repo", repo], check=False)


def main():
    parser = argparse.ArgumentParser()
    commands = parser.add_subparsers(dest="command", required=True)
    waiter = commands.add_parser("wait-linux")
    waiter.add_argument("--targets", nargs="+", default=list(LINUX), choices=LINUX)
    waiter.add_argument("--timeout", type=int, default=5400)
    waiter.add_argument("--poll", type=int, default=20)
    canceller = commands.add_parser("supersede")
    canceller.add_argument("--tag", required=True)
    for command in (waiter, canceller):
        command.add_argument("--repo", required=True)
        command.add_argument("--run-id", type=int, required=True)
    args = parser.parse_args()
    if args.command == "wait-linux":
        wait_linux(args.repo, args.run_id, args.targets, args.timeout, args.poll)
    else:
        supersede(args.repo, args.tag, args.run_id)


if __name__ == "__main__":
    main()
