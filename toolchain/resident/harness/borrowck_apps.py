#!/usr/bin/env python3
"""borrowck_apps.py: `ra-jit borrowck --project` over the applications of apps/apps.json.

usage: borrowck_apps.py [NAME...] [--bin ra-jit] [--apps-dir dir] [--jobs N] [--json out.json]

Every application there is code rustc accepts, so each borrow-check error the checker reports
in one is a false one, and a file it panics on has none of its errors reported. Each app is
cloned at its `rev` (or taken from `apps/<source>`), given what `cargo metadata` needs that the
clone lacks (its `prepare` module's `sources`), checked, and its errors and panics printed by
app; `ra-jit borrowck --project` refuses a workspace `cargo metadata` fails on. Exits 1 when any app has one, or could not be checked.
"""
import argparse, asyncio, collections, importlib, json, os, sys, time
from typing import Any

HERE = os.path.dirname(os.path.abspath(__file__))
SPECS = os.path.join(HERE, "apps", "apps.json")
# Workspaces of the harness rustc accepts, checked as the apps are (CI checks them too).
LOCAL = {"corpus": os.path.join(HERE, "corpus"), "mixfutures": os.path.join(HERE, "mixfutures")}


class StepError(Exception):
    pass


async def run(argv, cwd, timeout, env=None):
    """Runs a command to its end, returning (exit code, stdout, stderr)."""
    proc = await asyncio.create_subprocess_exec(*argv, cwd=cwd, env=env, stdout=asyncio.subprocess.PIPE,
                                                stderr=asyncio.subprocess.PIPE)
    try:
        async with asyncio.timeout(timeout):
            out, err = await proc.communicate()
    except TimeoutError:
        proc.kill()
        await proc.wait()
        raise StepError(f"`{' '.join(argv)}` timed out after {timeout}s in {cwd}")
    return proc.returncode, out.decode(errors="replace"), err.decode(errors="replace")


async def checked(argv, cwd, timeout):
    code, out, err = await run(argv, cwd, timeout)
    if code != 0:
        raise StepError(f"`{' '.join(argv)}` exited {code} in {cwd}: {err[-800:]}")
    return out


async def checkout(name, spec, apps_dir):
    """The app's workspace at its pinned commit."""
    if "source" in spec:
        return os.path.join(HERE, "apps", spec["source"])
    ws = os.path.join(apps_dir, name)
    if not os.path.isdir(os.path.join(ws, ".git")):
        os.makedirs(ws, exist_ok=True)
        await checked(["git", "init", "-q"], ws, 60)
        await checked(["git", "remote", "add", "origin", spec["repo"]], ws, 60)
    have = (await run(["git", "rev-parse", "HEAD"], ws, 60))[1].strip()
    if have != spec["rev"]:
        await checked(["git", "fetch", "-q", "--depth", "1", "origin", spec["rev"]], ws, 600)
        await checked(["git", "checkout", "-q", "FETCH_HEAD"], ws, 120)
    # What `cargo metadata` needs that the clone lacks (bun's vendored crates), without the
    # build its `prepare` makes for building it: the checker builds nothing.
    if "prepare" in spec:
        await asyncio.to_thread(importlib.import_module(spec["prepare"]).sources, ws)
    return ws


async def check(name, spec, args, gate) -> dict[str, Any]:
    """{"errors": [...], "panics": [...], "summary": {...}} or {"failed": why}."""
    async with gate:
        began = time.monotonic()
        try:
            ws = LOCAL[name] if name in LOCAL else await checkout(name, spec, args.apps_dir)
            env = dict(os.environ, RUST_BACKTRACE="0")
            code, out, err = await run([args.bin, "borrowck", "--project", ws], ws, args.timeout, env)
        except StepError as e:
            return {"failed": str(e)}
        lines = []
        for line in out.splitlines():
            try:
                lines.append(json.loads(line))
            except ValueError:
                pass
        result: dict[str, Any] = {"errors": [l for l in lines if "error" in l], "panics": [l for l in lines if "panic" in l],
                  "summary": lines[-1] if lines and "files" in lines[-1] else None,
                  "secs": round(time.monotonic() - began)}
        if result["summary"] is None:
            result["failed"] = f"`ra-jit borrowck --project` exited {code} with no summary: {err[-800:]}"
        print(f"{name}: {len(result['errors'])} errors, {len(result['panics'])} panics"
              f"{', FAILED' if 'failed' in result else ''} ({result['secs']}s)", file=sys.stderr, flush=True)
        return result


async def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("names", nargs="*")
    ap.add_argument("--bin", default=os.environ.get("RA_JIT_BIN", "ra-jit"))
    ap.add_argument("--apps-dir", default=os.environ.get("APPS_DIR") or os.path.join(HERE, "vendor", "apps"))
    ap.add_argument("--jobs", type=int, default=4)
    ap.add_argument("--timeout", type=int, default=3600)
    ap.add_argument("--json")
    ap.add_argument("--skip", action="append", default=[], help="apps left out")
    args = ap.parse_args()
    args.bin = os.path.abspath(args.bin) if os.sep in args.bin else args.bin
    specs = json.load(open(SPECS))
    specs.update({name: {} for name in LOCAL})
    names = args.names or [n for n in specs if n not in args.skip]
    gate = asyncio.Semaphore(args.jobs)
    async with asyncio.TaskGroup() as group:
        tasks = {name: group.create_task(check(name, specs[name], args, gate)) for name in names}
    results = {name: task.result() for name, task in tasks.items()}
    bad = 0
    for name, r in results.items():
        if "failed" in r:
            bad += 1
            print(f"\n== {name}: FAILED: {r['failed']}")
        if r.get("errors") or r.get("panics"):
            bad += 1
            print(f"\n== {name}: {len(r['errors'])} false errors, {len(r['panics'])} panics")
            for e in r["errors"]:
                d = e["error"]
                print(f"   {e['path']}:{d.get('line')}: {d.get('code')} {d.get('message', '')[:160]}")
            for p in r["panics"]:
                print(f"   {p['path']}: panic: {p['panic'][:200]}")
        elif "failed" not in r:
            print(f"== {name}: clean, {r['summary']}")
    if args.json:
        json.dump(results, open(args.json, "w"), indent=1)
    by = collections.Counter("clean" if not (r.get("errors") or r.get("panics") or "failed" in r) else "bad"
                             for r in results.values())
    print(f"\n{by['clean']} of {len(results)} apps check clean")
    sys.exit(1 if bad else 0)


if __name__ == "__main__":
    asyncio.run(main())
