# Managed Windows MSVC builds

The helper in `src/msvc_tools.rs` supports `x86_64-pc-windows-msvc` and
`aarch64-pc-windows-msvc` from x86_64 or aarch64 Linux. It is separate from
Windows GNU and never silently substitutes that ABI.

## Explicit license consent

Before downloading or using the managed path, the caller must review the
Microsoft SDK terms at `https://go.microsoft.com/fwlink/?LinkId=2086102` and,
only when authorized, explicitly set:

```sh
export JUSTRUST_ACCEPT_MICROSOFT_SDK_LICENSE=1
```

Neither justrust nor its agents should set this on the caller's behalf.
Missing values, `true`, `yes`, and `0` do not authorize use. Both `prepare` and
`apply` enforce consent, including warm-cache use. This is necessary because
cargo-xwin itself assumes acceptance rather than prompting. `prepare` only
installs cargo-xwin. SDK/CRT download happens when the configured cargo-xwin
command runs. No consent was supplied or SDK downloaded while implementing
this helper.

## Pinned inputs and installation

| Input | Pin |
| --- | --- |
| cargo-xwin | 0.20.2 |
| Linux x86_64 archive SHA-256 | `81eb698b63fc196ad0601167dbc4bd191fdf7543b48ff1ab6cded6eb354f92bd` |
| Linux aarch64 archive SHA-256 | `71420b6949774eb75fc89704191b5e858146645083ef7ffba11cd008030a9a0f` |
| Visual Studio channel | 17.14 release |
| Windows SDK selection | 10.0.26100 |
| MSVC CRT selection | 14.44.17.14 (manifest component ID) |
| Variant | desktop, without ATL/debug libraries/debug symbols |

Archive checksums are from the official v0.20.2 GitHub release asset metadata.
SDK and CRT selections were checked against Microsoft's public VS 17 release
manifest on 2026-10-26. The upstream SDK selection uses the `10.0.26100` SDK
family identifier, not the manifest package servicing version `10.0.26100.15`.
Likewise xwin 0.6.7, embedded by cargo-xwin 0.20.2, requires the CRT component
identifier `14.44.17.14`, not payload version `14.44.35220`. The latter was the
published header/library package version observed in the reviewed manifest.
The upstream xwin downloader checks payload hashes supplied by Microsoft's
manifest. This is not an independently pinned Microsoft manifest: the channel
can change or stop serving the selected version. In that case fail rather than
switch to an unspecified/latest SDK. Fully immutable SDK provenance requires
an authorized provisioned manifest/payload mirror and is not claimed here.

Tools are installed atomically under the account's justrust toolchain root in
`msvc-tools/<revision>-<host>/bin`. Archives are verified before extraction.
An installation lock coordinates processes, partial installs fail closed,
and `JUSTRUST_TOOLCHAIN_AUTO_INSTALL=0` disables missing-tool downloads.
SDK caches live under the same revision in `sdk-cache/<target>`. Cache revision
changes accompany pin changes so cargo-xwin's architecture-only `DONE` marker
cannot reuse an SDK from another pin. Do not share this writable directory
across tenants. Never accept an arbitrary caller-provided shared cache path.

Clang and LLVM utilities must already be provisioned by the build host. The
license-independent `prepare_llvm()` helper discovers genuine `clang`,
`llvm-ar`, and `lld-link` (including version-suffixed names and distribution
`/usr/lib/llvm-N/bin` directories), then publishes
account-local aliases, using an explicitly requested package fallback if needed:

- `clang` and `clang-cl` point to the genuine Clang driver. Its executable name
  selects the supported MSVC-compatible command-line mode.
- `llvm-ar`, `llvm-lib`, and `llvm-dlltool` point to genuine LLVM archive tools.
  LLVM's executable-name dispatch selects the Windows librarian/import-library
  drivers. GNU `ar` is not a substitute.
- `lld-link` points to the genuine LLVM COFF linker.

Every new identity is functionally probed for both x86_64 and aarch64: compile
freestanding C to COFF through `clang-cl`, create/read an archive through
`llvm-lib`/`llvm-ar`, create an import library through `llvm-dlltool`, and link a
PE executable with `lld-link /nodefaultlib`. The COFF and PE machine fields are
checked. No target binary runs, and these probes use no SDK, system headers,
Microsoft libraries, license opt-in, or signing keys.
Missing tools and failing driver modes fail explicitly rather than falling
back to unrelated tools. A bare `clang-cl` alias alone is not sufficient:
LLVM's archive and COFF linker drivers must also pass the probes.

The probes themselves never access the network. If genuine `llvm-ar` is missing,
only an explicit `JUSTRUST_INSTALL_LLVM_TOOLS=1` or
`prepare_llvm_with_install(true)` authorizes a rootless package fallback. The
package is Ubuntu noble `llvm-18` version `1:18.1.3-1`, downloaded over HTTPS,
SHA-256 verified before `dpkg-deb --extract`, with no maintainer scripts run.
It provides genuine LLVM librarian/archive drivers, not replacements written
by justrust. The host must supply compatible libLLVM18, glibc >= 2.38, and other
shared-library dependencies. A loader/version probe fails before publication
if they are absent. `JUSTRUST_TOOLCHAIN_AUTO_INSTALL=0` vetoes the download even
with the LLVM opt-in. A previously installed verified package is reusable.

