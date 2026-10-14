# Opt-in Windows cross-toolchain validation on Linux

Status on October 14, 2026: **freestanding x64 and ARM64 PE cross-linking verified
on the existing hosted Linux machine, not a working hosted Windows build path**.
No EC2 provisioning, package installation, SDK download, deployment, signing,
release workflow edits, or credential forwarding was performed.

## Run explicitly

Python 3.11+, installed clang/lld-link and rustc are required. No new Python
packages are used. A probe refuses to run without `--opt-in` and refuses to reuse
an existing output directory, including a symlink. Choose a directory outside
other Cargo workspaces and outside directories containing ancestor Cargo config.

```sh
python3 -B scripts/windows-cross-probe.py --opt-in \
  --out "$JCODE_SCRATCH_DIR/windows-x64-new" \
  --target x86_64-pc-windows-msvc
python3 -B scripts/windows-cross-probe.py --opt-in \
  --out "$JCODE_SCRATCH_DIR/windows-arm64-new" \
  --target aarch64-pc-windows-msvc
python3 -B tests/windows_cross_probe_test.py -v
```

Each run compiles C to COFF, links an SDK-free `/nodefaultlib` executable, checks
the PE machine field, and queries the installed Rust target library directory.
It writes a full `report.json` with command arguments, output, exit status and
monotonic duration. Exit 2 means blocked, not successful Windows support. Exit 0
requires both SDK C linking and a minimal Rust std executable to cross-link.
Neither executable is run, so even exit 0 does not imply Windows runtime support.
The freestanding binary is an architecture/linker probe, not a runnable app.

To bootstrap a **fresh probe workspace** from a separately reviewed, licensed,
case-correct SDK/CRT tree, supply `--sdk /absolute/path`. Expected directories:

```text
crt/include
crt/lib/{x86_64,aarch64}
sdk/include/{ucrt,um,shared}
sdk/lib/{ucrt,um}/{x86_64,aarch64}
```

Only the selected architecture is required. This script does not acquire the
SDK, accept licenses, install Rust std or modify a real project. It generates a
quoted clang-cl driver wrapper, target-specific Cargo linker/libpath settings,
and a tiny standalone Rust workspace. It attempts Windows headers plus CRT/API
linking, followed by `justrust build --local --offline` only when std is present.
Local mode is intentional: the SDK paths are local to that machine and hosted
routing/configuration support is not verified. Toolchain auto-install is disabled
and the system rustc is selected to match the prerequisite query. An explicit
`--target-dir` prevents per-agent slots from changing the inspected artifact path.
Automatic CLI updates, prebuilt/dependency caches, linker overrides and the
Cargo fork are disabled for that optional Rust command. No global CLI reinstall
is needed for this standalone helper. During the announced hosted benchmark lease
(03:44 to 04:15 UTC on October 14), do not run further remote probes or builds.
The recorded tiny LLVM probes preceded receipt of that lease notice at 03:50 UTC.
This is not a complete C++/CMake/archiver toolchain configuration.

Subprocess environment is allowlisted to PATH, HOME, LANG and TMPDIR plus fixed
justrust toolchain controls for the Rust probe. Signing/cloud/SSH environment and
caller Rust flags are not propagated. HOME remains available for normal local
tool lookup/configuration: this helper is **not a sandbox** for untrusted tools,
and it does not replace hosted per-account isolation. Do not copy credentials
into its output or SDK tree. Keep signing and publication on trusted native CI.

## Observed measurements and hard limits

Evidence: `bench/results/windows-cross-probe-hosted-20261014.log`. The two JSON
objects are raw stdout from this script executed through `justrust remote ssh`
on the already-running account host. Remote temporary directories were cleaned.
The SSH wrapper exits 0 after collecting both reports, while each report's
`verdict` is **blocked**. Do not interpret SSH success as build success.

