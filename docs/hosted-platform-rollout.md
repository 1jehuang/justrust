# Hosted release platform rollout (2026-10-26)

This is a build capability rollout, not a claim that all official Jcode release
artifacts have been published, signed, or executed on their native platforms.
The existing release workflow is not replaced by these helpers.

## Platform status

| Target | Implementation | Real Jcode acceptance |
| --- | --- | --- |
| `x86_64-unknown-linux-gnu.2.17` | Managed Zig baseline and ELF version validation | Full release built, downloaded, and exercised in a glibc 2.17 container. See [baseline evidence](hosted-glibc217.md). |
| `x86_64-pc-windows-gnu` | Managed MinGW and validated PE transfer | Prior full-feature Jcode cross-build trial exists, but this is not the official MSVC release target. See [trial](jcode-hosted-release-trial-20261026.md). |
| `x86_64-pc-windows-msvc`, `aarch64-pc-windows-msvc` | Pinned cargo-xwin configuration and architecture-specific PE validation | Actual hosted requests reached setup and correctly refused absent explicit Microsoft SDK-license consent. No MSVC Jcode artifact claimed. |
| `x86_64-apple-darwin`, `aarch64-apple-darwin` | Inventory-checked host-local SDK import, pinned Zig, architecture-specific Mach-O validation | Actual hosted requests correctly refused absent a managed Apple SDK. No full macOS Jcode artifact claimed. |
| `aarch64-unknown-linux-gnu` | Managed GNU cross-toolchain, rebuilt standard library and target-only emulated-TLS enforcement | Full-feature Jcode built and downloaded, including vendored OpenSSL. ELF/TLS structure verified, requires glibc 2.39. Native execution not tested. See [trial](jcode-arm64-hosted-trial-20261026.md). |

For MSVC, see [managed Windows MSVC](managed-windows-msvc.md). The license
consent is an operator decision, not an environment variable that the agent may
silently set. LLVM tools are also build-host prerequisites. For Darwin, see
[managed Apple SDKs](managed-darwin.md). A signing certificate does not replace
SDK headers, libraries, or framework stubs. Do not forward arbitrary SDK paths or
private credentials from clients to compile workers.

For GNU ARM64, see [the TLS and toolchain contract](managed-linux-arm64.md).
Do not suppress `--fix-cortex-a53-843419`, use stock prebuilt standard libraries,
or put target TLS options into host-global Rust flags merely to get a link to
succeed. Preserve the official release's rebuilt-std/emulated-TLS contract.

## Verification so far

The integrated native GNU platform implementation passed 808 tests, with five ignored and
none failed, on hosted run `20261026-151514353-642553`. Strict clippy passed
on run `20261026-151526211-642552`. This is source-level
regression evidence, not execution of Windows, macOS, or ARM64 release binaries.
The actual Windows/MSVC and Darwin requests were run with remote execution
required and returned nonzero for their missing prerequisites. No fallback
artifact was accepted as a successful cross-build.

## Signing without exposing keys to builds

The manual [trusted signing handoff](trusted-release-signing.md) accepts a
trusted artifact hash plus an already-installed native signing identity. It is
plan-only by default. Explicit execution signs a private copy, verifies it with
the native verifier, and publishes to a new path without replacing the unsigned
input. It does not accept raw private keys, PFX passwords, or credentials in
build arguments. Its 13 mocked tests passed; real signing is still unverified.

The helper currently needs a trusted macOS signing host for Developer ID or a
trusted Windows signing host with a certificate-store identity and SignTool.
This deliberately keeps signing keys off general-purpose compile workers.
macOS notarization, Windows reputation, native runtime testing, packaging, and
release publication are separate acceptance gates. Cross-compilation avoids
needing a separate machine for every architecture's compile stage, not every
platform-specific release operation.
