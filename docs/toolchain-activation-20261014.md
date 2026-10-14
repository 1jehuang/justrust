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

## Acceptance and activation

Compiler task `413023ndsi` is still in progress. Its stage 1 compiler finished
successfully, but stage 2, backend and matching-tool acceptance remain pending.
No candidate has been installed or activated yet. The default remains the
initial compiler. Final identities, validation results, installed/default
status and any blockers will be recorded before completion.
