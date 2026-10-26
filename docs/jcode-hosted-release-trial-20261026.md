# Actual Jcode hosted release trial, 2026-10-26

Follow-up: the Linux x86-64 glibc 2.17 blocker identified here was fixed and
verified later the same day. See [hosted glibc 2.17](hosted-glibc217.md) for the
real application builds and baseline-container checks. Other platform gates
below remain separate work.

This trial used the real `~/jcode` workspace, not the small cross-toolchain
fixture. Nothing was signed, tagged, uploaded to a public release, or installed
over the Jcode executable serving the user's session. Existing MSVC, macOS,
ARM64 and portable Linux release jobs remain unchanged.

## Source and build contract

- Base revision: Jcode commit `9a48e25a56cfb1d54acfc546d57d268399de7461`.
  The final build command verified a clean working tree at launch and the same
  HEAD at completion. Other sessions introduced uncommitted TUI changes before
  the final post-build inspection, so these are working-tree trial artifacts,
  not attested immutable snapshots or exact-tag release artifacts.
- Exact compiler: Rust `1.99.0`, through required hosted justrust routing.
- Package/binary: `-p jcode --bin jcode`, `--locked --release`.
- Full default features retained, including PDF and Bedrock. Linux additionally
  enabled the release workflow's `linux-compat-vendored-openssl` feature.
- Release optimization: level 3 and incremental compilation disabled, verified
  in recorded compilation units rather than inferred from the word `release`.
- Public version/commit/date/tag/dirty/changelog metadata supplied explicitly
  from the source checkout. `.git` is not mirrored to hosted builders.
- `Cargo.lock` was not edited. Initial unrelated TUI changes were committed by
  their owning session while the trial ran; no Jcode source was edited here.

These are build and smoke-test observations, not a benchmark comparing
hosted against local compilation or a claim of a general speedup.

## Important release bugs found and fixed

The first real Windows GNU trial succeeded, but its recorded Rust invocations
used `opt_level=1` and incremental compilation despite the caller requesting
level 3 and no incremental compilation. The initial executable also contained
`v0.99.1 (unknown)`, not the source commit.

- `bebd3501e` forwards an explicit allowlist of public Cargo release profile
  controls and `CARGO_INCREMENTAL`. It preserves an empty
  `CARGO_BUILD_RUSTC_WRAPPER` disable request, not an arbitrary local wrapper
  executable or secret environment.
- `f341c107d` forwards explicit `JCODE_BUILD_GIT_HASH`, `DATE`, `TAG` and `DIRTY`
  metadata. Similarly named token/private-directory variables stay excluded.
- Both changes include transport regression coverage and passed hosted scoped
  tests and all-target clippy. The installed client was built from the committed
  source, excluding concurrent uncommitted work in this repository.

## Observed results

### Linux x86-64: compiled and launched, not portable-release compatible

Run `20261026-142835127-347039` built the full application with the Linux
release feature, level 3 and incremental compilation disabled. The hosted
compile took 96.3 seconds, with 107.1 seconds client elapsed including transport.
The downloaded binary launched and reported `jcode v0.99.1 (9a48e25a5)`.
The `cloud receive`, `credentials`, `router-credentials` and `cancel-return`
help commands all exited successfully without running a daemon.

`objdump -T` found a highest required symbol version of **GLIBC_2.39**. The
current official x86-64 Linux release script requires **GLIBC_2.17**, so this
artifact must not replace it. Enabling vendored OpenSSL alone does not make the
rest of the sysroot portable. The manylinux build or an equivalently verified
baseline toolchain remains necessary.

### Windows GNU x86-64: real full-application cross-build

- `20261026-142049697-283659`: initial full-feature application build passed,
  but used the wrong release optimization and unknown Git provenance. This
  is diagnostic evidence only, not a release candidate.
- `20261026-142337498-305063`: full-feature application passed with explicit
  Cargo configuration for level 3 and incremental=false. Recorded root library
  and binary invocations confirm both settings. Client elapsed was 146.5 seconds
  and hosted build elapsed 134.4 seconds. A PE x86-64 executable was downloaded.

- `20261026-143022408-358770`: final Windows build using the fixed environment
  transport passed with level 3, incremental=false, and the expected embedded
  `v0.99.1 (9a48e25a5)` version bytes. Hosted build elapsed was 99.9 seconds;
  client elapsed including transfer was 119.7 seconds. No Windows program was
  executed to check that embedded string.

Final local trial files under `$JCODE_SCRATCH_DIR/jcode-release-trial/`:

| File | SHA-256 |
| --- | --- |
| `jcode-linux-opt3` | `0807d2e89c72cac3f8ff31132aafaadead390e96fa3bc33f19a456ac23e8f2a0` |
| `jcode-windows-gnu-opt3.exe` | `119f57fd62c295d4273a1a4f4f5f590ee9d76e9b1dfe70d30c63138ab00274fd` |

No Windows runtime execution or Authenticode signing is claimed. GNU is an
additional target, not a substitute for the official Windows MSVC ABI.

### Other official targets

Required-hosted build attempts for `x86_64-pc-windows-msvc`,
`aarch64-pc-windows-msvc`, `x86_64-apple-darwin`, `aarch64-apple-darwin`, and
`aarch64-unknown-linux-gnu` were rejected with status 1 before compilation,
without falling back locally. The ARM64 official Linux workflow also requires
its emulated-TLS standard-library build, not merely an ARM target flag.

The current rejection text incorrectly describes some release cross-builds as
"debug and cross build run locally"; it is a routing limitation, not evidence
that the requested release optimization was missing. No platform guards were
removed to get a nominal success.

## Publication decision

Do not migrate the complete release matrix based on this trial. Remaining gates
include the Linux 2.17 sysroot baseline, the existing ARM64 TLS contract,
macOS/MSVC toolchains, native target launch/installer tests, platform signing,
and exact-tag release CI/provenance checks. Successful cross-compilation does
not waive those checks. The generated binaries are local trial artifacts only.
