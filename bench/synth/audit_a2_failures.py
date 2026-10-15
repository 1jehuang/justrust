#!/usr/bin/env python3
"""Read-only A2 failing-test audit using preserved binaries, never Cargo.

Each invocation gets disposable HOME/application/config/runtime/temp state.
Original-path and same-inode deps-path executions are diagnostic controls,
not a changed benchmark definition or exact historical replay.
"""
import argparse
import collections
import hashlib
import json
import os
from pathlib import Path
import signal
import subprocess
import sys
import tempfile


def sha(path):
    digest = hashlib.sha256()
    with path.open("rb") as stream:
        for block in iter(lambda: stream.read(1024 * 1024), b""):
            digest.update(block)
    return digest.hexdigest()


def snapshot_manifest(root):
    manifest = {}
    for path in sorted(root.rglob("*")):
        relative = str(path.relative_to(root))
        if path.is_symlink():
            manifest[relative] = {"symlink": os.readlink(path)}
        elif path.is_file():
            manifest[relative] = {"sha256": sha(path), "mode": path.stat().st_mode & 0o777}
        elif path.is_dir():
            manifest[relative] = {"directory": True}
    return manifest


def invoke(binary, test, cwd, scratch, label, extra_files=None, timeout=40):
    with tempfile.TemporaryDirectory(prefix="a2-failure-state-", dir=scratch) as tmp:
        home = Path(tmp)
        for directory in [".jcode", ".config", ".cache", ".local/share", "run", "tmp", "scratch"]:
            (home / directory).mkdir(parents=True, exist_ok=True)
        (home / "run").chmod(0o700)
        env = {
            "PATH": os.environ.get("PATH", os.defpath),
            "HOME": str(home), "JCODE_HOME": str(home / ".jcode"),
            "JCODE_RUNTIME_DIR": str(home / "run"),
            "JCODE_SCRATCH_DIR": str(home / "scratch"),
            "XDG_CONFIG_HOME": str(home / ".config"),
            "XDG_CACHE_HOME": str(home / ".cache"),
            "XDG_DATA_HOME": str(home / ".local/share"),
            "XDG_RUNTIME_DIR": str(home / "run"),
            "TMPDIR": str(home / "tmp"),
            # Original build/out paths do not trigger the app's browser guard.
            # This is a safety control, not an outcome override or harness fix.
            "JCODE_NO_BROWSER": "1",
            "JCODE_NO_TELEMETRY": "1",
            "PYTHONDONTWRITEBYTECODE": "1",
        }
        for relative, value in (extra_files or {}).items():
            path = home / relative
            if not path.resolve().is_relative_to(home):
                raise ValueError("synthetic config path escapes the disposable home")
            path.parent.mkdir(parents=True, exist_ok=True)
            path.write_text(value)
        command = [str(binary), "--exact", test, "--nocapture"]
        process = subprocess.Popen(command, cwd=cwd, env=env, stdout=subprocess.PIPE,
                                   stderr=subprocess.PIPE, text=True, start_new_session=True)
        timed_out = False
        try:
            out, err = process.communicate(timeout=timeout)
        except subprocess.TimeoutExpired:
            timed_out = True
            try:
                os.killpg(process.pid, signal.SIGKILL)
            except ProcessLookupError:
                pass
            out, err = process.communicate()
        output = out + err
        for private, public in [(str(home), "$CASE"), (str(Path.home()), "$HOME")]:
            output = output.replace(private, public)
        return {"control": label, "exit": process.returncode, "timed_out": timed_out,
                "selected_one_test": "running 1 test" in output,
                "output": output}


