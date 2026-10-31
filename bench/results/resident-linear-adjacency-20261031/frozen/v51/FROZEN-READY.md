# FROZEN_READY: native-adjacency-dual-v51

Released ownership of exactly:
- toolchain/resident/crates/hir-ty-codegen/cached.rs
- toolchain/resident/crates/hir-ty-codegen/Cargo.toml
- docs/resident-linear-adjacency.md

Default-OFF compile-time feature: hir-ty-codegen/native-adjacency-diagnostic.
Diagnostic output identity: native-adjacency-dual-v51 timings_invalid=true.
Both selectors run on each identical reached visit and assert exact equality.
No runtime speedup claimed. v50 is not promoted.

v50 artifacts remain immutable and their SHA256SUMS were verified before edits.
original/ is exact pre-v51 v50 source/doc and pre-feature Cargo.toml.
candidate/ is the full frozen v51 source/doc/manifest.
change.patch is incremental from original/ to candidate/, at checkout paths.
SHA256SUMS identifies all originals, candidate files, patch and this record.

Permitted source checks only: scoped standalone rustfmt and rustfmt --check,
plus scoped git diff --check. All passed. No native builds, tests, workloads,
commits or pushes run by worker. Native tests are supplied but unexecuted here.

Root-owned gates still outstanding:
1. Exact native feature/default tests, including shadow inspection fixture.
2. Diagnostic separate artifact directory and native artifact/cache identity.
3. Record build/version/feature controls, complete snapshot/source identity and
   final whole-binary SHA256. Binary SHA unavailable because builds prohibited.
4. Existing native workflow attribution, preserving report lines by phase.
5. Treat all dual-selector diagnostic timings as INVALID for speed claims.

No additional worker edits after release without coordinator authorization.
