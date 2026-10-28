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
| `x86_64-apple-darwin`, `aarch64-apple-darwin` | Inventory-checked host-local SDK import, pinned Zig, architecture-specific Mach-O validation | Hosted cross-build requests correctly refused absent a managed Apple SDK. Separately, GitHub native macOS ARM64 builds through `justrust --local`, native smoke, Developer ID signing, and Apple notarization passed in Jcode Actions run `40628778548`. Intel macOS and Linux-hosted Darwin cross-builds remain unverified. |
| `aarch64-unknown-linux-gnu` | Managed GNU cross-toolchain, rebuilt standard library and target-only emulated-TLS enforcement | Full-feature Jcode built and downloaded, including vendored OpenSSL. ELF/TLS structure verified, requires glibc 2.39. Subsequent hosted build to native `ubuntu-24.04-arm` smoke passed in Jcode Actions run `40434055378`. See [trial](jcode-arm64-hosted-trial-20261026.md). |

For MSVC, see [managed Windows MSVC](managed-windows-msvc.md). The license
consent is an operator decision, not an environment variable that the agent may
silently set. LLVM prerequisites have a separate, strictly boolean
`JUSTRUST_INSTALL_LLVM_TOOLS=1` opt-in, forwarded to hosted builds without
implying Microsoft SDK-license consent. The helper validates genuine Clang,
archive/import-library drivers and LLD by compiling and linking SDK-free x86_64
and ARM64 probes. These probes are not full Windows Jcode builds. Post-install
hosted probe `20261026-153028117-743725` passed for both architectures without
Microsoft SDK access or license consent. Separate forwarding regression
`20261026-152742658-724982` passed both tests.
For Darwin, see
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
build arguments. Its 17 Python safety tests pass, including inline `codesign`
requirement syntax. The embedded helper is reinstalled with fix `17c344f2e`.
Its standalone native Python signing execution separately passed protected
macOS ARM64 fixture run `40632462506`, including real native signing/verification,
unchanged input, wrong-hash refusal, and destination-collision refusal. See
[trusted release signing](trusted-release-signing.md) for its precise scope.

The helper currently needs a trusted macOS signing host for Developer ID or a
trusted Windows signing host with a certificate-store identity and SignTool.
This deliberately keeps signing keys off general-purpose compile workers.
macOS notarization, Windows reputation, native runtime testing, packaging, and
release publication are separate acceptance gates. Cross-compilation avoids
needing a separate machine for every architecture's compile stage, not every
platform-specific release operation.

## Apple CI acceptance on October 28, 2026

Jcode's validation-only workflow run `40628778548`, attempt 1, passed at source
`a143bccbc9c28d7ce6abbbd47a715dba3a9bff96`. Its native macOS ARM64 producer,
separate native consumer, and protected signing job all succeeded. The signed
binary passed Apple's Developer ID chain/identifier/Team ID verification and
native launch/help smoke. Apple's notary service returned `Accepted`, submission
`049344cd-ace4-4dbc-bf2e-0c45890c5ed4`. The downloaded signed binary matched its
signing provenance SHA256
`2a60e86c2d2a75888bcc40e3c9ede3663817778afab53dbbad2bc9338068e127`.

Dedicated encrypted signing material is held outside the source tree and in the
existing protected GitHub environment. It is not forwarded to compile workers.
Required-reviewer/master-only protections were retained and approval used the
normal configured reviewer API under user authorization. Existing Apple
credentials were not revoked. Public evidence is committed in Jcode at
`docs/evidence/native-handoff-macos-signing-40628778548.json`.

Nothing was tagged or published. This validates macOS ARM64 native build,
smoke/signing/notarization handoff, not all-platform release publication, a hosted
Darwin cross-build, Intel macOS, Windows signing, full application integration,
or offline Gatekeeper behavior.
