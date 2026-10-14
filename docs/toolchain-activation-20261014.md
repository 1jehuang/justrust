# Vendored toolchain activation receipt, October 14, 2026

## Scope and safety

This receipt owns only the build, installation and activation workflow. No compiler,
Cargo scheduler, root implementation, or legacy checkout source was edited by this
workflow. The initial repository HEAD was
`6d898a9ea8ab2f2bfb3cac67736b33baf89dce3d`, with concurrent unrelated dirty source.
Build and validation observations below are acceptance evidence, not speed claims.
No before/after benchmark comparison was performed.

Initial default: `jr-ac20de5e7921`. Existing prefixes, official
`nightly-2026-10-08`, and legacy `~/jr-rustc` and `~/jr-cargo` must remain intact.
`JUSTRUST_TOOLCHAIN=jr-ac20de5e7921` selects the previous compiler explicitly,
`JUSTRUST_TOOLCHAIN=system` selects system fallback, and
`JUSTRUST_CARGO_FORK=0` disables the owned Cargo in favor of official Cargo.

## Source and build provenance

- Compiler command, from `toolchain/rustc`:
  `JR_STAGE=2 JR_TOOLS=1 ./build-jr.sh release`.
  This builds stage 2, matching rustdoc/clippy, std, LLVM and Cranelift backends.
  Bootstrap is a native local build, as required by the vendoring helper.
- The initial owned Cargo helper attempt included dirty scheduling and execution
  graph source. It was explicitly cancelled before installation because that
  unrelated work is not assumed ready for promotion. No success or identity is
  claimed for that cancelled attempt.
- The activation Cargo candidate instead uses a complete tracked-source
  `git archive 6d898a9ea8ab2f2bfb3cac67736b33baf89dce3d toolchain/cargo`
  extraction under scratch, preserving ignored-but-tracked fixtures. The SHA-256
  of `git ls-tree -r` for that Cargo tree is
  `d22b06ea196ec669e743e5e2cbc9c02fdbacca10c138fc8d766f3ca3ed8c3fd7`.
  Its helper is invoked with `JUSTRUST_REMOTE=off` and the existing absolute
  Cargo target directory for dependency artifact reuse. This is the documented
  local-installable-artifact exception, not an alternative hosted build claim.
  `all-static`, Cargo release `1.101.0-nightly`, and nightly channel match
  official `nightly-2026-10-08`.

## Recovery after the build server restart

The initial builds were interrupted by the server restart at 14:59:55 UTC on
October 14. At 15:01:55 UTC no live build processes remained. The resumed worker
started incremental compiler task `413023ndsi` and archived Cargo task
`420840jkx2` at 15:06:53 and 15:07:00 UTC, respectively. These are build task IDs,
not completed acceptance runs.

Before resuming, the compiler identity matched the stopped worker's receipt:
`910e39a6c3ef8b6fe54663a4f513213687b9bf4b`. The SHA-256 identity of matching
rustdoc/clippy sources and build configuration was also unchanged:
`833d47eb34a8cb45cce12fe0363d59ad44b31165baccc610899291072ad9add8`.
All 3,084 tracked files and exact symlink targets in the Cargo archive matched
Git blob identities from the selected commit. The SHA-256 of the null-delimited
`git ls-tree -r -z` manifest was
`ed2e97fec18f236b0b3a932d4b19e39929ded67397da220ca69e611b5254feaf`.
This is distinct from the newline-delimited tree digest above.

## Completed clean Cargo candidate

Task `420840jkx2` completed successfully at 15:13:41 UTC. Its recorded build
was `20261014-080700967-408953`, exit 0, 400.4 seconds. All 3,084 archived
tracked-file identities matched again after the build. The binary reports
`cargo 1.101.0-nightly (f3865b2a4 2026-09-29)` and owned identity
`97274c04c2800a7786d87b4a65287a074241b971`. A separate artifact copy was
preserved outside the shared target directory before further builds. Its SHA-256
is `6f16da371952f86c0874ed57a37a1ae4d35f5c747e4c93fceee9b5ce6bf88ee7`.
The version report confirms vendored libgit2, curl and OpenSSL. These are
provenance and acceptance observations, not a speed comparison.

## Staged compiler and correctness acceptance

Compiler task `413023ndsi` completed exit 0 at 15:37:20 UTC after 1,827.3
seconds. The full stage 2 release compiler, LLVM and Cranelift backends, std,
rustdoc and clippy were built together. Bootstrap reported successful completion
in `0:30:26` and copied the sysroot to `build/jr/release-stage2`.

Post-build compiler and matching-tool source identities exactly matched the
pre-build values above. The installed compiler reports
`rustc 1.101.0-nightly (910e39a6c 2026-10-14)`, LLVM `23.1.3`.
`justrust toolchain jr-install toolchain/rustc/build/jr/release-stage2 --no-default`
staged `jr-910e39a6c3ef`, and the preserved clean Cargo binary was installed only
into that candidate. Its independent prefix is
`~/.justrust/toolchains/staged/jr-910e39a6c3ef`.

