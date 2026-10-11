# Hermetic C sysroot for -sys crates: assessment

Question (toolchain.md phase 3): what would a vendored C toolchain and
-sys library sysroot take for the Jcode Desktop dependency graph, and what
would it buy for cold builds and the prebuilt store?

## What Desktop actually builds (snapshot 7d38fb7 + jcode abff94eeb)

`cargo tree -p jcode-desktop-ui -e normal,build`, plus build-script run
times from 70 to 145 recorded runs per crate (`~/.justrust/runs/*/scripts.jsonl`,
median wall of runs that were not cache hits):

| crate | via | what its build script does | median run | build-script cache |
|---|---|---|---:|---|
| aws-lc-sys 0.44 | rustls (aws-smithy, reqwest, hyper-rustls) | compiles aws-lc (C, asm) with cc/cmake, static `aws_lc_0_44_0_crypto`, 20 MB OUT_DIR | **39 s** | 53/139 hit |
| libsqlite3-sys 0.30 (bundled) | rusqlite (jcode-base) | compiles sqlite3.c with cc, static | **8.3 s** | 50/138 |
| ring 0.17 | rustls | compiles C + perlasm with cc, static | **4.4 s** | 53/145 |
| freetype-sys 0.20 | zed-font-kit (gpui) | pkg-config probe `freetype2`, links system `-lfreetype` | ~0 s | 44/74 |
| yeslogic-fontconfig-sys 6 | zed-font-kit | pkg-config or dlopen, `-lfontconfig` | ~0 s | 44/74 |
| alsa-sys 0.3 | cpal (jcode-base) | pkg-config probe `alsa`, `-lasound` | ~0 s | 46/77 |
| wayland-sys 0.31 | wayland-backend (gpui_linux) | `dlopen` feature: no link, nothing probed | ~0 s | 0/75 (no rerun-if: never stored) |
| xkbcommon | gpui_linux | `xkbcommon-dl`: dlopen at runtime, no -sys build | none | |
| openssl | not in Desktop's graph (rustls everywhere) | | none | |

Readings:

- **Measured on the prebuilt store (FINDINGS 30).** A cold Desktop check
  from an empty local cache, with every Rust dependency the store has
  restored (808/833), still took 50.8 s against 14.6 s for a warm local
  cache. The difference is 25 Rust units that missed (all downstream of the
  aws-lc/ring C chain) plus the build scripts, which the store does not
  carry: aws-lc-sys 12.8 s, libsqlite3-sys 7.6 s, ring 4.8 s, onig_sys
  2.3 s in that run.
- **Time is in C compiles, not in probes.** Three crates (aws-lc, sqlite,
  ring) cost about 52 s of build-script time on a cold build. The
  pkg-config crates take milliseconds and wayland/xkbcommon are dlopen'd at
  run time, so they never link at build time.
- **Those three are what keeps the cross-machine cache short of the
  floor.** FINDINGS 23: laptop vs build machine keys agreed on 288/314
  crates. The 26 that did not were the aws-lc/ring/sqlite chain: their static
  archives are built by the host gcc and keyed by content, and their
  dependents' keys cascade from that.
- **The build-script cache cannot share them across machines today.** Its
  base key includes `system_stamp()` (mtime of `/var/lib/pacman/local`,
  `/etc/ld.so.cache`, ...), so any package install, and every other
  machine, misses. That is correct as long as the C compiler and headers
  come from the host.

## What a vendored sysroot means here

Concretely, three layers, cheapest first:

1. **Pinned C toolchain** (clang + lld + llvm-ar from the same LLVM the
   pinned rustc ships, or a zig-cc style bundle) installed next to the
   pinned toolchain in `~/.justrust/toolchains/<id>/c/`, plus a minimal
   glibc sysroot (headers + `libc.so` stubs for the oldest supported glibc,
   e.g. 2.28, so proc macros and bins load everywhere). justrust exports
   `CC_x86_64_unknown_linux_gnu`, `CXX_...`, `AR_...`, `CFLAGS_...=--sysroot=...`
   to build scripts. That alone makes aws-lc, ring and sqlite bit-identical
   across machines: they need nothing from the host but a compiler and libc
   headers.
2. **Build-script key without the system stamp** for packages that only
   use the vendored toolchain: when `CC`/`AR` point into the toolchain dir
   and `PKG_CONFIG` is not consulted (no `PKG_CONFIG*` in the script's
   `rerun-if-env-changed`), key by the toolchain id instead of
   `system_stamp()`. The prebuilt store can then carry build-script runs
   too (today `publish` skips them), so a cold build replays aws-lc's 20 MB
   OUT_DIR instead of running 39 s of cmake.
3. **Vendored -sys libraries** (wayland, xkbcommon, fontconfig, freetype,
   alsa, openssl): a pkg-config sysroot (`PKG_CONFIG_SYSROOT_DIR`,
   `PKG_CONFIG_LIBDIR`) with headers, `.pc` files and link stubs. For
   Desktop this buys **nothing at build time**: these scripts are already
   fast and cache well locally, and wayland/xkbcommon do not link. It
   matters only for (a) sharing their build-script runs through the store,
   and (b) machines without the dev packages installed (CI images, fresh
   build hosts), where the probe fails and the build stops. Runtime still
   needs the real system libraries, so this is headers + stubs, not a full
   distribution.

## Cost estimate

| layer | work | size | buys (Desktop cold check) |
|---|---|---|---|
| 1. clang/lld/ar + glibc 2.28 sysroot | toolchain build farm job, a `c` component in `justrust toolchain install`, env injection in the shim | ~150 MB compressed (clang+lld) + ~30 MB sysroot | cross-machine key agreement for the aws-lc/ring/sqlite chain (26 of 314 crates in FINDINGS 23), so remote and prebuilt hits reach the warm floor |
| 2. toolchain-keyed build-script entries + publish them | buildscript.rs key change, `publish` handles script entries, store layout `v1/s/` | per entry, aws-lc 20 MB | ~52 s of build-script time on the first cold build of a new machine (39 + 8.3 + 4.4) |
| 3. pkg-config sysroot for wayland, xkbcommon, fontconfig, freetype, alsa, openssl | headers + .pc + stub .so per lib, built once per toolchain cut | ~20 MB | builds on hosts without -dev packages; no measurable time on this laptop |

Recommendation: do layer 1 and 2 together (they are useless apart: 1 makes
outputs portable, 2 lets the store carry them). Defer layer 3 until a CI
image or hosted build host lacks the dev packages.

Risks: aws-lc's build script checks the compiler (`cc` crate detection,
cmake on some targets) and may pick different code paths for clang vs gcc,
so a build made with the vendored clang is not byte-identical to one made
with host gcc. That is fine as long as everyone uses the vendored one: the
key changes with the toolchain id, never silently.
