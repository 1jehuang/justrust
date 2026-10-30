# Resident diagnostic test-body skeleton (v28)

## Scope and status

This is a diagnostic-cache-only candidate. It does not change global `INERT`,
`outline::skeleton`, executable outlines, function-list reuse or codepatch policy.
`errors_of` alone uses `outline::diagnostic_skeleton(db, file, within.as_deref())`.
This worker prepared source and native unit fixtures without running builds,
tests, native workloads or commits. Root then validated the frozen Rust sources
and measured the scoped workload described below. Those observations are not a
general speedup claim or a default-promotion gate.

### Actual v27 gate and v28 repair

Root's v27 native check/build passed, but only three of eight new tests passed.
The five failures are retained in
`candidate-unit-v27/resident_diagnostic_skeleton.log`, run
`20261030-142404178-4153131`, under the scratch root. Positive builtin-body
eligibility and actual body visitation failed, so no performance candidate was
admitted. The v27 fixtures incorrectly declared `macro_rules! test` as a
rustc builtin. `collect_macro_rules` only looks up bang/eager builtins and marks
this spelling `UnimplementedBuiltIn`. Attribute builtins are registered by
`collect_macro_def` for macro2 definitions. The actual core declaration is
`#[rustc_builtin_macro] pub macro test($item:item) {}`.

The proc-shadow fixture also listed `deps:proc_macros` explicitly. The fixture
builder resolves explicit crate dependencies before adding the synthetic proc
macro crate, which caused a missing-key panic. `proc_macros: identity` already
installs that server and adds its dependency to every fixture crate. V28 removes
only the premature explicit dependency and retains the actual identity macro
imported as `test`.

The shared-file fixture explicitly imports `super::test` inside its cfg(test)
child module. Macro2 definitions have module scope rather than macro_rules'
textual scope, and this fixture has no core prelude to import it implicitly.

V28 corrects those fixture definitions and imports only, without changing
production provenance, eligibility, positive body counts or any of the eight
test assertions.
All eight tests remained mandatory and unexecuted by this worker. Root's repeated
v28 native gate passed all eight. Exact frozen v27 originals are retained under
the distinct `test-diagnostic-skeleton-v28/originals/` handoff directory.

## Why test-body edits invalidated the old key

The old diagnostic key called `outline::skeleton`, which calls `skeleton_of`,
`hash_outside_bodies` and `is_unseen_body`. The latter requires `is_inert` on the
function. `INERT` intentionally omits `test`, so `#[test]` bodies contribute their
full tokens to the old key. Changing a token changes that key and prevents reuse
of the previous clean function map. The existing
`bodies_others_see_into_are_kept` assertion for `#[test]` remains unchanged.
Root's v26 observation of all 373 bodies accepted per edit is consistent with
this source path. The actual v28 profile below confirms reduced selection on
this workload. Other syntax or resolver hazards may still prevent reuse.

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
pub macro test($item:item) {}
#[test]
fn example() { let _: u8 = 1; }
```

A plain `#[test]` fixture without core/prelude/builtin definitions is unresolved
and must not demonstrate eligibility.

## Root validation and scoped observations (2026-10-30)

The exact frozen v28 Rust sources passed native check/build, all eight new
fixtures, existing outline/prior focused gates and the full **359-test** native
suite. The new fixture run is `20261030-143016482-4189483`, the full native run
is `20261030-143103071-1087`; admission is recorded in the scratch root's
`candidate-admission-v28.json`. Root also reported all **700** benchmark regression
tests passing in **9.624 s**, retained in `bench-v28-regression.log`.
The strict five-edit runtime, error and recovery gates passed on candidate RSA24.

### End-to-end scoped medians

These are observations for the protected resident one-line test-body edit
workload, not the comprehensive session-weighted synthetic workload. Old is the
v26 engine, new is v28; the Blaze reference is a separate control.

| Run | Old engine (ms) | New engine (ms) | Blaze reference (ms) |
| --- | ---: | ---: | ---: |
| Fixed five-edit comparison | 107.675 | 86.353 | See reference-normalized comparison |
| Counterbalanced 42-edit run | 99.509 | 83.863 | 111.151 |
| Reverse-order 42-edit run | 104.264 | 82.007 | 103.373 |
| Independent fresh candidate-first five-edit run | 102.349298 | 82.189889 | Old 110.293067 / new 110.002826 |