Acceptance commands explicitly select `JUSTRUST_TOOLCHAIN=jr-910e39a6c3ef`
and `JUSTRUST_REMOTE=off`. This is the local unpublished-candidate exception:
the hosted fleet cannot execute this as-yet-unpublished compiler. These local
runs are not hosted validation or performance comparisons.

| Gate | Recorded run | Outcome |
| --- | --- | --- |
| LLVM smoke, build script, proc macro, std, thread, panic catch and doctest | `20261014-083809859-922284` | Passed, 2 tests |
| Cranelift same full smoke | `20261014-083812519-923332` | Failed at panic catching, exit 101 |
| Previous default, identical full Cranelift smoke | `20261014-083916746-942812` | Same panic-catching failure, exit 101 |
| Scoped Cranelift build script/proc macro/std/thread smoke | `20261014-084002198-956622` | Passed, 1 test, unwind test explicitly filtered out |
| LLVM separated basic and panic-catching tests plus doctest | `20261014-084002402-956807` | Passed, 3 tests |
| Smoke matching clippy, all targets, `-D warnings` | `20261014-084002883-957106` | Passed |
| Root `justrust check` | `20261014-083822449-926488` | Passed |
| Root `justrust test` | `20261014-083922439-944518` | Passed, 457 tests, 3 ignored |
| Root `justrust clippy -- -D warnings` | `20261014-084032468-970131` | Passed |
| Desktop `justrust test -p jcode-desktop-ui --lib -- fps_counter`, after reversible one-line test edit | `20261014-084033333-970370` | Passed, 3 tests, 1,351 filtered out |

The negative Cranelift result is retained, not hidden by the narrower smoke.
The unchanged backend's `compiler/rustc_codegen_cranelift/Cargo.toml` defines
`unwinding` as an opt-in feature, excluded by default. In `src/abi/mod.rs`,
`codegen_call_with_unwind_action` replaces unwind actions with `Unreachable`
when that feature is disabled. The exact same smoke fails on the preserved
previous default. Thus basic Cranelift acceptance does not establish panic
unwinding correctness. LLVM passes the full panic-catching contract. Enabling
and validating Cranelift unwinding is separate native-toolchain work, not an
incidental source change in this receipt-only workflow. The backend smoke uses
`RUSTFLAGS=-Zcodegen-backend=llvm` or `RUSTFLAGS=-Zcodegen-backend=cranelift`
with separate target directories. The distinguishing assertion is
`assert!(std::panic::catch_unwind(|| panic!("expected smoke panic")).is_err())`.

## Final provenance and preservation checks

At 15:46 UTC the native compiler identity and all 3,084 Cargo archive blob
identities still matched. Matching clippy/rustdoc sources and the release build
helper were also unchanged. The ignored generated `bootstrap.toml` had been
rewritten at 15:40:13 UTC to dev settings by a subsequent workflow, after this
release build completed and its candidate was independently copied. Its current
combined tools/config digest was therefore
`36d192f0106df87af2514479a72bb37572621dcaeb6bfed7311e206693539c24`, not the
release digest. Reconstructing the exact release config read-only from the
unchanged tracked base and helper settings restored the original combined digest
`833d47eb34a8cb45cce12fe0363d59ad44b31165baccc610899291072ad9add8`.
The concurrent generated dev config was not overwritten. Source stability and
installed artifact identity do not depend on subsequent bootstrap configuration.

The installed compiler driver and Cranelift backend byte-for-byte match the
completed release archive. Their SHA-256 values are respectively
`16617750423a7a56a1bcc335ccd9f12d6f52d82cdb78a4646b2bb59e2e0a8dd8` and
`d25ee39db84a6c941179c092aadcdc8902b8f71cded77f02a79590da8c70cea5`.
The candidate's installed clean Cargo SHA-256 also matches its preserved artifact
above. The installed candidate contains no symlinks. Its `installed_at` marker
remains zero, so it cannot become the default accidentally. Previous/default and
official fallback marker checksums still match their pre-workflow values, and
both preserved compilers execute their version commands successfully.

## Activation decision

Desktop task `43324080uz` completed exit 0 at 15:44:55 UTC. The edit changed
`[8_333_333; 3]` to `[8_333_333; 4]` in its existing test. The exit trap restored
the original source byte-for-byte from backup, and a separate `cmp` and clean
file-scoped Git diff verified restoration. The before/after restored-file SHA-256
was `24de30f7556c2e126bb52f685377f656c6352dfdf83e1bc9f9d03f037d877e8d`.
This was a real correctness workload, with a cold compiler transition, not an
incremental speed comparison. Existing Desktop warnings were displayed and not
modified by this workflow.

All build and correctness tasks owned by this receipt are terminal. The
completed result is an independently installed, explicit-pin-only candidate,
not global activation. The default remains `jr-ac20de5e7921`, with the official
`nightly-2026-10-08` fallback preserved.

Correctness acceptance alone is not a general speed claim or a default-promotion
gate. The coordinator explicitly took ownership of the matched session-weighted
40-point baseline/candidate comparison, task `706392rm53`, and the final promotion
decision. That separate gate was still running at this correctness handoff.
Until it is adequate, the candidate remains in the staged namespace. No speed
claim or default-promotion success is made by this receipt.