| Package architecture | SHA-256 |
| --- | --- |
| amd64 | `24c6a9d7b763c2b9ce3199012eca7366cada3e41978ecee022e9d44a110a9c6d` |
| arm64 | `e9dd1b2ae8187de6b138bcd8a8ab3bf04fb7639c68e90e30a1b86b266510e7e6` |

These checksums were read from Ubuntu's official noble package indexes. Package
storage is account-local under `llvm-packages/llvm-18.1.3-1-<arch>-<sha256>`.
The LLVM opt-in never grants Microsoft SDK consent and does not permit signing.

Aliases live under `llvm-tools/llvm-v1-<sha256>/bin`. The identity hashes a
manifest of canonical executable paths and SHA-256 content hashes, not merely
version output. MSVC installation/cache paths include that identity and keep a
link to the verified alias directory. A changed executable creates a new
identity. These are aliases to installed binaries, not a downloaded immutable
LLVM distribution: the host must keep them and their shared libraries stable
while builds run. Host-image pinning remains necessary for full reproducibility.
CMake/Ninja when required by dependencies and the Rust target standard library
must also be provisioned separately. Generic host `CC`, `CXX`, and `AR` are
unchanged; cargo-xwin configures target compilation.

Run the explicit no-SDK acceptance probe using the following command. This
ignored test explicitly permits downloading the pinned LLVM-only package if
needed, but does not authorize any Microsoft SDK operation:

```sh
justrust test --lib msvc_tools::tests::installed_llvm_compiles_archives_and_links_both_targets_without_sdk -- --ignored
```

The ordinary unit suite does not require LLVM to be installed. The explicit
acceptance test requires genuine Clang/LLD and a compatible host runtime for
installed or rootlessly provisioned LLVM archive tools.

## Caller integration contract

```rust
pub fn supported(target: &str) -> bool;
pub fn prepare_llvm() -> anyhow::Result<std::path::PathBuf>;
pub fn prepare_llvm_with_install(install: bool) -> anyhow::Result<std::path::PathBuf>;
pub fn prepare(target: &str) -> anyhow::Result<std::path::PathBuf>;
pub fn apply(
    cmd: &mut std::process::Command,
    root: &std::path::Path,
    target: &str,
) -> anyhow::Result<()>;
```

Call `prepare_llvm()` independently for license-free prerequisite preflight.
Call `prepare(target)` on the actual build host after caller license consent;
it prepares LLVM aliases as part of the operation. Then construct a command for
`root.join("bin/cargo-xwin")` with arguments
`xwin build --release --target <target>` (or the supported check/test operation),
and call `apply` before execution. Alternatively use `cargo xwin ...` with the
PATH configured by `apply`. The root is the value returned by `prepare`, never
an untrusted path. Explicit command-level consent overrides inherited consent,
including explicit removal. Target validation precedes side effects.

The coordinator owns routing, module registration, Rust target installation,
artifact collection, and cache identity. Include the MSVC tool revision and
SDK/CRT/LLVM inputs in any shared artifact-cache identity. Allowlist only the
exact caller consent flag for hosted execution, not arbitrary `XWIN_*` values,
SDK paths, credentials, or signing environment. `apply` overwrites the fixed
XWIN settings and disables justrust's native linker interception. It does not
change the operation or add a target, so the caller must supply the validated
target and must not add conflicting `--xwin-*` command-line overrides.

## Tests, execution and signing

Compile/check capability is not Windows test execution. Native Windows tests
need a Windows runner, and Wine is a separate explicit runner choice. Do not
report cross-compilation as passing Windows runtime tests.

The helper accepts no private key, certificate, password, keychain path,
signing command, or signing environment. Produce unsigned artifacts and sign
them in a separately authorized user-controlled workflow. Never forward keys
to the hosted builder. License opt-in is not signing authorization.

Unit tests cover strict consent, supported targets, hash shapes, target-separated
caches, pinned environment overrides, and preservation of host compiler tools.
A real MSVC build remains gated on explicit caller license acceptance and host
LLVM provisioning. No speedup claim follows from this helper.

## Reviewed official source

- `https://github.com/rust-cross/cargo-xwin/releases/tag/v0.20.2`
- `https://github.com/rust-cross/cargo-xwin/blob/v0.20.2/README.md`
- `https://github.com/rust-cross/cargo-xwin/blob/v0.20.2/src/options.rs`
- `https://github.com/rust-cross/cargo-xwin/blob/v0.20.2/src/compiler/clang_cl.rs`
- `https://github.com/llvm/llvm-project/blob/llvmorg-18.1.8/llvm/tools/llvm-ar/llvm-ar.cpp`
- `https://github.com/Jake-Shadle/xwin/blob/0.6.7/src/lib.rs`
- `https://aka.ms/vs/17/release/channel`
