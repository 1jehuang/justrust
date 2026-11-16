#!/usr/bin/env python3
"""Require a snapshot-wrapper shell that preserves Cargo-style env names.

This is a runner prerequisite, not an injected CARGO_BIN_EXE value. Cargo
alone supplies binary paths. dash removes non-shell-identifier names on
entry, breaking env! in hyphenated packages behind a #!/bin/sh rustc wrapper.
"""
import os
from pathlib import Path
import subprocess
import sys


def check(shell="/bin/sh"):
    key = "JUSTRUST-shell-environment-probe"
    result = subprocess.run(
        [str(shell), "-c", 'exec "$@"', "snapshot-wrapper", sys.executable,
         "-c", "import os,sys; sys.exit(os.environ.get(sys.argv[1]) != 'retained')", key],
        env={**os.environ, key: "retained"}, capture_output=True, text=True,
    )
    if result.returncode:
        raise RuntimeError(
            f"{shell} does not preserve non-identifier environment names through exec; "
            "pinned rustc wrappers require a bash-backed /bin/sh to retain Cargo's "
            "hyphenated binary-path variables. Configure the ephemeral runner, "
            "do not inject binary paths or rewrite snapshots.\n" + result.stderr)
    return str(Path(shell).resolve())


if __name__ == "__main__":
    print(f"snapshot wrapper shell: {check()}")
