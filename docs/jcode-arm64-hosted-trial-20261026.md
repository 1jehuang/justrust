# Full Jcode ARM64 hosted build trial (2026-10-26)

## Result

A real full-feature Jcode ARM64 executable was built on the existing hosted
Linux x86_64 machine and returned through the validated artifact path. This is
**compile and artifact-validation success**, not native execution, Termux
acceptance, signing, packaging, or publication.

- Jcode source: clean Git archive of `eaba9240aa4a02401a113debb3cce463bf335074`.
  Active uncommitted work in the Jcode checkout was not modified or included.
- justrust implementation: `d2778bd03` and `18127788b`.
- Compiler: exact managed `nightly-2026-10-08`, with matching rust-src.
- GNU toolchain: checksum-pinned, account-local GCC 13.2 and binutils 2.42,
  with an ARM64 glibc 2.39 sysroot. No system package installation required.
- Features: default `pdf` and `bedrock`, plus Jcode's existing
  `linux-compat-vendored-openssl` feature. No default feature was disabled.
- Cargo: `build --locked --release -Zbuild-std=std,panic_abort
  --target aarch64-unknown-linux-gnu -p jcode --bin jcode
  --features linux-compat-vendored-openssl`.
- Release profile opt-level 3, incremental disabled, repository development
  rustc wrapper explicitly disabled. Public release metadata was supplied from
  that exact revision through the existing `JCODE_BUILD_*` environment fields.

The first default-feature attempt, run `20261026-151916181-673081`, failed at
cross OpenSSL discovery. The retry uses the project's existing vendoring option
so OpenSSL is built for ARM64 rather than discovering host x86_64 libraries.
This retains OpenSSL functionality instead of disabling its dependents.

## Recorded measurements

These are individual build observations, not a comprehensive speedup claim.

| Run | Result |
| --- | --- |
| `20261026-152034595-679135` | Hosted full release succeeds, 139.78 seconds recorded build wall time. End-to-end script including sync, download and artifact inspection: 154.31 seconds. |
| `20261026-152323026-695569` | Same revision and flags: 630 fresh units, zero compiled. Hosted client reports 2.2 seconds. Same artifact hash. |

The first successful build is not a pristine cold-cache measurement: the failed
OpenSSL attempt had already compiled some dependencies. Both runs required
hosted execution, with no local build fallback.

## Artifact inspection

- SHA256: `d106ceae55fdbd32c67d82bf6c8f4b33505bd67c73e8785714861699f5e3ca0b`.
- Size: 173,831,288 bytes, unstripped.
- ELF64 little-endian AArch64 PIE, interpreter `/lib/ld-linux-aarch64.so.1`.
- No `PT_TLS` program header.
- Application emulated-TLS symbols present, plus
  `__emutls_get_address@GCC_4.3.0` imported from `libgcc_s.so.1`.
- Dynamic dependencies: `libgcc_s.so.1`, `libm.so.6`, `libc.so.6`, and the loader.
  No dynamic OpenSSL dependency.
- Maximum imported glibc symbol version: **GLIBC_2.39**. This is not the x86_64
  glibc 2.17 baseline and must not be advertised as compatible with older ARM64
  glibc environments.

The managed helper preserves Rust's Cortex-A53 linker workaround and always
requires rebuilt `std` and `panic_abort`, injecting target-only emulated TLS.
Host tools retain native TLS. See [managed ARM64](managed-linux-arm64.md).

## Reproduce from the chosen source revision

Set public `JCODE_BUILD_*` metadata as in the release workflow, then run from a
clean checkout or archive:

```sh
JUSTRUST_REMOTE_REQUIRED=1 \
JUSTRUST_TOOLCHAIN=nightly-2026-10-08 \
JUSTRUST_TOOLCHAIN_REQUIRED=1 \
CARGO_BUILD_RUSTC_WRAPPER='' \
CARGO_PROFILE_RELEASE_OPT_LEVEL=3 \
CARGO_INCREMENTAL=0 \
JCODE_RELEASE_BUILD=1 JCODE_CI_BUILD=1 \
justrust build --remote --locked --release \
  -Zbuild-std=std,panic_abort --target aarch64-unknown-linux-gnu \
  -p jcode --bin jcode --features linux-compat-vendored-openssl
```

Inspect the returned artifact with `readelf -h`, `readelf -lW`, `readelf -d`,
`readelf -V`, and `nm`. Do not run it directly on the x86_64 build host.
The retained local trial artifact is under the session scratch directory's
`jcode-arm64-trial/jcode`.

Native ARM64 execution, multithreaded runtime behavior, and the project's
specific Termux loader compatibility still require target-side validation.
The release workflow was not changed to publish this artifact automatically.
