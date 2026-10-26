# Managed GNU/Linux ARM64 releases

The managed target is exactly `aarch64-unknown-linux-gnu`. Linux x86_64 hosts
use checksum-pinned Ubuntu Noble GCC 13.2, GNU binutils 2.42, and an ARM64
glibc 2.39 development sysroot, extracted rootlessly into the account toolchain
store. Host glibc 2.38 or newer, `ar`, and `tar` with xz/zstd support are
required. Package scripts are never executed. No system packages are installed
and no root access is used. The target sysroot is glibc 2.39, not a promise of
GLIBC_2.17 compatibility.
This is not a target substitution: GNU/Linux output remains GNU/Linux output.
The GNU linker preserves Rust's Cortex-A53 erratum mitigation, which the
previous Zig path could not accept.

## Official Jcode TLS contract

The official Jcode ARM64 release rebuilds the standard library with emulated
TLS for its Termux compatibility requirement. A successful generic ARM64
build using the precompiled standard library is **not** an equivalent release.
Keep all three parts together:

1. A nightly compiler and matching `rust-src`.
2. Cargo `-Zbuild-std=std,panic_abort`.
3. Target-only `-Ztls-model=emulated`.

The existing managed compiler `nightly-2026-10-08` includes the source component
at `~/.justrust/toolchains/nightly-2026-10-08/lib/rustlib/src/rust/library`.
Do not use `RUSTC_BOOTSTRAP` or a stable compiler as a substitute. Do not put
emulated TLS in global `RUSTFLAGS` or `CARGO_ENCODED_RUSTFLAGS`: host build
scripts and proc macros must not inherit the ARM64 TLS setting.

An invocation after selecting that managed nightly is:

```sh
CARGO_TARGET_AARCH64_UNKNOWN_LINUX_GNU_RUSTFLAGS='-Ztls-model=emulated' \
  JUSTRUST_TOOLCHAIN=nightly-2026-10-08 \
  justrust build --release --target aarch64-unknown-linux-gnu \
  -Zbuild-std=std,panic_abort
```

Preserve any other necessary target flags when setting this variable. Do not
carry a native `-fuse-ld=mold` setting into the GNU cross linker. Compiler
selection and remote routing belong to the calling command orchestration,
not to `arm64_tools`.

## Integration API

- `supported(target)` recognizes the exact managed target independently of
  the calling host. `configure` separately requires a Linux build host.
- `configure(&mut Command, &[String])` requires that explicit target and the
  full build-std list, injects target-only emulated TLS when absent, rejects
  conflicting TLS/global flags, and applies pinned GCC tools.
- `ensure()` provisions the pinned compiler and sysroot; `apply(cmd, root)`
  selects GCC, G++, ar, and the explicit sysroot for the ARM target only. Host
  `CC`, `CXX`, and `AR` remain unchanged. The pinned host helper libraries are
  added to the child library search path. `-Clinker-flavor=gcc` plus a final
  `-Clink-arg=-fuse-ld=bfd` prevents Rust's self-contained lld preference from
  bypassing the pinned GNU binutils.
- `validate_artifact(bytes)` structurally checks little-endian ELF64,
  AArch64, executable/shared-object type, segment bounds, and an executable
  load segment. It does not certify TLS provenance or runtime behavior.
- `validate(args, target_flags, global_flags)` checks already-resolved flags,
  including configuration-file flags supplied by command orchestration.

This dedicated path always requires both `std` and `panic_abort` in the
explicit build-std list, even when no TLS flags were supplied. Arguments after
`--` are not Cargo options. A later incomplete build-std list cannot bypass
validation. All nonempty global rustflags and all conflicting TLS models are
rejected. Empty global variables are removed from the child command because
even an empty encoded rustflags value overrides target flags. Existing target
flags are preserved, and `-Ztls-model=emulated` is appended if absent. There is
no generic ARM mode on this dedicated official-release path.

`configure` cannot discover arbitrary Cargo configuration by itself. Callers
must pass resolved `.cargo/config.toml`/`--config` rustflags through `validate`
or normalize them into the target-specific command environment first. This
is important because the official Jcode release workflow uses a target table
in `.cargo/config.toml`. Do not silently drop that table and claim compatibility.

## Validation status (2026-10-26)

The real x86_64-to-ARM64 TLS probe rebuilt `core`, `std`, and `panic_abort`
with managed `nightly-2026-10-08`, but linking failed: Zig 0.13.0 rejects
`--fix-cortex-a53-843419`, which the standard Rust GNU/Linux ARM64 target
passes to preserve its Cortex-A53 erratum mitigation. The run was
`20261026-150729007-588313` (21.6 seconds). The same failure occurred with
the managed jr compiler (`20261026-150653255-584130`). `-Xlinker` forwarding
also failed. Do not silently strip this mitigation or change the Rust target.
These failures motivated replacing Zig with the managed GNU cross linker,
not removing the mitigation. No executable or runtime acceptance was obtained
from those Zig runs. This machine has no `qemu-aarch64` executable.

The replacement was exercised locally on Linux x86_64. The opt-in
`managed_gcc_download_smoke` test downloaded and checksum-verified all pins,
then linked an AArch64 C executable with `--fix-cortex-a53-843419` intact
(run `20261026-151459496-641309`). A real Rust fixture using thread-local
`Cell` state and `std::thread::spawn` rebuilt `std` and `panic_abort` with
`nightly-2026-10-08` and emulated TLS. Its final explicitly BFD-linked build
passed in 23.4 seconds (run `20261026-151646341-652003`). ELF inspection found
AArch64 ELF64, interpreter `/lib/ld-linux-aarch64.so.1`, application and std
`__emutls_v` symbols plus `__emutls_get_address`, and no `PT_TLS` segment.
GNU BFD reported version 2.42. These are build/structure checks, not execution
or full Jcode/Termux acceptance. Runtime remains untested on this machine.

## Acceptance checks

Cross-compilation proves neither execution nor Termux compatibility. Check:

```sh
readelf -h target/aarch64-unknown-linux-gnu/release/PROGRAM
readelf -lW target/aarch64-unknown-linux-gnu/release/PROGRAM
readelf -sW target/aarch64-unknown-linux-gnu/release/PROGRAM
```

Require ELF64, machine AArch64, and the intended GNU/Linux interpreter. For
an emulated-TLS fixture, verify `__emutls_get_address` and emulated TLS storage
symbols, and inspect the program headers for unexpected native `PT_TLS`.
Include actual thread-local state access and thread isolation in the fixture.
Inspect the build output to confirm `core`, `std`, and `panic_abort` were built
from the selected source tree. Run the resulting program only on an ARM64
device or a configured AArch64 QEMU environment. A successful link, ELF header,
or TLS symbol inspection alone must never be reported as runtime acceptance.

The native compiler, bundled C dependencies, supported glibc baseline, dynamic
libraries, and Termux loader behavior still need checks on the real Jcode
artifact. The TLS probe is a component check, not a substitute for that release
gate. No default-performance improvement is claimed by adding this target.
