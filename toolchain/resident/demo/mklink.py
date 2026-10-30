#!/usr/bin/env python3
"""Turn rustc's link line for `app` into link.sh, with the ra-jit object taking precedence over libhot."""
import shlex, subprocess, sys
import os; os.utime("app/src/main.rs")
out = subprocess.run(["cargo", "rustc", "-p", "app", "--", "--print", "link-args", "-C", "save-temps"],
                     capture_output=True, text=True, check=True)
out = out.stdout + out.stderr
line = next(l for l in out.splitlines() if '"cc"' in l)
args = shlex.split(line)
args = args[next(i for i, a in enumerate(args) if a == "cc"):]
args = [a for a in args if not a.endswith("symbols.o")]
first_rlib = next(i for i, a in enumerate(args) if a.endswith(".rlib"))
args[first_rlib:first_rlib] = ["ra-jit.o", "-Wl,--allow-multiple-definition"]
o = args.index("-o")
args[o + 1] = "app-ra-jit"
open("link.sh", "w").write("#!/bin/sh\nexec " + shlex.join(args) + "\n")
