#!/usr/bin/env python3
"""Reproduce pure A2 assertions using preserved binaries, without compiling.

Only disposable hardlinks are created. Original snapshots and binaries are
never modified. Full historical environment and per-invocation binary hashes
were not captured, so this is not an exact historical replay or a benchmark.
"""
import argparse
import hashlib
import json
import os
from pathlib import Path
import subprocess
import tempfile


def sha(path):
    digest = hashlib.sha256()
    with path.open("rb") as stream:
        for block in iter(lambda: stream.read(1024 * 1024), b""):
            digest.update(block)
    return digest.hexdigest()


def reproduce(root):
    root = root.resolve()
    run_ids = {"base": "20261014-223949395-1788897209",
               "app": "20261014-223710077-1788897721"}
    metadata = {key: json.loads((root / "home/runs" / rid / "meta.json").read_text())
                for key, rid in run_ids.items()}
    target = Path(metadata["base"]["env"]["CARGO_TARGET_DIR"])
    binaries = {
        "base": target / "debug/build/jcode-base/aceefc8ad712df67/out/jcode_base-aceefc8ad712df67",
        "app": target / "debug/build/jcode-app-core/b11c65914a3d522b/out/jcode_app_core-b11c65914a3d522b",
    }
    for binary in binaries.values():
        if not binary.resolve().is_relative_to(root):
            raise ValueError("binary is outside the declared A2 tree")
    workspace = Path(metadata["base"]["cwd"])
    if not workspace.resolve().is_relative_to(root):
        raise ValueError("workspace is outside the declared A2 tree")
    original_hashes = {key: sha(path) for key, path in binaries.items()}
    results = []

    def run(label, key, test, binary=None, cwd=None, extra_env=None, clean_home=None):
        meta = metadata[key]
        env = os.environ.copy() if clean_home is None else {
            "PATH": os.environ.get("PATH", os.defpath),
            "HOME": str(clean_home),
            "JCODE_HOME": str(clean_home / ".jcode"),
            "XDG_CONFIG_HOME": str(clean_home / ".config"),
            "XDG_CACHE_HOME": str(clean_home / ".cache"),
            "XDG_DATA_HOME": str(clean_home / ".local/share"),
        }
        env.update(meta["env"])
        if extra_env:
            env.update(extra_env)
        command = [str(binary or binaries[key]), "--exact", test, "--nocapture"]
        result = subprocess.run(command, cwd=cwd or meta["cwd"], env=env,
                                capture_output=True, text=True, timeout=30)
        output = result.stdout + result.stderr
        for private, public in ((str(root), "$A2"), (str(Path.home()), "$HOME")):
            output = output.replace(private, public)
        results.append({"case": label, "test": test, "exit": result.returncode,
                        "output": output})

    auth = "auth::tests::browser_suppressed_inside_test_harness_without_env_overrides"
    run("original-build-out", "base", auth)
    run("path-parser-positive-controls", "base",
        "auth::tests::test_harness_paths_include_custom_cargo_target_directories")
    scratch = os.environ.get("JCODE_SCRATCH_DIR")
    with tempfile.TemporaryDirectory(prefix="a2-path-controls-", dir=scratch) as tmp:
        scratch_root = Path(tmp)
        # Avoid accidentally satisfying the older '/target/' fallback.
        if "target" in scratch_root.parts:
            raise ValueError("diagnostic scratch path must not contain a target component")
        for parent, name, label in [
            ("deps", binaries["base"].name, "hashed-deps-same-inode"),
            ("out", binaries["base"].name, "hashed-non-deps-negative"),
            ("deps", "not_a_harness", "unhashed-deps-negative"),
        ]:
            directory = scratch_root / parent
            directory.mkdir(exist_ok=True)
            alias = directory / name
            os.link(binaries["base"], alias)
            assert os.path.samefile(binaries["base"], alias)
            run(label, "base", auth, binary=alias)

    git_test = "tool::bash::tests::repository_commands_export_a_logged_cargo_function"
    run("gitless-manifest-ancestor", "app", git_test)
    # The test calls find_repo_in_ancestors(CARGO_MANIFEST_DIR) directly:
    # changing cwd and the get_repo_dir override cannot change its result.
    run("gitless-explicit-cwd-and-repo-override", "app", git_test,
        cwd=workspace / "crates/jcode-app-core",
        extra_env={"JCODE_REPO_DIR": str(workspace)})
    for name in ["tool_descriptions_stay_under_token_cap",
                 "tool_parameter_descriptions_stay_under_token_cap"]:
        run(name, "app", f"tool::tests::{name}")

    with tempfile.TemporaryDirectory(prefix="a2-clean-state-", dir=scratch) as tmp:
        home = Path(tmp)
        for name in ["tool_descriptions_stay_under_token_cap",
                     "tool_parameter_descriptions_stay_under_token_cap"]:
            run("empty-home-" + name, "app", f"tool::tests::{name}", clean_home=home)
        run("empty-home-anthropic-catalog", "base",
            "provider::tests::test_anthropic_live_catalog_replaces_static_fallback_list",
            clean_home=home)
        config = home / ".jcode/config/jcode/anthropic.env"
        config.parent.mkdir(parents=True, exist_ok=True)
        config.write_text("ANTHROPIC_API_KEY=synthetic-diagnostic-not-a-real-key\n")
        run("fake-config-key-anthropic-catalog", "base",
            "provider::tests::test_anthropic_live_catalog_replaces_static_fallback_list",
            clean_home=home)
        config.unlink()
        run("removed-fake-key-anthropic-catalog", "base",
            "provider::tests::test_anthropic_live_catalog_replaces_static_fallback_list",
            clean_home=home)

    sources = ["crates/jcode-base/src/auth/mod.rs", "crates/jcode-base/src/auth/tests.rs",
               "crates/jcode-build-support/src/paths.rs",
               "crates/jcode-app-core/src/tool/bash_tests.rs",
               "crates/jcode-app-core/src/tool/tests.rs",
               "crates/jcode-app-core/src/tool/compile_remote.rs",
               "crates/jcode-app-core/src/tool/gmail.rs",
               "crates/jcode-message-types/src/lib.rs",
               "crates/jcode-core/src/util.rs",
               "crates/jcode-base/src/provider/models.rs",
               "crates/jcode-base/src/provider/tests/catalog_subscription.rs",
               "crates/jcode-provider-env/src/lib.rs",
               "crates/jcode-storage/src/lib.rs"]
    remote_line = next(line for line in (workspace / sources[5]).read_text().splitlines()
                       if line.startswith("const DESCRIPTION: &str = "))
    remote = json.loads(remote_line.split(" = ", 1)[1].removesuffix(";"))
    gmail_line = next(line for line in (workspace / sources[6]).read_text().splitlines()
                      if "Comma-separated Bcc addresses, hidden" in line)
    gmail = json.loads(gmail_line.split(":", 1)[1].strip())
    # Only the source locations satisfying the manifest half of the predicate
    # matter. Do not report private ancestor names or read credential files.
    manifest_candidates = [directory for directory in (workspace, *workspace.parents)
                           if (directory / "Cargo.toml").is_file()
                           and 'name = "jcode"' in (directory / "Cargo.toml").read_text()]
    assert original_hashes == {key: sha(path) for key, path in binaries.items()}
    return {
        "schema": 1,
        "scope": "targeted no-build isolation diagnostics, not performance evidence",
        "historical_binary_identity_attested": False,
        "environment": "current environment plus recorded meta.env subset, not complete historical environment",
        "recorded_environment_keys": sorted(metadata["base"]["env"]),
        "metadata_sha256": {key: sha(root / "home/runs" / rid / "meta.json")
                            for key, rid in run_ids.items()},
        "preserved_binary_sha256_before_and_after": original_hashes,
        "source_sha256": {path: sha(workspace / path) for path in sources},
        "static_token_cap_estimates": {
            "compile_remote": {"utf8_bytes": len(remote.encode()), "estimate": len(remote.encode()) // 4, "cap": 20},
            "gmail_bcc": {"utf8_bytes": len(gmail.encode()), "estimate": len(gmail.encode()) // 4, "cap": 25},
        },
        "git_manifest_candidates": len(manifest_candidates),
        "git_manifest_candidates_with_dotgit": sum((path / ".git").exists() for path in manifest_candidates),
        "results": results,
        "limitations": ["Target artifacts may have been overwritten by later points in A2.",
                        "Single selected tests do not recreate whole-suite parallel state.",
                        "No app source, original binary, benchmark observation or expectation was changed."],
    }


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("a2_root", type=Path)
    args = parser.parse_args()
    print(json.dumps(reproduce(args.a2_root), indent=2))
