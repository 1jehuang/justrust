"""Opt-in benchmark application runtime isolation, never compiler flag rewriting."""
from contextlib import contextmanager
import os
from pathlib import Path
import subprocess
import tempfile

SELECTOR = "JUSTRUST_BENCH_RUNTIME_CONTEXT"
POLICY = "jcode-isolated-v1"
PREFIXES = ("JCODE_", "ANTHROPIC_", "OPENAI_", "OPENROUTER_", "CLAUDE_", "CODEX_", "GEMINI_", "GOOGLE_API_", "GIT_")


def provenance():
    value = os.environ.get(SELECTOR)
    if not value:
        return {"policy": "inherited"}
    if value != POLICY:
        raise ValueError(f"unsupported {SELECTOR}: {value!r}")
    return {"policy": POLICY, "state": "fresh per command", "git": "temporary empty repository if absent",
            "credentials": "provider/session environment removed, private HOME and XDG directories",
            "compiler_environment": "unchanged"}


def require_matching_context(a, b):
    """Old result files without policy metadata used the inherited context."""
    inherited = {"policy": "inherited"}
    if a.get("env", {}).get("runtime_context", inherited) != b.get("env", {}).get("runtime_context", inherited):
        raise ValueError("benchmark runtime contexts differ, results are not comparable")


@contextmanager
def runtime_context(env, cwd):
    """Yield isolated child env. No source content, command, or cache identity changes.

    Keep Cargo/rustup homes explicit before replacing HOME, so selecting this
    runtime policy cannot silently select a different compiler or registry.
    A real Git marker is needed by application filesystem-based discovery.
    The marker is ours only while this command runs, never a checkout/worktree.
    """
    policy = provenance()
    if policy["policy"] == "inherited":
        yield env
        return
    cwd = Path(cwd).resolve()
    with tempfile.TemporaryDirectory(prefix="justrust-runtime-") as temporary:
        private = Path(temporary)
        home = private / "home"
        home.mkdir(mode=0o700)
        child = {key: value for key, value in env.items() if not key.startswith(PREFIXES)}
        old_home = Path(env.get("HOME", str(Path.home())))
        child.setdefault("CARGO_HOME", str(old_home / ".cargo"))
        child.setdefault("RUSTUP_HOME", str(old_home / ".rustup"))
        child.update(HOME=str(home), JCODE_HOME=str(home / ".jcode"),
                     XDG_CONFIG_HOME=str(home / ".config"), XDG_DATA_HOME=str(home / ".local/share"),
                     XDG_CACHE_HOME=str(home / ".cache"), XDG_STATE_HOME=str(home / ".local/state"),
                     XDG_RUNTIME_DIR=str(private / "runtime"))
        Path(child["XDG_RUNTIME_DIR"]).mkdir(mode=0o700)
        marker = cwd / ".git"
        created = False
        marker_text = f"gitdir: {private / 'git'}\n"
        try:
            if not any((parent / ".git").exists() for parent in (cwd, *cwd.parents)):
                git_env = {key: value for key, value in child.items() if not key.startswith("GIT_")}
                subprocess.run(["git", "init", "--bare", "--quiet", "--template=", str(private / "git")],
                               env=git_env, check=True, capture_output=True)
                subprocess.run(["git", "--git-dir", str(private / "git"), "config", "core.bare", "false"],
                               env=git_env, check=True, capture_output=True)
                with marker.open("x") as output:
                    output.write(marker_text)
                created = True
            yield child
        finally:
            if created:
                if marker.read_text() != marker_text:
                    raise RuntimeError(f"benchmark Git marker changed during execution: {marker}")
                marker.unlink()
