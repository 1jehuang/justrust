"""Fail-fast, explicit runtime prerequisites for pinned benchmark tests.

Never install packages or alter the user's shell/global configuration. Callers
can supply a private sqlite3 on PATH. Only measured child processes receive the
pinned umask, preserving the security checks in the project being exercised.
"""
import hashlib
from pathlib import Path
import shutil
import subprocess

CHILD_UMASK = 0o022
POLICY = "pinned-test-runtime-v1"


def provenance(sqlite=None):
    return {"policy": POLICY, "child_umask": "0022", "sqlite3": sqlite}


def validate(require_sqlite=False):
    if not shutil.which("git"):
        raise ValueError("bench: Git is required for exact pinned snapshot metadata")
    sqlite = None
    if require_sqlite:
        executable = shutil.which("sqlite3")
        if executable is None:
            raise ValueError("bench: pinned tests require the sqlite3 executable, not only libsqlite3; "
                             "provide an explicit sqlite3 installation on PATH before starting the run")
        path = Path(executable).resolve(strict=True)
        try:
            version = subprocess.run([str(path), "--version"], check=True, capture_output=True,
                                     text=True, timeout=10).stdout.strip()
        except (OSError, subprocess.SubprocessError) as err:
            raise ValueError("bench: sqlite3 on PATH could not report its version") from err
        if not version or len(version) > 512:
            raise ValueError("bench: malformed sqlite3 version output")
        sqlite = {"version": version, "sha256": hashlib.sha256(path.read_bytes()).hexdigest()}
    return provenance(sqlite)