The fixed five-edit comparison reported a raw median delta of **-19.80%**, but
the reference-normalized delta was only **-5.84%**, below the requested 10%
floor. Its comparison explicitly labels noise unknown and gives no significance
or speedup verdict. The counterbalanced and reverse runs are additional scoped
observations, not evidence of a general/default improvement. Exact evidence:

- Scratch root `suite-v28-compare.log` and its before/after result files.
- Scratch root `suite-v28-repeat-compare.log`, `suite-v28-repeat.exit` (0) and
  its independent candidate/baseline result files.
- `/home/jeremy/.jcode/scratch/resident-interleaved-v28-20261030/state.json`.
- `/home/jeremy/.jcode/scratch/resident-interleaved-v28-reverse-20261030/state.json`.

The independent fresh candidate-first replication (RSA25 new, then RSB19 old)
passed all gates. Its raw engine median delta was **-19.6967%** and its
reference-normalized delta was **-19.4848%**, exceeding the requested 10% floor
on this scoped fixture. This does not erase the first fixed five-edit run's
below-floor normalized result. The two 42-edit runs above provide additional
warm, counterbalanced/reverse-order observations. All comparisons remain
descriptive, with machine noise unknown and no statistical significance verdict.
No general/default promotion is inferred from these measurements.

### Actual native collector evidence

`/home/jeremy/.jcode/scratch/resident-test-skeleton-v28-profile-20261030/state.json`
contains **12 actual rows**, six old/new pairs, with passing runtime gates and
unchanged source/tool identities. Replicates 1 through 5 show **373 selected
bodies before and one after** for each test-body edit. Replicate 0 additionally
contains a second file's 231 selected bodies on each engine. Those inventories
must not be summed into a per-edit improvement or assigned to trace thread IDs
as an invented file/thread bijection. All observed transport drop counts were
zero.

For replicates 2 through 5, the observed `diagnostics.semantic.collect` spans
were **87.795 / 78.284 / 81.348 / 77.439 ms** before and
**17.282 / 15.482 / 14.672 / 19.239 ms** after. The diagnostic skeleton phase
was approximately **1.0 ms before versus 1.4 ms after**, so the semantic resolver
proof has a measured cost as well as enabling selective collection. These are
nested/inclusive phase observations, not additive critical-path savings or a
substitute for end-to-end timings. The profile marks machine contamination
unknown. It establishes actual selection reduction on this workload, not a
general performance conclusion.

## Handoff artifacts

Initial originals, frozen candidate copies, exact source patches, SHA-256 hashes
and readiness are under
`/home/jeremy/.jcode/scratch/blaze-native-engine-20261030/test-diagnostic-skeleton-v28/`.
The documentation path was absent before v27. V27's original main.rs was saved
after root expanded ownership following collector commit `3b13e250b`. V28's
three originals are exact v27 frozen candidates, with their original hashes.
Root ran native gates and the scoped observations above against those exact
frozen Rust sources. The frozen candidate documentation remains preserved as
part of the readiness artifacts; this later documentation-only update records
root's outcomes without changing the Rust candidates. Commits and further
measurement remain root-owned. Ownership is released in the readiness record.

## Portable retained evidence

Source commit `fe8df07f3` is accompanied by the evidence-only bundle at
`bench/results/resident-test-diagnostic-skeleton-20261030/`. It retains the v27
failure, v28 8/359 native and 700 regression gates, four strict sessions with
88 total rows, two warm runs with 282 total rows, and all 12 profile rows.
Run its `validate.py` for a manifest and semantic replay without native tools.
Its 31 offline controls passed, including isolated relocated replay and
coherently rehashed wrong-marker, failing-test, order, phase and thread mutations.
Historical scratch paths in raw receipts are labels, not replay dependencies.

The validator checks all four outer and five semantic profile labels, actual
raw trace correspondence, finite nonnegative durations, wall containment and
same-thread semantic-parent containment without inventing file/thread identity.
The two replicate-zero `module_context` spans round to zero microseconds, so
strict positivity for every individual tiny span would contradict the evidence.
Every required label has positive observations elsewhere in the twelve rows.
These offline checks establish retained evidence consistency, not a new native
execution or a general/default performance gate.
