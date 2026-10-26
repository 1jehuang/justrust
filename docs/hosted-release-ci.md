# Hosted release binaries in GitHub Actions

The API behind `jcode.sh/rust` provides authenticated, account-isolated Linux
build hosts, not a stateless HTTP upload/download build endpoint. The client
admits a build through `build/runs`, obtains pinned SSH host keys through
`build/host/connect`, mirrors source, runs Cargo, and downloads binaries over
that same tenant SSH connection. No cloud protocol change is required.

## Command contract

On a fresh Linux runner, `JCODE_API_KEY` selects hosted automatically. Existing
backend configuration takes precedence, so `justrust remote use hosted` makes
that selection explicit. Store an account API key as a GitHub Actions secret,
not in source, logs, an artifact, or a fork PR workflow.

```sh
justrust remote use hosted
JUSTRUST_REMOTE_REQUIRED=1 JUSTRUST_TOOLCHAIN_REQUIRED=1 JUSTRUST_TOOLCHAIN=1.90.0 \
  justrust build --remote --release --locked --target x86_64-unknown-linux-gnu --bin my-app
test -x target/x86_64-unknown-linux-gnu/release/my-app
```

Replace the illustrative Rust version with the exact tested project version.
`JUSTRUST_REMOTE_REQUIRED=1` forces remote routing and exits nonzero instead of
running locally for unsupported commands, missing prerequisites, admission or
infrastructure errors, and failed artifact downloads. `--local` conflicts with
this requirement and fails. Without this opt-in, ordinary fail-open routing is
unchanged. `--remote` alone still permits infrastructure fallback.

Supported commands: `build --release`, `build --profile <name>` (except dev/debug),
plus `check`, `clippy`, and `test`, using Cargo's normal selectors (`-p`, `--bin`,
`--manifest-path`, features, `--locked`, `--config`). Outputs are under Cargo's
resolved target directory, `target/release/` or `target/<profile>/` by default.
An explicit native GNU/Linux `--target` is supported when its architecture
matches the Linux runner and hosted machine (`x86_64-unknown-linux-gnu` or
`aarch64-unknown-linux-gnu`). These outputs are under
`target/<triple>/<profile>/`. Explicit static musl release targets are described
below. Other targets on this native `build` path and custom JSON targets are rejected.
Do not pass `--target-dir`. A Cargo-configured `build.target` without an explicit
supported `--target` is not supported by artifact discovery.
Debug builds, `run`, macOS/Windows binaries, and arbitrary cross targets are
not supported by this release artifact path.

## Static Linux musl releases

The native release path supports exactly
`x86_64-unknown-linux-musl` and `aarch64-unknown-linux-musl`:

```sh
justrust remote use hosted
JUSTRUST_REMOTE_REQUIRED=1 JUSTRUST_TOOLCHAIN_REQUIRED=1 JUSTRUST_TOOLCHAIN=1.99.0 \
  justrust build --remote --release --locked --target aarch64-unknown-linux-musl --bin my-app
```

The client installs the selected official compiler's matching target std
library, checksum-verifying the version's manifest and component archive. It
does not borrow libraries from the host's default rustup toolchain. Custom
`jr-*` compilers must already include their own matching target libraries.
The native path selects `rust-lld` from that exact compiler's host sysroot and
disables the development linker shim. Rust's target std supplies its static
musl CRT and libc. No system linker, Zig, root access, Python/pip installation,
cloud deployment or protocol change is needed for this repository's Rust-only
dependencies. An absent pinned linker is an error, not a system fallback.
`JUSTRUST_TOOLCHAIN_AUTO_INSTALL=0` prevents automatic target downloads as well.

A separate compatibility `zigbuild` command installs checksum-pinned Zig 0.13.0
and cargo-zigbuild 0.20.1 in the account toolchain store. Its compiler selector
disables Python-package discovery. This is not the release workflow: real ARM
validation with Rust 1.99.0 fails because Zig rejects rustc's
`--fix-cortex-a53-843419` linker option. Newer cargo-zigbuild filters that option,
but dropping a CPU erratum mitigation is not the fix adopted here. Native
`rust-lld` accepts the option unchanged.

