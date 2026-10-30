# Resident diagnostic test-body skeleton (v27)

## Scope and status

This is a diagnostic-cache-only candidate. It does not change global `INERT`,
`outline::skeleton`, executable outlines, function-list reuse or codepatch policy.
`errors_of` alone uses `outline::diagnostic_skeleton(db, file, within.as_deref())`.
Source and native unit fixtures were prepared without running builds, tests,
native workloads or commits. Root owns native validation and measurement after
freeze. No speedup or passing-test claim is made here.

## Why test-body edits invalidated the old key

The old diagnostic key called `outline::skeleton`, which calls `skeleton_of`,
`hash_outside_bodies` and `is_unseen_body`. The latter requires `is_inert` on the
function. `INERT` intentionally omits `test`, so `#[test]` bodies contribute their
full tokens to the old key. Changing a token changes that key and prevents reuse
of the previous clean function map. The existing
`bodies_others_see_into_are_kept` assertion for `#[test]` remains unchanged.
The root's v26 observation of all 373 bodies accepted per edit is consistent
with this source path, but only root's measured v27 run can establish a change
for that workload. Other syntax or resolver hazards may still prevent reuse.

## Bounded semantic proof

The diagnostic-only candidate accepts an argument-free, unqualified `#[test]`
only when its actual attribute path resolves to a macro whose **resolved
expander** is `hir_def::MacroExpander::BuiltInAttr(expander)` with
`expander.is_test()`. The proof path is:

1. `Semantics::parse` registers the source syntax with that semantic context.
2. `Semantics::resolve_path(attr.path())` uses attribute-namespace resolution
   (`source_analyzer.rs`, `resolve_hir_path_as_attr_macro`). It must return
   `PathResolution::Def(ModuleDef::Macro(mac))`, not a syntactic builtin label.
3. Public `From<hir::Macro> for hir_def::MacroId` exposes the resolved identity.
4. Lookup of `Macro2Id` or `MacroRulesId` supplies its actual expander. Proc-macro
   IDs, declarative expanders, other builtins and unresolved paths do not qualify.

`Function::is_test` is deliberately not used: it reads the syntactic IS_TEST
attribute flag, not macro provenance. `test` is not an inert builtin attribute
in the resolver's inert attribute table, so its path must resolve as an actual
macro. The exact built-in Test expander adds `cfg(test)` and preserves its input,
unlike a user-defined attribute macro that may rewrite the complete function.

The check uses every relevant crate selected by `errors_of`'s `within` set.
Each context needs one module for the file, from that exact crate, in the same
edition. `Semantics::new_within` can fall back outside its preferred set, so the
returned module's crate identity is checked explicitly. No selected crate,
multiple module contexts or edition disagreement returns `None`. Proven source
function ranges are intersected across contexts. An unresolved or shadowed
attribute in any selected context therefore prevents hiding that body. This is
intentionally bounded and may reject optimizable files rather than guess.

## Preserved skeleton contract

The original skeleton walk, item-ID restrictions and hashing are shared through
policy-parameterized private helpers. Only the function's attribute predicate may
recognize the proven builtin test. Ancestor attribute checks keep the original
policy. Const, async/gen, return-position impl Trait, inline(always), body inner
attributes and macro definitions still keep the body visible. Nested declarations
in a would-be hidden body still return `None`, including constants and statics.
The original item-position macro restrictions are unchanged. Expression-position
macro calls retain the original behavior, not an additional blanket rejection.
Signatures, attributes, outer items and visible bodies are still hashed.

Changing macro resolution, crate graphs or other files must continue to obey the
existing diagnostic-cache invalidation contract. This candidate does not create
an independent persistent resolver proof or reuse proof across database changes:
it resolves the test attributes again for every requested diagnostic key.

## Native fixtures for root's gate

Filter `resident_diagnostic_skeleton` selects **eight** new native tests:
seven in `outline::tests` and one in `main.rs`'s `tests` module.

- Builtin-resolved test bodies hash identically across unequal-length edits,
  while the global skeleton still differs.
- Signature and attribute edits change the diagnostic key.
- Nested declarations and item-position macros return `None`.
- Unresolved, declarative and proc-macro-shadowed `test` bodies remain visible.
  The proc fixture imports the predefined identity attribute as `test`.
- Existing const/async/impl Trait/body-attribute/inline(always)/ancestor-attribute
  and unknown-attribute guards remain conservative.
- A file used under multiple modules is refused.
- A shared file is resolved in its selected cfg(test) crate rather than the first
  relevant crate. An empty selected context is refused.
- The real `errors_of` oracle compares exact `(FileRange, code, message)` vectors
  after cold, unchanged, body-only, type-error/recovery, signature,
  attribute and nested-item/recovery revisions. Test-only thread-local counts
  require positive cold/full, unchanged/zero and body-only/one selection.

The oracle removes only its exact `(FileId, within fingerprint, rules)` CLEAN
key for a fresh check and uses RAII to restore the selective sequence's cache,
including on panic. Its outer guard restores any previous value for that same
key. It never clears unrelated keys. A nested-item revision has a `None`
skeleton and retains the older clean entry under existing production behavior,
so exact-source nested recovery legitimately asks zero bodies. Recovery after
an erroneous `Some` key asks all bodies because that revision consumed the old
clean entry. No production eviction behavior was changed to fit the test.

Minimal positive fixture, derived from native resolver sources:

```rust
//- /main.rs crate:main cfg:test
#[rustc_builtin_macro]
macro_rules! test { () => {} }
#[test]
fn example() { let _: u8 = 1; }
```

A plain `#[test]` fixture without core/prelude/builtin definitions is unresolved
and must not demonstrate eligibility.

## Handoff artifacts

Initial originals, frozen candidate copies, exact source patches, SHA-256 hashes
and readiness are under
`/home/jeremy/.jcode/scratch/blaze-native-engine-20261030/test-diagnostic-skeleton-v27/`.
The documentation path was absent initially. `main.rs` ownership was explicitly
expanded by root after collector commit `3b13e250b`, and its saved original is
from that handoff. Root must run native gates on these exact frozen sources
before committing or measuring. Ownership is released in the readiness record.
