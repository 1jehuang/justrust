# Cross-target test bundles

`justrust test-bundle pack` and `justrust test-bundle run` are native commands
that separate **compiled, not tested** artifacts from a later local test run.
They neither build code nor select a cross toolchain, download libraries,
start a hosted build, or execute anything during `pack`. Normal `justrust test`
semantics are unchanged. The standalone Python 3.9+ standard-library helper
`scripts/cross-test-bundle.py` remains interoperable with the native version-1
manifest and result schemas. Its `bundle` operation corresponds to native
`pack` (also accepted as native `bundle`).

Hosted native Linux `test --no-run` returns the selected executables and rewrites
Cargo JSON to downloaded local paths. Explicit Windows GNU builds and
compile-only tests are also supported on x86_64 Linux workers through the
[managed cross-toolchain path](managed-windows-gnu.md). This is not hosted macOS
or Windows MSVC support. Bundling alone neither provisions tools nor proves
runtime compatibility.

## Build, bundle, transfer, run

Capture the successful Cargo JSON stream from your existing build workflow.
For example, where the build runner preserves Cargo JSON on stdout:

```sh
justrust test --no-run --target x86_64-unknown-linux-gnu \
  --message-format=json > cargo-messages.jsonl
# Check the build command's exit status before continuing.
justrust test-bundle pack \
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
trusted transfer mechanism, together with the justrust binary for the target
host (or the Python helper), to the target machine.
No archive extraction is implemented or needed.

On the matching target machine:

```sh
justrust test-bundle run \
  --bundle test-bundle --results test-results.json
# Or forward arguments verbatim to every libtest executable:
justrust test-bundle run \
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
MSVC, and Apple Darwin, plus `x86_64-pc-windows-gnu`. `run` requires the
allowlisted target's OS and CPU architecture to match the native runner's
OS and architecture (`std::env::consts::OS` and `ARCH`). The Python helper
checks `sys.platform` and `platform.machine()`, normalizing `AMD64` to
`x86_64` and `arm64` to `aarch64`. Neither path silently
uses emulation or Rosetta. This check does not prove libc, OS version, CPU
features, SDK or dynamic-loader compatibility.

Native paths additionally reject parent (`..`) components in all caller paths,
even when they would resolve inside the intended root. Use normalized paths.
The native runner checks the architecture of its own binary, not the physical
CPU underneath an emulator. Use a native justrust binary on the execution host.

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
only that the requested invocations succeeded or failed. Non-test harness
modes `--list`, `--help`, `-h` and `--bench` are rejected before execution with
status 2 and no result file, rather than being mislabeled passed. Combined
short help flags such as `-qh` are also rejected by the native runner. Filtered
runs, ignored tests and other harness-specific arguments still do not prove
that every project test passed. Full harness output remains on the
inherited console streams.

Exit status is 0 for successful bundling or all-zero test executable exits,
1 for any failed test executable or spawn error, and 2 for invalid input,
platform mismatch or ordinary filesystem errors. All executables are attempted
after an ordinary test failure. The native runner writes an initial failed
record before execution, then replaces that same newly reserved record with
the completed result. Interrupted runs can leave this conservative partial
record. The Python helper writes its partial result when cleanup runs. Hard
termination during a write may leave an incomplete result file. A compile-only
bundle never receives a tests-passed label.

## Regression and acceptance tests

```sh
justrust test --test test_bundle
python3 -m unittest discover -s tests -p test_cross_test_bundle.py -v
# This fixture needs local artifacts, hence its explicit --local build.
bash ci/check-test-bundle.sh "$PWD/target/debug/justrust"
```

The Rust CLI tests verify strict schemas, paths, symlinks, size bounds,
all-artifact validation, host rejection, immutable manifests, result overwrite
refusal, forwarded arguments, successful and deliberately failing execution,
and both native-pack/Python-run and Python-pack/native-run interoperability.
The CI fixture compiles a real Rust libtest executable from Cargo JSON, runs
one successful and one deliberately failing selection using the native CLI,
and checks manifest and result status independently. These checks establish
same-host execution. Separate hosted acceptance runs documented in
[cross-compilation-rollout.md](cross-compilation-rollout.md) verify Linux and
Windows GNU compilation/download, not Windows or macOS runtime execution.
Python helper unit tests retain mocked-execution trust-boundary
coverage without accidentally executing cross-built code.