Return paths remain `target/<triple>/<profile>/`. Before replacing
a local file, the client checks the ELF64 machine architecture and rejects
`PT_INTERP` and `DT_NEEDED` entries. It does not attempt to run an ARM artifact
with the runner's native `ldd`. Static PIE is accepted. Unknown local outputs
are not reused as evidence of a matching hosted release.

This path supports optimized builds with one explicit target and no
`--target-dir`. It is for static executables, not dynamically linked musl
applications or cdylibs. Projects needing additional native libraries must
still provide compatible static libraries and verify their build-script and
proc-macro requirements. Do not substitute GNU artifacts for musl releases.

## Artifact transfer

Executable files, shared libraries, and un-hashed example binaries are fetched,
including warm-host outputs that predate this CI invocation. Downloads verify
zstd content checksums and size and replace files atomically. Previously fetched
unchanged outputs are skipped, changed outputs may use zstd deltas. The stream
scans the profile directory, not Cargo's selected-target manifest: it can also
fetch older unselected binaries. Package only explicitly expected names after
Cargo succeeds, never glob the whole profile directory. Static `.a`, `.rlib`,
`.rmeta`, debug symbols, installers, and packages are not fetched.

## Toolchain and environment

For final published binaries, set `JUSTRUST_TOOLCHAIN` to the exact tested
numeric version and `JUSTRUST_TOOLCHAIN_REQUIRED=1`. justrust installs its
checksum-verified official toolchain on the host if needed. Required mode refuses
missing, invalid or substituted pins before Cargo starts, even when ordinary
installation would fail open to the system compiler. Set
`JUSTRUST_TOOLCHAIN_AUTO_INSTALL=0` too if CI must use only preinstalled versions.
A checked-in justrust pin can also be used with required mode.

`RUSTUP_TOOLCHAIN` is forwarded and suppresses the hosted store default, but is
not sufficient for release pinning: a standalone Cargo/rustc installation can
ignore it. The explicit justrust pin selects exact compiler paths and removes
`RUSTUP_TOOLCHAIN` from Cargo's environment. Do not use
`JUSTRUST_TOOLCHAIN=system` for a strict release pin. Without explicit pinning,
hosted builds may use justrust's store toolchain, intended for iteration rather
than an implicit release contract.

Public release controls forwarded include `RUSTUP_TOOLCHAIN`, `RUSTFLAGS`,
`CARGO_ENCODED_RUSTFLAGS`, `JUSTRUST_RELEASE_BUILD`, `JCODE_DESKTOP_VERSION`,
`JCODE_DESKTOP_BUILD_METADATA`, `SOURCE_DATE_EPOCH`, `JCODE_RELEASE_BUILD`,
`JCODE_BUILD_METADATA_FILE`, `JCODE_BUILD_SEMVER`, `JCODE_BUILD_CHANGELOG_RAW`,
and the existing justrust optimization controls. Metadata file inputs must point
to files inside mirrored workspace roots. Prefer a workspace-relative path and
generate a nonignored snapshot before invoking justrust. Prefer `--config` for Cargo
profile settings, since arbitrary `CARGO_PROFILE_*` variables are not forwarded.
API keys and publishing credentials are not forwarded. Arbitrary build-script
environment variables are not forwarded, and `.git` is not mirrored. Projects
that embed git history or commit metadata must supply supported explicit metadata
inputs or validate their fallback behavior before publishing. Do not assume git
seed optimization gives build scripts the checkout's git metadata.

Both runner and host must be Linux. Native GNU builds require matching
architecture; the explicit musl path supports the two target architectures
above independently of the supported Linux build-host architecture. The runner
needs Rust/Cargo (metadata), SSH, rsync and zstd. The host must have the project's
native development libraries and exact toolchain. `ldd` checks the downloaded
GNU binary against the runner for missing libraries and too-new glibc, but does not
certify compatibility with all end-user distributions. Native GNU/Linux hosted
outputs are not automatically portable musl binaries.

## Recommended CI adoption

1. Add a manually dispatched, nonpublishing validation job first. Pin justrust
   to a reviewed immutable commit containing this support, install with
   `cargo install --path <checkout> --locked`, and pin actions and Rust versions
   according to the repository's supply-chain policy.
