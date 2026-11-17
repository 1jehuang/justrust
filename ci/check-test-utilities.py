#!/usr/bin/env python3
"""Check exact pinned tests' host utility requirements without editing sources."""
import hashlib
import json
import os
from pathlib import Path
import shutil
import subprocess
import tempfile
import time


def check_sleep(executable="/bin/sleep"):
    # browser_tests.rs copies sleep under this name, then replaces its inode.
    # A multicall dispatcher keyed on argv[0] cannot stand in for that binary.
    with tempfile.TemporaryDirectory(dir=os.environ.get("RUNNER_TEMP") or
                                     os.environ.get("JCODE_SCRATCH_DIR")) as tmp:
        renamed = Path(tmp) / "firefox-agent-bridge-host"
        shutil.copy2(executable, renamed)
        child = subprocess.Popen([str(renamed), "30"], stdout=subprocess.PIPE,
                                 stderr=subprocess.PIPE)
        try:
            try:
                _, stderr = child.communicate(timeout=0.25)
            except subprocess.TimeoutExpired:
                return
            raise RuntimeError("pinned browser tests require /bin/sleep to run after copying "
                               "to another basename; provision standalone GNU sleep: " +
                               stderr.decode(errors="replace"))
        finally:
            if child.poll() is None:
                child.kill()
            child.communicate()


def reading_stdin(pid):
    # Match the pinned jcode-core stdin_detect.rs syscall/wchan contract.
    proc = Path(f"/proc/{pid}")
    try:
        fields = (proc / "syscall").read_text().split()
        if len(fields) >= 2 and fields[0] in ("0", "63") and fields[1] == "0x0":
            return True
    except OSError:
        pass
    try:
        return ((proc / "wchan").read_text().strip() in
                {"n_tty_read", "wait_woken", "pipe_read", "pipe_wait_readable",
                 "unix_stream_read_generic"} and
                "pipe" in os.readlink(proc / "fd/0"))
    except OSError:
        return False


def check_head(executable=None):
    executable = executable or shutil.which("head")
    if not executable:
        raise RuntimeError("pinned stdin tests require head on PATH")
    child = subprocess.Popen([str(executable), "-n1"], stdin=subprocess.PIPE,
                             stdout=subprocess.PIPE, stderr=subprocess.PIPE)
    try:
        deadline = time.monotonic() + 2
        while child.poll() is None and time.monotonic() < deadline:
            if reading_stdin(child.pid):
                stdout, stderr = child.communicate(b"stdin-probe\n", timeout=2)
                if child.returncode == 0 and stdout == b"stdin-probe\n":
                    return
                raise RuntimeError(f"head failed stdin roundtrip: {stderr!r}")
            time.sleep(0.01)
        raise RuntimeError("pinned stdin tests require detectable blocking head stdin reads; "
                           "provision GNU head before benchmarking")
    finally:
        if child.poll() is None:
            child.kill()
        child.communicate()


def check_poppler():
    """Reject absent or unusable PDF tools before compiling either workspace."""
    paths = {}
    for program in ("pdfinfo", "pdftoppm"):
        path = shutil.which(program)
        if not path:
            raise RuntimeError(f"pinned Desktop PDF tests require {program}; install poppler-utils")
        try:
            result = subprocess.run([path, "-v"], capture_output=True, text=True, timeout=10)
        except (OSError, subprocess.TimeoutExpired) as error:
            raise RuntimeError(f"pinned Desktop PDF tool {program} is unusable: {error}") from error
        if result.returncode or not (result.stdout + result.stderr).startswith(f"{program} version "):
            raise RuntimeError(f"pinned Desktop PDF tool {program} failed its version probe: "
                               f"{result.stdout}{result.stderr}")
        paths[program] = path
    return paths


def check():
    check_sleep()
    check_head()
    paths = {"sleep": "/bin/sleep", "head": shutil.which("head"), **check_poppler()}
    return {name: {"path": str(Path(path).resolve()),
                   "sha256": hashlib.sha256(Path(path).read_bytes()).hexdigest()}
            for name, path in paths.items()}


if __name__ == "__main__":
    print(json.dumps(check(), indent=2, sort_keys=True))
