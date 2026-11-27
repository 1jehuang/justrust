#!/usr/bin/env python3
"""Tests for release policy freezing, fail-closed selection and provenance."""
import importlib.util
import io
import json
import os
from pathlib import Path
import shutil
import textwrap
import tempfile
import unittest
from unittest.mock import patch

spec = importlib.util.spec_from_file_location("release_policy", Path(__file__).with_name("release-policy.py"))
p = importlib.util.module_from_spec(spec)
spec.loader.exec_module(p)


class ReleasePolicyTests(unittest.TestCase):
    def policy(self, toolchain="jr-ac20de5e7921", protocol=1):
        return {"toolchain": toolchain, "protocol": protocol, "release": {"schema": 1, "targets": {
            target: ({"toolchain": toolchain, "protocol": protocol, "kind": "hosted-policy"}
                     if target.endswith("linux-musl") else {"toolchain": "1.99.0", "kind": "stock-compatibility"})
            for target in p.TARGETS}}}

    def setUp(self):
        self.policy = self.policy()
        self.snapshot = p.freeze(self.policy)
        self.sha = p.digest(self.snapshot)

    def test_exact_specs(self):
        for value in ("jr-ac20de5e7921", "1.99.0", "nightly-2026-10-08", "beta-2026-10-08"):
            self.assertEqual(p.parse(json.dumps(ReleasePolicyTests.policy(self, value)))["toolchain"], value)

    def test_malformed_malicious_and_protocol_mismatch(self):
        for value in ("stable", "nightly", " jr-ac20de5e7921", "jr-ac20de5e7921\n", "jr-1234567/../x", "$(id)", "1.99.0\nEVIL=1", "x" * 129):
            with self.subTest(value=value), self.assertRaises(ValueError):
                p.parse(json.dumps(ReleasePolicyTests.policy(self, value)))
        for protocol in (0, 2, True, "1", 1.0, None):
            with self.subTest(protocol=protocol), self.assertRaises(ValueError):
                p.parse(json.dumps(ReleasePolicyTests.policy(self, protocol=protocol)))
        for text in ("", "null", "[]", "{}", '{"error":"unpublished"}',
                     '{"toolchain":"1.99.0","protocol":1,"protocol":2}',
                     '{"toolchain":"1.99.0","protocol":1,"extra":true}'):
            with self.subTest(text=text), self.assertRaises(ValueError):
                p.parse(text)

    def test_mapping_rejects_partial_and_inconsistent_platforms(self):
        for change in ("missing", "extra", "linux-drift", "moving-compat", "jr-compat", "schema"):
            policy = ReleasePolicyTests.policy(self)
            targets = policy["release"]["targets"]
            if change == "missing":
                del targets[p.TARGETS[0]]
            elif change == "extra":
                targets["unknown"] = targets[p.TARGETS[0]]
            elif change == "linux-drift":
                targets[p.TARGETS[0]]["toolchain"] = "jr-1234567"
            elif change == "schema":
                policy["release"]["schema"] = True
            else:
                targets[p.TARGETS[2]]["toolchain"] = "stable" if change == "moving-compat" else "jr-ac20de5e7921"
            with self.subTest(change=change), self.assertRaises(ValueError):
                p.parse(json.dumps(policy))

    def test_fetch_failure_has_no_cached_or_fallback_policy(self):
        with tempfile.TemporaryDirectory() as tmp:
            output = Path(tmp) / "policy.json"
            source = Path(tmp) / "policy.rs"
            source.write_text("pub const SUPPORTED: std::ops::RangeInclusive<u32> = 1..=1;")
            args = ["release-policy.py", "freeze", "--output", str(output), "--protocol-source", str(source)]
            with patch.object(p.sys, "argv", args), patch.object(p.subprocess, "run", side_effect=p.subprocess.CalledProcessError(22, "curl")):
                with self.assertRaises(p.subprocess.CalledProcessError):
                    p.main()
            self.assertFalse(output.exists())
            response = p.subprocess.CompletedProcess([], 0, stdout=json.dumps(self.policy).encode())
            with patch.object(p.sys, "argv", args), patch.object(p.subprocess, "run", return_value=response) as fetch, patch("sys.stdout", new_callable=io.StringIO):
                p.main()
                self.assertEqual(fetch.call_count, 1)
                self.assertIn("--proto", fetch.call_args.args[0])
                self.assertIn("--max-time", fetch.call_args.args[0])
            self.assertEqual(p.validate(json.loads(output.read_text()), self.sha), self.snapshot)

    def test_candidate_protocol_validation(self):
        with tempfile.TemporaryDirectory() as tmp:
            source = Path(tmp) / "policy.rs"
            source.write_text("pub const SUPPORTED: std::ops::RangeInclusive<u32> = 1..=1;")
            p.check_protocol_source(self.policy, source)
            source.write_text("pub const SUPPORTED: std::ops::RangeInclusive<u32> = 2..=3;")
            with self.assertRaises(ValueError):
                p.check_protocol_source(self.policy, source)

    def test_snapshot_identity_and_drift(self):
        self.assertEqual(p.validate(self.snapshot, self.sha), self.snapshot)
        drift = p.freeze(ReleasePolicyTests.policy(self, "jr-123456789abc"))
        with self.assertRaisesRegex(ValueError, "drift"):
            p.validate(drift, self.sha)
        drift["policy"]["release"]["targets"][p.TARGETS[2]]["toolchain"] = "stable"
        with self.assertRaises(ValueError):
            p.validate(drift, p.digest(drift))

    def test_explicit_override_precedence_and_no_per_job_fetch(self):
        with patch.dict(os.environ, {"JUSTRUST_TOOLCHAIN": "system", "RUSTUP_TOOLCHAIN": "stable"}), patch.object(p.subprocess, "run", side_effect=AssertionError("per-job fetch")):
            env = dict(os.environ)
            env.update(p.environment(self.snapshot, p.TARGETS[0]))
            self.assertEqual(env["JUSTRUST_TOOLCHAIN"], self.policy["toolchain"])
            self.assertEqual(env["JUSTRUST_TOOLCHAIN_REQUIRED"], "1")
            self.assertEqual(env["JUSTRUST_TOOLCHAIN_POLICY"], "off")
            for target in p.TARGETS[:2]:
                self.assertEqual(p.selected(self.snapshot, target), self.policy["toolchain"])

    def test_bootstrap_and_platform_compatibility_are_separate(self):
        self.assertEqual(self.snapshot["bootstrap"], "1.99.0")
        self.assertNotEqual(p.selected(self.snapshot, p.TARGETS[0]), self.snapshot["bootstrap"])
        for target in p.TARGETS[2:]:
            self.assertEqual(p.selected(self.snapshot, target), "1.99.0")
        workflow = (Path(__file__).parents[1] / ".github/workflows/release.yml").read_text()
        self.assertNotIn("rustup default stable", workflow)
        self.assertNotIn("JUSTRUST_TOOLCHAIN: 1.99.0", workflow)
        self.assertIn('--bin justrust "${build_std[@]}" $DIST_FLAGS', workflow)
        self.assertIn('if [[ "$JUSTRUST_TOOLCHAIN" == jr-* ]]; then', workflow)
        self.assertIn('stage-c-runtime', workflow)
        self.assertIn('-Clink-arg=--fix-cortex-a53-843419', workflow)
        self.assertIn('build_std=(-Zbuild-std)', workflow)
        self.assertNotIn('build_command=zigbuild', workflow)
        self.assertIn('!/toolchain/', workflow)
        self.assertIn("check-linux-release.sh", workflow)
        self.assertIn('lto="thin"', workflow)
        self.assertIn('ref: ${{ needs.preflight.outputs.sha }}', workflow)
        self.assertIn('[[ "$current" == "$SHA" ]]', workflow)
        self.assertIn('rustup toolchain install 1.99.0 --profile minimal --target', workflow)
        self.assertIn('It never supplies Rust rlibs to the service-selected compiler', workflow)

    @unittest.skipUnless(shutil.which("bash"), "bash required for workflow command test")
    def test_workflow_uses_frozen_compiler_for_both_linux_targets(self):
        workflow = (Path(__file__).parents[1] / ".github/workflows/release.yml").read_text()
        command = workflow.split("          build_std=()", 1)[1].split("          mkdir -p dist", 1)[0]
        command = "build_std=()\n" + textwrap.dedent(command)
        for target in p.TARGETS[:2]:
            for compiler in ("jr-ac20de5e7921", "1.99.0"):
                env = dict(os.environ, JUSTRUST_TOOLCHAIN=compiler, DIST_FLAGS="", RUNNER_TEMP="/unused", POLICY_SHA256=self.sha)
                script = 'set -euo pipefail\njustrust() { printf "%s\\n" "$@"; }\npython3() { if [[ $1 == -c ]]; then command python3 "$@"; fi; }\n' + command.replace("${{ matrix.target }}", target)
                result = p.subprocess.run(["bash", "-c", script], env=env, text=True, stdout=p.subprocess.PIPE, check=True)
                args = result.stdout.splitlines()
                self.assertEqual(args[0], "build")
                self.assertEqual(args[args.index("--target") + 1], target)
                self.assertEqual("-Zbuild-std" in args, compiler.startswith("jr-"))
                flags = []
                if "--config" in args:
                    config = args[args.index("--config") + 1]
                    key, value = config.split("=", 1)
                    self.assertEqual(key, f"target.{target}.rustflags")
                    flags = json.loads(value)
                self.assertEqual("-Clink-arg=--fix-cortex-a53-843419" in flags, target.startswith("aarch64-"))
                self.assertEqual(any(flag.startswith("-Lnative=") for flag in flags), compiler.startswith("jr-"))

    def c_source(self, directory, target):
        directory.mkdir()
        header = bytearray(64)
        header[:6] = b"\x7fELF\x02\x01"
        header[18:20] = (62 if target.startswith("x86_64-") else 183).to_bytes(2, "little")
        for name in p.C_FILES:
            (directory / name).write_bytes(header if name.endswith(".o") else b"!<arch>\n")
        return directory

    def test_c_runtime_is_c_only_target_matched_and_hash_bound(self):
        for target in p.TARGETS[:2]:
            with tempfile.TemporaryDirectory() as tmp:
                root = Path(tmp)
                source = self.c_source(root / "source", target)
                destination = root / "staged"
                manifest = p.stage_c_runtime(self.snapshot, target, source, destination)
                self.assertEqual(manifest["kind"], "c-abi-only")
                self.assertEqual(manifest["toolchain"], self.snapshot["bootstrap"])
                self.assertEqual(p.check_c_runtime(self.snapshot, target, destination), manifest)
                self.assertFalse(any(path.suffix == ".rlib" for path in destination.iterdir()))
                with self.assertRaises(ValueError):
                    p.stage_c_runtime(self.snapshot, target, source, destination)
                (destination / "libc.a").write_bytes(b"!<arch>\nmodified")
                with self.assertRaisesRegex(ValueError, "drift"):
                    p.check_c_runtime(self.snapshot, target, destination)
                (source / "libstd.rlib").write_bytes(b"not allowed")
                with self.assertRaisesRegex(ValueError, "C-only"):
                    p.stage_c_runtime(self.snapshot, target, source, root / "bad")

    def test_c_runtime_refuses_wrong_architecture_symlinks_and_unpinned_source(self):
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            source = self.c_source(root / "source", p.TARGETS[0])
            with self.assertRaisesRegex(ValueError, "architecture"):
                p.c_files(source, p.TARGETS[1])
            (source / "crt1.o").unlink()
            (source / "crt1.o").symlink_to(source / "crti.o")
            with self.assertRaisesRegex(ValueError, "non-symlink"):
                p.c_files(source, p.TARGETS[0])
        response = p.subprocess.CompletedProcess([], 0, stdout="release: 1.100.0\n")
        with patch.object(p.subprocess, "run", return_value=response) as run:
            with self.assertRaisesRegex(ValueError, "frozen exact"):
                p.installed_c_runtime(self.snapshot, p.TARGETS[0])
            self.assertEqual(run.call_args.args[0][2], self.snapshot["bootstrap"])

    def test_attests_actual_recorded_compiler_and_refuses_fallback(self):
        with tempfile.TemporaryDirectory() as tmp:
            runs = Path(tmp)
            run = runs / "release-run"
            run.mkdir()
            unrelated = runs / "unrelated-live-run"
            unrelated.mkdir()
            (unrelated / "meta.json").write_text("")
            with self.assertRaisesRegex(ValueError, "exactly one"):
                p.attest(self.snapshot, p.TARGETS[0], 99, Path.cwd(), runs)
            meta = {"id": "release-run", "start": 100, "cwd": str(Path.cwd()),
                    "args": ["build", "--release", "--target", p.TARGETS[0], "--bin", "justrust"],
                    "place": "hosted", "toolchain": self.policy["toolchain"],
                    "real_rustc": "/home/ubuntu/.justrust/toolchains/jr-ac20de5e7921/bin/rustc"}
            (run / "meta.json").write_text(json.dumps(meta))
            (run / "summary.json").write_text('{"exit":0}')
            binary = runs / "justrust"
            binary.write_bytes(b"release artifact")
            result = p.attest(self.snapshot, p.TARGETS[0], 99, Path.cwd(), runs, binary)
            self.assertEqual(result["binary_sha256"], p.hashlib.sha256(binary.read_bytes()).hexdigest())
            self.assertEqual(result["compiler"], meta["real_rustc"])
            # Current clients leave a speculative prebuild of the same command
            # beside the real run, possibly unfinished. It attests nothing.
            for name, summary in (("release-run-pre", None), ("finished-pre", '{"exit":0}')):
                prebuild = runs / name
                prebuild.mkdir()
                (prebuild / "meta.json").write_text(json.dumps(dict(meta, id=name, start=101)))
                if summary:
                    (prebuild / "summary.json").write_text(summary)
            self.assertEqual(p.attest(self.snapshot, p.TARGETS[0], 99, Path.cwd(), runs)["run_id"], "release-run")
            # A two-slot promotion installs the new compiler under staged/ or managed/staged/.
            for path in (
                "/home/ubuntu/.justrust/toolchains/staged/jr-ac20de5e7921/bin/rustc",
                "/home/ubuntu/.justrust/toolchains/managed/jr-ac20de5e7921/bin/rustc",
                "/home/ubuntu/.justrust/toolchains/managed/staged/jr-ac20de5e7921/bin/rustc",
            ):
                staged = dict(meta, real_rustc=path)
                (run / "meta.json").write_text(json.dumps(staged))
                self.assertEqual(p.attest(self.snapshot, p.TARGETS[0], 99, Path.cwd(), runs)["compiler"], path)
            for key, value in (("toolchain", "1.99.0"), ("place", "local"), ("real_rustc", "/usr/bin/rustc"), ("real_rustc", "/toolchains/staged/other/bin/rustc"), ("real_rustc", "/toolchains/jr-ac20de5e7921/bin/../other/bin/rustc")):
                bad = dict(meta)
                bad[key] = value
                (run / "meta.json").write_text(json.dumps(bad))
                with self.subTest(key=key), self.assertRaises(ValueError):
                    p.attest(self.snapshot, p.TARGETS[0], 99, Path.cwd(), runs)


if __name__ == "__main__":
    unittest.main()