2. Configure a `JCODE_API_KEY` secret for an entitled account. Restrict this job
   to trusted refs. Set a timeout, serialize builds sharing a checkout path, and
   use least privilege (`contents: read` is sufficient for a validation job).
3. Build with the strict command above, verify the expected binary exists, run
   an application version/smoke test, inspect its ELF architecture and dynamic
   dependencies, and compare embedded release metadata with the requested ref.
4. For musl, also inspect the target's ELF machine and absence of a dynamic
   loader and shared-library dependencies. Run native and ARM/QEMU version and
   help smoke tests. `ci/check-linux-release.sh` performs these checks.
5. Package, sign, checksum and upload only that verified binary on the runner.
   Keep GitHub publishing in the separate existing release job. Keep Windows
   and macOS on their existing local jobs.

## Validation (2026-10-26 UTC)

The native path was exercised on the real authenticated hosted service with
Rust 1.99.0 and the release workflow's thin-LTO, one-codegen-unit, stripped
profile. Both returned artifacts passed ELF64 machine, no `PT_INTERP`, no
`DT_NEEDED`, exact `0.1.0-alpha.19` version and help checks. ARM execution used
QEMU user emulation on the x86_64 runner, not just header inspection.

| Target | Run | Hosted wall | Result |
| --- | --- | ---: | --- |
| aarch64 musl | `20261025-225716573-42029` | 114.7 s | Static artifact and QEMU smoke passed |
| x86_64 musl | `20261025-225911440-55033` | 110.7 s | Static artifact and native smoke passed |

Warm-host repeats `20261025-230131206-70695` (ARM) and
`20261025-230138968-71527` (x86_64) compiled no units and passed the same
artifact/smoke checks. x86_64 reused its known local artifact; ARM was fetched
again and revalidated.

These are correctness-validation observations, not a before/after benchmark or
speedup claim. The failed Zig ARM run `20261025-225053880-4189670` motivated the
native linker path rather than removal of its unsupported erratum option.
Hosted unit tests passed 720 tests (3 ignored), the routing/artifact/toolchain
integration selection passed 33 tests, and all-target Clippy passed with
warnings denied. A local static-musl fixture stayed `Fresh` when switching from
recorded to `JUSTRUST_DISABLE=1` execution. Unrecorded Zig initialization was
also exercised. Local execution here is specifically the recording-parity
check; the release artifact checks above ran through hosted admission.

GitHub Actions validation has not run: the repository secret list contains no
`JCODE_API_KEY` and the `JUSTRUST_HOSTED_RELEASES` variable is unset. Keep the
gate off until an authorized durable key is provisioned and the manual CI run
passes. The immutable bootstrap client is
`d0c35300773df0103e3beeb090a5a1a5c6681185`. No public release was created.

## Repository rollout gate

`.github/workflows/hosted-linux-validation.yml` is manually dispatched and does
not publish releases. It builds and smoke-tests both musl targets through the
hosted service using an immutable bootstrap client and Rust 1.99.0. The bootstrap
is compiled with `JUSTRUST_RELEASE_BUILD=0`; only the binaries being released
receive `JUSTRUST_RELEASE_BUILD=1`. Credentials are not given to the bootstrap
compiler or forwarded to hosted build scripts.

The Linux jobs in `release.yml` select this hosted path only when the repository
variable `JUSTRUST_HOSTED_RELEASES` equals `1`. Before setting it, configure the
durable `JCODE_API_KEY` repository secret and require a successful nonpublishing
validation run for both targets. An absent secret fails before bootstrap/build,
not by silently compiling locally. With the variable unset, the original local
Linux zigbuild jobs remain active. macOS and Windows compile/package commands
are unchanged. Hosted jobs sharing the checkout path are serialized.

As of the initial implementation, the repository had no `JCODE_API_KEY` secret.
Local authenticated hosted validation is not proof that GitHub Actions has
credentials. The gate must stay off until that independent prerequisite and
the nonpublishing CI run are verified. No new public release is required to
install the reviewed bootstrap client.

Hosted builds use the account's normal build allowance and admission rules.
Nothing here changes billing, provisions API secrets, runs a public release,
or certifies a particular application's release ABI. The existing public release
publishing step remains unchanged. This document describes the supported
client path and a gated rollout, not an already-promoted Linux release service.
