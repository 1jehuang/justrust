"""Opt-in authenticated hosted synthetic discovery. Never runs a local compiler."""
from __future__ import annotations

import hashlib
import json
import os
from pathlib import Path
import shlex
import subprocess


def enabled() -> bool:
    return os.environ.get("JUSTRUST_SYNTH_PROBE_REMOTE") == "1"


def parse_env(text: str) -> dict[str, str]:
    """Parse service shell assignments as data, never execute/eval them."""
    values = {}
    for line in text.splitlines():
        words = shlex.split(line)
        if len(words) != 1 or "=" not in words[0]:
            raise RuntimeError("synth: malformed authenticated remote environment")
        key, value = words[0].split("=", 1)
        if key not in {"JR_REMOTE_KIND", "JR_REMOTE_DEST", "JR_REMOTE_SSH"}:
            raise RuntimeError("synth: unexpected remote environment assignment")
        if key in values:
            raise RuntimeError("synth: duplicate remote environment assignment")
        values[key] = value
    if values.get("JR_REMOTE_KIND") != "hosted" or not values.get("JR_REMOTE_DEST") or not values.get("JR_REMOTE_SSH"):
        raise RuntimeError("synth: authenticated hosted guest required")
    return values


class HostedProbe:
    def __init__(self):
        result = subprocess.run(["justrust", "remote", "env"], capture_output=True, text=True)
        if result.returncode:
            raise RuntimeError("synth: hosted probe authentication failed\n" + result.stderr)
        values = parse_env(result.stdout)
        self.dest = values["JR_REMOTE_DEST"]
        self.shell = values["JR_REMOTE_SSH"]
        self.ssh = shlex.split(self.shell)
        if not self.ssh or self.dest.startswith("-"):
            raise RuntimeError("synth: invalid service transport")

    def run(self, script: str, *, check: bool = True):
        result = subprocess.run([*self.ssh, self.dest, script], capture_output=True, text=True)
        if check and result.returncode:
            raise RuntimeError(f"synth: hosted probe transport/command failed (exit {result.returncode})\n{result.stdout}\n{result.stderr}")
        return result

    @staticmethod
    def environment() -> str:
        # This is execution *inside* the authenticated guest, not a local fallback.
        return ('export PATH="$HOME/.cargo/bin:$HOME/.justrust/bin:$PATH"; '
                'export JUSTRUST_TOOLCHAIN_AUTO_INSTALL=off; '
                'export JUSTRUST_REMOTE=off; '
                'export JUSTRUST_LOCAL_REASON="authenticated hosted synthetic discovery"; ')

    def sync(self, root: Path, cwd: Path) -> None:
        root = root.resolve(strict=True)
        cwd.resolve(strict=True).relative_to(root)
        if root == Path("/") or not (root / cwd.relative_to(root) / "Cargo.toml").is_file():
            raise RuntimeError("synth: refusing unowned/non-workspace probe source root")
        # Reject links escaping the owned snapshot. Do not dereference symlinks
        # during rsync, and never mirror target/cache/toolchain trees.
        excluded = {"target", ".git", ".pristine", ".justrust", ".cache", "node_modules", "__pycache__"}
        pending = [str(root)]
        while pending:
            base = pending.pop()
            # Like os.walk(followlinks=False), recheck before descending in case
            # a queued directory was replaced by a link. Fail before transport.
            if os.path.islink(base):
                raise RuntimeError(f"synth: source directory became a symlink: {base}")
            with os.scandir(base) as entries:
                for entry in entries:
                    # os.walk classifies links to directories as directories for
                    # pruning. A broken or file link with an excluded name must
                    # still be checked. Never recurse into links.
                    if entry.name in excluded and entry.is_dir():
                        continue
                    if entry.is_symlink():
                        if not Path(entry.path).resolve().is_relative_to(root):
                            raise RuntimeError(f"synth: source symlink escapes owned snapshot: {entry.path}")
                    elif entry.is_dir(follow_symlinks=False):
                        pending.append(entry.path)
        self.run("mkdir -p -- " + shlex.quote(str(root)))
        cmd = ["rsync", "-az", "--checksum", "--delete", "--secluded-args", "-e", self.shell]
        for name in sorted(excluded):
            cmd += ["--exclude", name + "/"]
        result = subprocess.run([*cmd, str(root) + "/", self.dest + ":" + str(root) + "/"],
                                capture_output=True, text=True)
        if result.returncode:
            raise RuntimeError("synth: hosted probe source sync failed\n" + result.stderr)

    def identity(self, root: Path, cwd: Path) -> dict:
        self.sync(root, cwd)
        script = (self.environment() + "cd " + shlex.quote(str(cwd)) +
                  " && justrust --version && justrust toolchain status")
        result = self.run(script)
        selected = result.stdout.splitlines()[1:2]
        if not selected or not selected[0].startswith(("default ", "pinned ", "system ")) or "not installed" in selected[0]:
            raise RuntimeError("synth: cannot identify installed hosted probe compiler")
        return {"mode": "authenticated-hosted-v1", "destination": self.dest,
                "compiler": result.stdout.strip()}

    def check(self, root: Path, cwd: Path, args: list[str], target: Path):
        self.sync(root, cwd)
        script = (self.environment() + "cd " + shlex.quote(str(cwd)) + " && " +
                  "CARGO_TARGET_DIR=" + shlex.quote(str(target)) + " " +
                  shlex.join(["justrust", "cargo", "check", *args]))
        # A deliberate Rust type error must survive as exit 101 and raw JSON.
        result = self.run(script, check=False)
        if result.returncode not in (0, 101):
            raise RuntimeError(f"synth: hosted probe failed (exit {result.returncode})\n{result.stdout}\n{result.stderr}")
        return result


def cache_key(identity: dict) -> str:
    return hashlib.sha256(json.dumps(identity, sort_keys=True).encode()).hexdigest()[:20]
