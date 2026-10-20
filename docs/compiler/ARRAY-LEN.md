# Array receiver adjustment for builtin slice length

Status: implemented and validated. Native, authority and graph gates passed after repairing the discovered trait-method precedence and explicit-reference receiver cases. Integrated onto bounds parent c0c20304caa060792f9a6cb30855b38d9a00a356, preserving its original-source bounds and nonmoving borrow checks.

## Contract

A zero-argument, zero-type-argument `.len()` on a native `[T;N]`, `&[T;N]` or `&mut[T;N]` receiver uses an ordinary shared borrow/reborrow, canonical array-to-slice unsizing, then existing SliceLen. Keep source occurrence and generated CFG proofs. Never replace receiver evaluation with a length constant or synthesize raw/static storage authority.

Local and field arrays, nonCopy elements, shared/mutable array references and supported temporary array values are covered. Receivers evaluate once; a mutable reference is not consumed. Temporary values with destructors retain the existing explicit Unsupported boundary until ordinary temporary-drop lowering can preserve their exact lifetime. Local nonCopy/Drop array elements must remain in place and be dropped normally.

Canonical method lookup precedes array unsizing: receiver depth, autoref kind, inherent-before-trait priority, in-scope trait membership and generic inference remain the existing selector's responsibility. A selected or deferred array method suppresses builtin fallback, even if that method later fails mutability, argument or result checks. Imported and hidden trait methods must behave like stock Rust, including array literals and generic bounds. The source candidate must not override user-defined methods on nominal types. Arguments, generic arguments, moved arrays and unused invalid bodies retain rejection. Native slices and strings retain existing behavior. No constant generics, general method unsizing, public pointer-layout or full matrix support claim.

## IR / graph

Shared Borrow of existing place, or existing borrowed-temporary Block, followed by existing ArrayToSlice then SliceLen. References use an actual shared reborrow where needed. An explicit mutable array-reference expression is evaluated once and saved using the existing full-statement `borrow_temporaries` protocol, then shared-borrowed through Deref. Both the original mutable loan and the new shared loan are retained. This is not a block-local storage escape, generic reference-return-call autoref, or new origin authority. All nodes remain per-function; CHECK must not request codegen. Bounds/source occurrence metadata is not changed.

## Gates

Actual stock-rustc versus jr-rust CLI CHECK/BUILD/run workers1/8: local/field array, shared/mutable references, nonCopy local, array temporary with effects, receiver-call evaluation once, normal drop after len. The trait-priority fixture covers array methods returning 99, generic bounds, array literals, and hidden versus imported traits. Stock-checked negative controls cover a selected mutable method on an immutable binding, ambiguous traits, deferred generic method arguments and a selected non-usize result. Explicit unsupported temporary Drop carrier must not be accepted approximately. Negative unused invalid body, moved receiver and invalid call shape; canonical source/CFG receiver and unsize tampering rejected. Graph integrity and per-function no-codegen gate. Retain unchanged full-program06_matrix as separate boundary, not an acceptance substitute.

## Observed acceptance

- `20261019-140553071-503380`: actual stock/native CLI CHECK/BUILD/run workers1/8 matches for local/field/ref/temporary array receivers, call evaluation once, and non-Copy local elements with normal Drop. Graph test observes shared dereference loan/origin, ArrayToSlice and no CHECK codegen.
- `20261019-140715852-516088`: canonical publication rejects same-typed sibling borrow substitution, replaced length operation, missing source occurrence, and erased CFG unsize origin.
- `20261019-140824997-521220`: invalid/moved receivers and malformed unused calls rejected in CHECK/BUILD workers1/8; temporary destructor receiver stays Unsupported. Full graph-integrity14 passed without ceiling changes.
- The normal-drop fixture spells unrelated field/index dereferences explicitly. The initial shorthand form hit the existing implicit-reference-autoderef boundary before reaching this feature. This work does not claim to add general autoderef.
- `20261019-142253604-595268` isolated the new priority regression: local/literal/generic-bound/imported/hidden/mutable-selected/explicit-generic method cases passed CHECK, while explicit `(&a)` and `(&mut a)` receivers lacked hints.
- `20261019-143254246-631518`: after the typed-borrow hint and statement-temporary fix, all nine original diagnostic cases passed CHECK on hosted, including both explicit reference forms. The temporary diagnostic was removed.
- `20261019-143411911-635914`: all 21 final focused tests passed on hosted. The actual CLI matches stock CHECK/BUILD/run at workers1/8 for trait-priority99, original shared/mutable reference expressions, named constant array lengths and empty arrays. Negative controls reject selected/deferred method fallback, mutation while shared results remain live, immutable mutable borrows, and escaping local owners. Graph checks retain both original mutable and shared-Deref loan origins, exclude CHECK codegen, and pass all14 integrity gates. The previously validated receiver/nonCopy/drop matrix was not replayed.
- `20261019-144303514-667288` and `20261019-144313071-667275`: fresh hosted compile and two actual CLI integration tests pass after the exact owned three-way merge onto the bounds parent. Named constants, empty arrays, trait99 priority and explicit shared/mutable array receivers retain stock behavior at workers1/8. The prior 21-test proof remains applicable to the unchanged array implementation.

Formatting was checked with `cargo fmt --all --check`. Remaining differences are in pre-existing, unchanged files outside this feature's Rust changes; they were not reformatted into this commit.
