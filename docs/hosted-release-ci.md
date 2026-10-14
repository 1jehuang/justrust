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
`target/<triple>/<profile>/`. Cross-target and custom JSON targets are rejected.
Do not pass `--target-dir`. A Cargo-configured `build.target` without an explicit
supported `--target` is not supported by artifact discovery.
Debug builds, `run`, `zigbuild`, macOS/Windows binaries, musl cross builds and
other cross targets are not supported by this release artifact path.

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

Both runner and host must be Linux with compatible architecture. The runner
needs Rust/Cargo (metadata), SSH, rsync and zstd. The host must have the project's
native development libraries and exact toolchain. `ldd` checks the downloaded
binary against the runner for missing libraries and too-new glibc, but does not
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
4. Package, sign, checksum and upload only that verified binary on the runner.
   Keep GitHub publishing in a separate existing release job. Retain local
   matrix jobs for Windows, macOS, musl and aarch64 until separately supported.

Hosted builds use the account's normal build allowance and admission rules.
Nothing here changes billing, provisions API secrets, runs a public release,
or certifies a particular application's release ABI. Existing public release
workflows are intentionally unchanged.
