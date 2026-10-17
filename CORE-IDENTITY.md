# Source-locked identity runtime probe

This checkpoint targets one unchanged attributed function from Rust's source,
not the entire `convert` module, a `core` crate artifact, `alloc`, `std`, or full
Rust support. `CORE-BRINGUP.md` and `IMPLEMENTATION.md` retain the larger program.

## Source provenance

`libraries/source-lock.json` selects the original documentation, attributes and
`pub const fn identity<T>(x: T) -> T { x }` bytes from a retained full source file.
The source envelope verifies the full file and selected range against SHA-256
and the reviewed probe pin. Licensing and source revision are retained alongside
the fixture. The native compiler does not invoke rustc, Cargo, LLVM or a stock
library implementation to compile or execute the provider.

The retained parent has 33,122 bytes and SHA-256
`0fd346eff31bfa6cc7c79fc4cba8367e98c579ccd7072dd141ce0bea76b602ac`.
Its selected byte range `[2258,3893)` includes original lines 48 through 107,
with SHA-256
`c4187b524514fd5daf107703898a3489da0cdf7806eea13988f4f7a365c4d6a8`.
The vendored source revision is `d907fba494adcabfdd642a18a7918b0c9847d35b`,
separate from the installed stock oracle revision. The retained MIT, Apache-2.0
and COPYRIGHT files preserve attribution to The Rust Project Contributors.

The explicit API/CLI source-probe request is separate from normal compilation:
`--source-lock PATH --source-item core::convert::identity`. Both flags are
required together. Ordinary client files keep the default attribute policy.
Only the exact verified provider envelope admits its three stability/diagnostic
metadata attributes. They are audit metadata, not source language items,
intrinsics, builtin recipes, trait facts or const-evaluation authority.
`inline(always)` retains the ordinary admitted hint policy.

Provider tokens map to the original full source file and selected byte range.
The provider is published as ordinary source and still goes through parsing,
universal body checking, move/borrow validation, instance planning and native
lowering. The function's name does not select an implementation. The current
probe concatenates an explicitly selected declaration with the authored client,
not a separately compiled library crate with cross-crate namespace or ABI facts.

## Const declaration boundary

Constness is retained in parsed and resolved declarations. The initial admitted
const body subset is structural and name-independent: one by-value identifier
parameter, matching result type, no body statements and that parameter as the
tail expression. Every declaration, including unused declarations, is validated.
Runtime calls use normal source semantics and ownership rules. Other const
bodies receive explicit unsupported diagnostics, not permissive runtime-only
checking of arbitrary const-invalid code.

This does not implement CTFE. Const items, const blocks, const generics,
const-required array expressions and general const-call legality remain open.
The presence of `rustc_const_stable` metadata does not bypass these boundaries.
A runtime forwarding probe must not be reported as successful compile-time
execution, whole-source-module acceptance, or core library compilation.

## Root-borrow inference boundary

The reference clients use a narrowly source-derived frontier for ordinary source
free functions whose original schema is exactly `(T) -> T`. It is independent
of the function's name and constness. An unannotated direct `let` may infer a
shared or mutable immediate borrow of an integer/Bool root binding, with the
root's type recovered from its original parameter, annotation or explicitly
suffixed integer/Bool initializer. Immutable replay checks lexical binding
identity, exact borrow kind, mutable capability, original initializer evidence
and the same inference transition. Callee bounds and ordinary move/loan analysis
remain mandatory. Shared and mutable traces cannot substitute for each other.

This does not remove the general inferred-reference admission fence. Projected
borrows, nested reference roots, unresolved nonscalar origins and wider inference
contexts remain explicitly unsupported. Existing reference-carrier contracts
remain separate delivered capabilities, not evidence that all reference
inference is complete.

## Stock oracle boundary

The exact attributed item is not an ordinary user-crate stock compilation.
Initial stock Rust 1.98.1 rejects its internal stability and diagnostic attributes
with E0658. An explicitly authorized compiler-internal policy experiment retaining
every original byte, with `RUSTC_BOOTSTRAP=1` and staged-api/internal-attribute
crate features, instead encounters a duplicate `convert_identity` diagnostic
item already supplied by stock core. Neither rejection is exact-source runtime
acceptance. Runtime client comparisons against stock `core::convert::identity`
are a behavioral reference, distinctly labeled from native execution of the
retained original declaration. No attributes are removed to manufacture a
successful unchanged-source stock oracle.

## Acceptance requirements

- Exact retained provider bytes, parent/range hashes, source revision and licenses.
- Independent stock classification, clearly separating default stock rejection
  of internal attributes from any explicit compiler-internal attribute policy
  experiment and from installed-core behavioral references.
- Native workers 1 and 8 CHECK/BUILD with ELF execution and exact output for
  primitives and supported non-Copy forwarding, plus ownership/unused-body
  rejection and explicit unsupported const contexts.
- Lock corruption, recomputed forged digests, range changes, attribute scope
  spoofing and provider/client provenance regressions.
- Shared SOURCE graph accounting, CHECK without native emission, deterministic
  worker output, full hosted regression, strict lint, formatting and independent
  authority review. No speed claim follows from these correctness gates.

## Observed acceptance and repaired regressions

Final hosted check `20261017-054934250-340621`, full regression
`20261017-054934732-340695` (1,758 tests in 93 binaries), strict all-target lint
`20261017-054948603-341641`, formatting and diff checks pass. Independent review
found no remaining blocker for this bounded contract, not for full Rust.

The unchanged identity corpus in `20261017-054428725-318116` passes all 72 native
worker/mode cells: ten runtime-valid clients, six ownership/unused-body invalid
clients, one stock-invalid const-body/native-unsupported boundary and one
stock-valid const-required/native-unsupported boundary. The latter two are not
claimed as successful Rust conformance. All 18 stock behavioral classifications,
two exact-source stock policy experiments and 64 separate CLI/lock rejection
gates pass. Native compilation runs without available compiler subprocess tools.
The original-source preparation/context tests pass seven cases in
`20261017-053717661-285529`.

The first native run `20261017-053354866-270222` passed 64 of 72 cells and exposed
unsupported inferred shared/mutable reference result replay. Both valid clients
remained positive requirements and now pass through the source-derived root
frontier, with exact capability/evidence mutation rejection tested in
`20261017-053828633-291153` and five canonical replay tests in
`20261017-054337142-314113`.

The first full run `20261017-054525312-322059` passed 1,755 of 1,758 tests and
exposed two existing nested-reference paths intercepted by the new frontier,
plus a const-generic diagnostic-category mismatch. The new frontier now checks
its exact original `(T) -> T` eligibility before applying scalar-root restrictions.
Other call shapes retain their previous inference path. Six unchanged nested
reference tests pass in `20261017-054852895-337604`; the final full run also
executes both new root-borrow tests and the const-context rejection tests.
No existing nested-reference fixture was demoted or rewritten.

Historical disabled const syntax passed its exact stock/native four-cell gate
in `20261017-053519727-276755` before its classification was promoted without
changing source bytes or expected output. The complete source corpus then passes
86 stock classifications and 344 native cells in `20261017-054435579-318114`:
40 supported, 33 invalid, 11 legal unsupported and two stock-warning policy
boundaries. This changes no previous checkpoint's historical counts.

All twelve cited run summaries, including both failed discovery runs, are
retained in `data/runs/xps14/2026-10-17-core-identity.jsonl`. Full const evaluation,
general reference inference, library crate artifacts, whole unchanged core,
alloc/std and full Rust remain open. No speed claim follows from this checkpoint.
