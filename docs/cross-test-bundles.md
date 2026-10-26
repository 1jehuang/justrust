# Cross-target test bundles

`scripts/cross-test-bundle.py` is a standalone Python 3.9+ standard-library
helper. It separates **compiled, not tested** artifacts from a later local test
run. It neither builds code nor selects a cross toolchain, downloads libraries,
starts a hosted build, or executes anything during `bundle`.

This does **not** enable hosted Windows or macOS builds today. A Linux builder
must already have a working cross compiler, target standard library, linker,
SDK and any native dependencies required by the project. This helper only
packages the resulting executables. It also works for same-target builds.

## Build, bundle, transfer, run

Capture the successful Cargo JSON stream from your existing build workflow.
For example, where the build runner preserves Cargo JSON on stdout:

```sh
justrust test --no-run --target x86_64-unknown-linux-gnu \
  --message-format=json > cargo-messages.jsonl
# Check the build command's exit status before continuing.
python3 scripts/cross-test-bundle.py bundle \
  --target x86_64-unknown-linux-gnu \
  --messages cargo-messages.jsonl \
  --artifact-root "$PWD/target" \
  --out test-bundle
```

The input must be actual newline-delimited Cargo JSON, not a human-oriented
justrust report. If a runner decorates or replaces stdout, obtain its raw Cargo
JSON instead. A successful `build-finished` event is required, and any failed
`build-finished` event rejects the stream. At least one `compiler-artifact`
with `profile.test: true` and a nonempty `executable` is required. `--target`
is an explicit assertion by the caller, not binary-format detection. Never
mix streams from different builds or targets.

Executable paths are interpreted relative to the invoking directory if not
absolute. Each must be a regular non-symlink file under the resolved explicit
`--artifact-root`. Symlink path components are rejected. No unrelated Cargo
filenames, library artifacts, source files or environment files are copied.
The output directory must not exist. Copy the directory intact using your
trusted transfer mechanism, together with the helper, to the target machine.
No archive extraction is implemented or needed.

On the matching target machine:

```sh
python3 scripts/cross-test-bundle.py run \
  --bundle test-bundle --results test-results.json
# Or forward arguments verbatim to every libtest executable:
python3 scripts/cross-test-bundle.py run \
  --bundle test-bundle --results filtered-results.json \
  --cwd /path/to/project -- my_test --exact --nocapture
```

**Only run trusted bundles.** Invoking `run` explicitly consents to arbitrary
code execution with your account's permissions. Hashes detect corruption,
not malicious producers. This is not a sandbox or an authenticity signature.
Keep the bundle and its parent directories protected against concurrent
modification. Validation is not a defense against an attacker changing files
between validation and execution.

## Compatibility and runtime limits

The allowlist covers `x86_64` and `aarch64` for Linux GNU and musl, Windows
MSVC, and Apple Darwin. `run` requires the allowlisted target's OS and CPU
architecture to match `sys.platform` and `platform.machine()` exactly after
normalizing `AMD64` to `x86_64` and `arm64` to `aarch64`. It does not silently
use emulation or Rosetta. This check does not prove libc, OS version, CPU
features, SDK or dynamic-loader compatibility.

Only self-contained **test executable bundles** are supported initially:

- No resource fixtures, working-tree files, native shared libraries, runtime
  DLLs, dynamic Rust libraries, debug symbols or SDK assets are bundled.
- OS runtime libraries required by the executable must already be installed.
  GNU versus musl runtime availability is not probed.
- The default working directory is the directory invoking `run`, not the
  bundle directory. `--cwd` overrides it for every executable. Projects whose
  tests depend on source paths or fixtures must arrange those themselves.
- This handles executable test harnesses, not doctests or an entire Cargo test
  orchestration. Build scripts and proc macros ran during the original build,
  not during bundling. Cross-build correctness remains the builder's job.
- Environment and standard streams are inherited. Every test executable runs
  sequentially using a subprocess argument list, never a shell. There is no
  timeout or per-test-function parser. A hanging test must be interrupted.

## Manifest and results

`manifest.json` schema version 1 contains `target`, `build_status: compiled`,
`test_status: not-tested`, and `tests` entries with a relative `bin/...` path,
byte size and lowercase SHA-256. No source absolute paths are recorded.
Duplicate JSON keys, unknown fields/schema/targets, empty or oversized test
lists, malformed entries, traversal paths and symlinks are rejected. The
manifest is limited to 4 MiB and 4096 executables. Cargo JSON lines are limited
to 4 MiB. All sizes and hashes are validated **before any execution**.

`run` never edits the manifest. The required `--results` is a new file outside
the bundle and must not already exist or use symlink components. Its separate
schema version 1 records the manifest hash, target, each executable's exit code
(or spawn error), and aggregate `test_status: passed|failed`. A result means
only that the requested invocations succeeded or failed. In particular,
filtered runs, `--list`, ignored tests and harness-specific arguments do not
prove that every project test passed. Full harness output remains on the
inherited console streams.

Exit status is 0 for successful bundling or all-zero test executable exits,
1 for any failed test executable or spawn error, and 2 for invalid input,
platform mismatch or ordinary filesystem errors. All executables are attempted
after an ordinary test failure. Interrupted runs leave a failed partial result
when Python can finish its cleanup, but hard termination may leave an incomplete
result file. A compile-only bundle never receives a tests-passed label.

## Helper regression tests

```sh
python3 -m unittest discover -s tests -p test_cross_test_bundle.py -v
```

These use fake executable bytes and mocked subprocess calls to exercise the
trust boundary without accidentally executing cross-built code. Real compatible
binaries can be exercised through the explicit `run` workflow above.

A local Linux x86_64 smoke workflow was also exercised with a small Rust crate:
`justrust test --local --no-run --target x86_64-unknown-linux-gnu
--message-format=json` supplied real Cargo events and local artifacts, then the
helper bundled and ran the harness. A selected passing test returned helper
status 0, and an explicitly selected failing test returned helper status 1
with the harness's exit code 101 recorded. This verifies same-target local
execution, not a Windows/macOS cross compilation or hosted artifact delivery.
Local mode was required here to keep fixture artifacts available for bundling.
