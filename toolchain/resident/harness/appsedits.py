"""appsedits.py: the edit loop of apps.py.

The daemon holds every crate of the program. Each edit of the application's `edits` (a file,
the text to replace and its replacement) is made on top of the ones before it; the daemon is
asked for the program, which is timed, and `check` (a command line of the program, the edit's
own or the application's `edit_check`) is run with the daemon's program and with the one
`cargo build` makes of the same source, which is timed too. Both have to print the same. How
each edit got linked, spliced or in full, is read from the daemon's log.
"""
import os, re, shlex, statistics, subprocess, time

import linkline
from difftest import RA_JIT, sysroot_rlibs


def peak_mb(pid):
    """The most memory the process has had resident."""
    for line in open(f"/proc/{pid}/status"):
        if line.startswith("VmHWM:"):
            return int(line.split()[1]) // 1024
    return 0


def run(fx, ws, spec, result):
    import apps
    edits = spec.get("edits")
    if not edits:
        return True
    crates = fx.crates[0].split(",")
    socket, log_path = os.path.join(fx.work, "ra-jit.sock"), os.path.join(fx.work, "daemon.log")
    obj, patch, binary = (os.path.join(fx.work, f"edits.{n}") for n in ("o", "patch.o", "bin"))
    link = list(fx.link_args)
    first = linkline.first_of_rustc(link)
    link[first:first] = [patch, obj, *([] if fx.alone else linkline.allowing_duplicates(False))]
    link = linkline.with_output(link, binary)
    if len(shlex.join(link)) > 100_000:
        # One argument holds the whole line, and the kernel takes no argument that long.
        response = os.path.join(fx.work, "link.args")
        open(response, "w").write("\n".join(shlex.quote(a) for a in link[1:]))
        link[1:] = ["@" + response]
    symbols = [x for rlib in fx.rlibs + sysroot_rlibs() for x in ("--symbols", rlib)]
    ask = [*(x for c in crates for x in ("--crate", c)), "--out", obj, "--patch", patch,
           "--link-cmd", shlex.join(link), "--splice", binary, *symbols]
    log = open(log_path, "a")
    began = time.monotonic()
    daemon = subprocess.Popen([RA_JIT, fx.project, "--socket", socket, *ask, *fx.ra_jit_args], stderr=log, stdout=log)
    while "watching" not in open(log_path).read():
        if daemon.poll() is not None:
            result["edits"] = {"error": open(log_path).read()[-1500:]}
            print(f"edits: the daemon did not start: {log_path}")
            return False
        time.sleep(0.05)
    row = {"first_s": round(time.monotonic() - began, 1), "steps": []}
    originals, wrong = {}, 0
    try:
        for edit in edits:
            path = os.path.join(ws, edit["file"])
            text = open(path).read()
            originals.setdefault(path, text)
            if edit["old"] not in text:
                raise SystemExit(f"edits: `{edit['old']}` is not in {edit['file']}")
            open(path, "w").write(text.replace(edit["old"], edit["new"], 1))
            seen = len(open(log_path).read())
            began = time.monotonic()
            reply = subprocess.run([RA_JIT, "sync", socket], capture_output=True, text=True, timeout=1800)
            sync_ms = (time.monotonic() - began) * 1000
            reply = (reply.stdout + reply.stderr).strip()
            links = re.findall(r"^link: \S+ \((.*)\)$", open(log_path).read()[seen:], re.M)
            how = "no link" if not links else "spliced" if links[-1].startswith("spliced") else \
                "full: " + (re.search(r"not spliced: (.*)", links[-1]) or [None, "not asked to splice"])[1]
            cargo, cargo_s = apps.reference(ws, spec, "debug", timed=True)
            fx.run_args, fx.stdin = edit.get("check", spec.get("edit_check", [])), None
            want, got = fx.behaviour(cargo), fx.behaviour(binary)
            same = reply.startswith("ok") and want == got
            wrong += not same
            row["steps"].append({"edit": f"{edit['file']}: {edit.get('what', edit['old'][:40])}",
                                 "sync_ms": round(sync_ms), "how": how, "cargo_s": cargo_s, "same": same})
            print(f"  edit {len(row['steps']):2}  sync {sync_ms:7.0f} ms  cargo {cargo_s:5.1f} s  {how[:60]:60} "
                  f"{'same' if same else 'WRONG: ' + reply[:200]}", flush=True)
    finally:
        row["peak_mb"] = peak_mb(daemon.pid) if daemon.poll() is None else 0
        daemon.kill()
        for path, text in originals.items():
            open(path, "w").write(text)
    steps = row["steps"]
    row.update(spliced=sum(s["how"] == "spliced" for s in steps), full=sum(s["how"].startswith("full") for s in steps),
               wrong=wrong, sync_ms=round(statistics.median(s["sync_ms"] for s in steps)),
               cargo_s=round(statistics.median(s["cargo_s"] for s in steps), 1))
    result["edits"] = row
    print(f"edits: {len(steps) - wrong} of {len(steps)} the same as cargo's; {row['spliced']} spliced, {row['full']} linked "
          f"in full; median {row['sync_ms']} ms to a program against cargo's {row['cargo_s']} s; the daemon's first "
          f"program {row['first_s']} s, peak {row['peak_mb']} MB")
    return wrong == 0
