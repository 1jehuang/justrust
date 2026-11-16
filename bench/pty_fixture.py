"""Fail-closed identity for the explicitly corrected pinned real-PTY test fixture.

This is a fixture contract revision, not the pristine abff94eeb test runtime.
Both comparison sides must use this same overlay and its snapshot fingerprint.
"""
import hashlib
from pathlib import Path
import subprocess

VERSION = "pinned-jcode-pty-self-exec-v1"
INPUT = "crates/jcode-tui/src/tui/app/tests/terminal_setup_command.rs"
INPUT_SHA256 = "2240d4d344f58e965c75d5f4d98072cd2dddc69bd5ca5bb45bbb50bb522aa1f9"
PATCH = "bench/patches/jcode-pty-self-exec-v1.patch"
PATCH_SHA256 = "5ced18aa42a5c37331dc9196d5a89fbc33c7cdd5bf0985440d3c267b55ea39d5"
ROOT = Path(__file__).resolve().parent.parent


def identity():
    actual = hashlib.sha256((ROOT / PATCH).read_bytes()).hexdigest()
    if actual != PATCH_SHA256:
        raise ValueError("bench: corrupt or unversioned PTY fixture patch")
    return {"version": VERSION, "input": INPUT, "input_sha256": INPUT_SHA256,
            "patch": PATCH, "patch_sha256": PATCH_SHA256}


def apply(destination):
    identity()
    actual = hashlib.sha256((destination / INPUT).read_bytes()).hexdigest()
    if actual != INPUT_SHA256:
        raise ValueError("bench: unexpected pinned PTY fixture input SHA256")
    patch = ROOT / PATCH
    subprocess.run(["git", "-C", str(destination), "apply", "--check", str(patch)], check=True)
    subprocess.run(["git", "-C", str(destination), "apply", str(patch)], check=True)
