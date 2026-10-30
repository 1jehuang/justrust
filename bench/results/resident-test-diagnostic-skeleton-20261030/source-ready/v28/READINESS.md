# v28 READINESS: fixture repair and explicit ownership release

Frozen 2026-10-30 21:28 UTC. All THREE paths are RELEASED back to coordinator
skunk. No further edits unless root explicitly reassigns ownership:

- toolchain/resident/crates/ra-jit/src/outline.rs
- toolchain/resident/crates/ra-jit/src/main.rs
- docs/resident-test-diagnostic-skeleton.md

## Exact candidate SHA-256

```text
f20477d485c0e7272d2986d410931439cb62fe72a105bbddbab7f8fae99e9362  toolchain/resident/crates/ra-jit/src/outline.rs
a76b2fb21419ff9f7dfdf173dbc1c4be64111566220f2b02d7dd282bf9ae510e  toolchain/resident/crates/ra-jit/src/main.rs
6d772334b097587c18cf4421109891e8f09bd6c64311b628923ab933f951089b  docs/resident-test-diagnostic-skeleton.md
787d954a943d4c1a02694398f909a8e3561e3ac2c39228de1148845cf813daf9  source.patch
5c4474829753e4537e2e9febfe735bba6a876ad43b27282563743f487d6b3f8a  v27-to-v28.patch
```

Full exact v27 originals for all three paths are in originals/, hashed by
originals.sha256. Full v28 candidate copies are in candidate/, hashed by
candidate.sha256. source.patch is the complete candidate against current HEAD
plus new docs; v27-to-v28.patch is the exact bounded repair relative to v27.
V27's own originals/candidate/readiness/patch were not edited. Its three frozen
candidate hashes were checked successfully after preparing this repair.

## Actual failure and repair

Root native v27 check/build passed, eight new tests yielded 3 pass / 5 fail,
run 20261030-142404178-4153131. The failure log remains under
../candidate-unit-v27/resident_diagnostic_skeleton.log. No performance candidate
was admitted. V28 has NOT been executed by this worker.

Real source cause: collect_macro_rules only recognizes bang/eager builtins.
The v27 macro_rules! test fixture was UnimplementedBuiltIn, so test bodies did
not qualify and the actual cache oracle did not visit two test functions.
collect_macro_def registers BuiltInAttr for macro2 declarations. V28 fixtures
now match core's actual declaration:

    #[rustc_builtin_macro]
    pub macro test($item:item) {}

The shared cfg(test) child fixture explicitly imports use super::test because
macro2 definitions are module-scoped and this fixture has no core prelude.

Proc fixture missing-key panic was an explicit deps:proc_macros header resolved
before that synthetic crate existed. V28 keeps proc_macros: identity and its
real server metadata/import identity as test, removing only the explicit deps.
The fixture builder already installs that server and adds dependencies to all
fixture crates after constructing explicit dependencies.

Production provenance/eligibility/hash/cache code is BYTE-UNCHANGED from v27.
All eight test bodies/assertions and positive selection counts are preserved.
Only fixture text, one explanation comment and documentation changed.
No syntactic fallback, relaxed builtin proof, reduced test count or weakened
positive count was introduced.

## Exact next gate

Filter resident_diagnostic_skeleton still selects EIGHT tests:
seven outline::tests plus tests::resident_diagnostic_skeleton_cache_matches_fresh_errors_after_each_edit.
All old outline tests remain unchanged. Root must run actual native gates,
including positive resolved-builtin eligibility, active crate proof, real proc
shadowing, body-one/unchanged-zero selection and exact cached-vs-fresh diagnostics.

Only rustfmt skip_children=true and git diff --check were run for v28. No builds,
tests, native workloads or commits were run by this worker. Root owns all next
gates, outcomes, promotion decisions and commits.