| Existing hosted Linux tooling | x64 | ARM64 |
| --- | ---: | ---: |
| clang 18.1.3, C to COFF | 13.81 ms | 13.91 ms |
| lld-link 18.1.3, COFF to PE | 9.11 ms | 7.14 ms |
| PE architecture verification | passed, 0x8664 | passed, 0xaa64 |
| Rust 1.99.0 target std | absent | absent |
| SDK/CRT | not supplied | not supplied |

These are tiny primitive probe timings, not release timings, throughput figures,
or a before/after speedup. Seven tests passed locally in 0.380s, including real
x64/ARM64 cross-linking, an ARM64 wrapper invocation with space-containing paths,
Cargo metadata parsing of the generated workspace/configuration, opt-in and
no-overwrite guards, and environment filtering. SDK fixtures test layout/config
only, not actual Microsoft headers/libraries. SDK C and Rust std linking remain
unverified. The local machine also lacks both Windows std targets and rustup.

## Release and native dependency inspection

Inspected Jcode `af0dff74f2342245df43d2aa2ba6620a75669414` and Desktop
`6f23ddef7df607f8fe7dd7f8f582cf1491ca7fd9` working trees, read-only:

* `jcode/.github/workflows/release.yml` builds both MSVC architectures natively.
  ARM64 uses `--no-default-features --features pdf`, unlike x64 defaults
  (`pdf`, `bedrock`). Launch/installer checks happen on Windows. Azure signing
  and publishing are separate jobs. Preserve that boundary and never send their
  credentials to compile hosts.
* Desktop's `cross-platform-release.yml` and `scripts/package-windows.ps1` build
  Desktop, CLI and harness API bridge, package them with PowerShell, verify the
  package and run native graphical/launch checks. Linking one executable cannot
  replace this acceptance workflow.
* Lockfiles contain aws-lc-sys, ring, libsqlite3-sys, CMake and bindgen paths.
  A lockfile inventory does not prove each is selected for a particular target.
  Validate the real feature graph and C/C++/assembly dependencies before claiming
  a release cross-build works. The helper only covers C and a minimal Rust crate.
* Desktop pins GPUI `be875c56343dcbd45127a9805781a9e128057dd7`, matching the inspected
  local GPUI checkout. `crates/gpui_windows/build.rs` guards release shader
  generation with host `cfg(target_os = "windows")`, invokes `fxc.exe`, and can
  locate it through the Windows registry. The release renderer includes generated
  `OUT_DIR/shaders_bytes.rs`. **Inference from these sources:** Linux cross-host
  release builds need a deliberate shader-generation fix or reviewed precompiled
  shader inputs, not merely clang/lld and std. This was not compiled to confirm.
* Desktop's macOS build script invokes xcrun and compiles an Objective-C AppKit
  updater. Linux Windows probes say nothing about macOS SDKs, frameworks,
  universal packaging, signing or notarization. No macOS support is claimed.

The current client environment forwarding list in `src/remote_build.rs` does not
include target-specific CC/CXX/linker or SDK path variables. Cloud bootstrap in
`justrust-cloud/src/build_hosts.js` installs Linux clang/lld and Rust but does not
establish a Windows SDK cross-toolchain. Do not widen forwarding to include
signing credentials or arbitrary machine-local SDK paths. A production solution
needs reviewed immutable host SDK/toolchain provisioning, target/toolchain/flags
cache separation, C++/CMake/archiver setup, the GPUI shader fix, and real Jcode
release builds via hosted routing followed by native Windows runtime/package
checks. This patch changes none of those shared interfaces.

Primary technical references consulted (not package selections):

```text
https://doc.rust-lang.org/stable/rustc/platform-support/windows-msvc.html
https://clang.llvm.org/docs/CrossCompilation.html
https://clang.llvm.org/docs/MSVCCompatibility.html
https://doc.rust-lang.org/rustc/platform-support/win7-windows-msvc.html
```

LLVM was recorded through integration_tools as an off-catalog selection before
using the already installed binaries. No cargo-xwin, xwin, Wine, SDK acquisition
service or new package was selected or used.