def audit(a2, baseline_audit):
    a2 = a2.resolve()
    baseline = json.loads(baseline_audit.read_text())
    failed = collections.defaultdict(list)
    for point in baseline["outcome_log_audits"]:
        for line in point["failed_test_lines"]:
            failed[line.split(" ... ")[0].removeprefix("test ")].append(point["point"])
    first = json.loads((a2 / "home/runs/20261014-223949395-1788897209/meta.json").read_text())
    workspace = Path(first["cwd"])
    target = Path(first["env"]["CARGO_TARGET_DIR"])
    binary_specs = {
        "base": ("jcode-base", "aceefc8ad712df67", "jcode_base"),
        "app": ("jcode-app-core", "b11c65914a3d522b", "jcode_app_core"),
        "tui": ("jcode-tui", "fc4cea1f23d3a710", "jcode_tui"),
        "desktop": ("jcode-desktop-ui", "afb067cc750c9735", "jcode_desktop_ui"),
    }
    binaries = {key: target / "debug/build" / package / ident / "out" / f"{stem}-{ident}"
                for key, (package, ident, stem) in binary_specs.items()}
    for path in [workspace, *binaries.values()]:
        if not path.resolve().is_relative_to(a2):
            raise ValueError("A2 input path escapes the declared tree")
    binary_hashes = {key: sha(path) for key, path in binaries.items()}
    before = snapshot_manifest(workspace.parent)
    scratch = os.environ.get("JCODE_SCRATCH_DIR")
    results = []
    with tempfile.TemporaryDirectory(prefix="a2-failure-aliases-", dir=scratch) as tmp:
        deps = Path(tmp) / "deps"
        if "target" in deps.parts:
            raise ValueError("scratch location must not accidentally satisfy /target/ detection")
        deps.mkdir()
        aliases = {}
        for key, binary in binaries.items():
            aliases[key] = deps / binary.name
            os.link(binary, aliases[key])
            assert os.path.samefile(binary, aliases[key])
        for index, (test, points) in enumerate(failed.items()):
            if test.startswith(("applet_runtime::", "workspace::")):
                key = "desktop"
            elif test.startswith("tui::"):
                key = "tui"
            elif test.startswith(("auth::", "provider::")):
                key = "base"
            else:
                key = "app"
            if key == "desktop":
                cwd = workspace.parent / "jcode-desktop/crates/jcode-desktop-ui"
            else:
                cwd = workspace / "crates" / binary_specs[key][0]
            # Cargo executes package tests from their package root, unlike the
            # recorded command's workspace cwd. Both locations are gitless.
            controls = [invoke(binaries[key], test, cwd, scratch, "original-path-clean-state"),
                        invoke(aliases[key], test, cwd, scratch, "hashed-deps-clean-state")]
            results.append({"test": test, "affected_points": points, "crate": key,
                            "cwd_relative_to_a2": str(cwd.relative_to(a2)), "controls": controls})
            print(f"audited {index + 1}/{len(failed)} {test}: "
                  f"{[entry['exit'] for entry in controls]}", file=sys.stderr)

        catalog = "provider::tests::test_anthropic_live_catalog_replaces_static_fallback_list"
        api = ".jcode/config/jcode/anthropic.env"
        oauth = ".jcode/auth.json"
        account = json.dumps({"anthropic_accounts": [{"label": "work", "access": "fake-access",
                                                     "refresh": "fake-refresh", "expires": 4102444800000}],
                              "active_anthropic_account": "work"})
        fake_api = "ANTHROPIC_API_KEY=synthetic-diagnostic-not-a-real-key\n"
        configs = [("neither", {}), ("api-only", {api: fake_api}),
                   ("oauth-only", {oauth: account}), ("api-and-oauth", {api: fake_api, oauth: account})]
        catalog_controls = [invoke(aliases["base"], catalog, workspace / "crates/jcode-base",
                                   scratch, label, files) for label, files in configs]
    assert binary_hashes == {key: sha(path) for key, path in binaries.items()}
    after = snapshot_manifest(workspace.parent)
    if before != after:
        changed = [path for path in before.keys() | after.keys() if before.get(path) != after.get(path)]
        raise RuntimeError(f"A2 snapshot content changed during diagnostic: {changed}")
    tree_digest = hashlib.sha256(json.dumps(before, sort_keys=True).encode()).hexdigest()
    return {
        "schema": 1, "scope": "all preserved A2 failing tests, targeted no-build isolation diagnostics",
        "baseline_audit_sha256": sha(baseline_audit),
        "historical_environment_or_per_run_binary_identity_attested": False,
        "preserved_current_binary_sha256_before_and_after": binary_hashes,
        "snapshot_tree_manifest_sha256_before_and_after": tree_digest,
        "snapshot_tree_manifest_entries": len(before),
        "unique_failing_tests": len(failed), "failing_test_occurrences": sum(map(len, failed.values())),
        "original_snapshots_and_observations_modified": False,
        "controls_are_benchmark_samples": False,
        "safety": "Disposable HOME/JCODE_HOME/XDG/runtime/temp paths, no inherited credentials/display/DBus, browser suppressed, captured stdout; no Cargo or compiler invoked",
        "results": results, "synthetic_catalog_controls": catalog_controls,
        "limitations": ["Preserved target binaries may reflect later A2 point recompiles.",
                        "Minimal environment is a diagnostic intervention, not a replay or benchmark.",
                        "Single-test passes cannot establish freedom from whole-suite races.",
                        "Original-path vs alias control isolates execution path within clean state only."]}


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("a2_root", type=Path)
    parser.add_argument("--baseline-audit", type=Path, default=Path(__file__).resolve().parents[1] /
                        "results/native-dispatch-synth40-A2-audit-20261015.json")
    args = parser.parse_args()
    print(json.dumps(audit(args.a2_root, args.baseline_audit), indent=2))
