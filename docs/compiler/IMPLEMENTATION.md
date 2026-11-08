# Remaining compiler implementation

Open feature work toward full Rust (edition 2024, Linux x86-64). Pick items that
unblock failing or new programs in `tests/fixtures/full_programs/` first. Check
items off in the same commit that lands them. Do not add progress narratives,
evidence notes or handoff diaries here: use tests and commit messages. History
of delivered work is in git (`git log -- docs/compiler/IMPLEMENTATION.md`).

### G1. Source organization, syntax and edition behavior

- [ ] Preserve original file/expansion spans, source maps and stable declaration
      provenance instead of relying on concatenated source positions. Original
      file syntax spans are delivered under SOURCE-PREPARATION.md; general
      semantic and macro-expansion provenance remain open.
- [ ] Complete glob imports, namespaces, block-local items/imports, cross-crate
      paths and edition-specific resolution. Bounded restricted visibility is
      delivered under VISIBILITY.md with stock/native scope acceptance. Bounded
      grouped use trees are delivered under GROUPED-IMPORTS.md. Named public
      reexports are delivered under REEXPORTS.md with stock/native privacy and
      identity acceptance.
- [ ] Cover remaining lexical/literal/item forms, attributes, cfg removal,
      recursive module discovery and useful multi-file/expanded diagnostics.
      The bounded source-envelope slice supports item/module configuration and
      explicit attribute policy, not arbitrary cfg positions or full Rust syntax
      in disabled items. The latter remains explicitly unsupported when outside
      the admitted syntax-only grammar. Crate-root `forbid(unsafe_code)` now
      has authenticated crate scope and checks original/expanded unsafe syntax;
      see `crate_forbid_unsafe_code_checks_real_syntax_and_macro_output` and
      `forbid_unsafe_code_inherits_into_files_but_not_dependency_crates`.
      General lint levels and `missing_debug_implementations` remain open.

**Next:** extend original provenance into semantic diagnostics and expansion,
then token trees/hygiene and source-owned library roots without weakening the
delivered source-map and import/privacy corpus. **Acceptance:** forward/cyclic/ambiguous lookup and unused invalid
imports have pinned outcomes, real file spans survive loading, and edits or
missing-file changes invalidate only justified consumers without stale lookup.

### G2. Typed values, patterns, inference and control flow

- [x] Indexed compound bounds-probe expansion preserves independently typed
      RHS snapshots, including heterogeneous and 128-bit shift counts, rather
      than forcing the destination element type. Existing operator/CFG
      validation remains unchanged (`COMPOUND-ASSIGNMENTS.md`).
- [x] Cast/repeat source-inventory chains traverse without recursive large
      dispatch frames, retaining original occurrence authority and unchanged
      nesting/resource checks (INTEGER-CASTS.md).
- [x] Recursive break-value parsing uses a small prefix frame, preserving the
      ordinary syntax nesting ceiling and original target authority
      (LABELED-CONTROL-FLOW.md).

- [x] Native i128/u128 scalar arithmetic, numeric casts, transport and exact
      typed constants under INTEGER128.md (not platform ABI or full Rust).
- [x] Bounded Unicode `char` literals, scalar transport, Copy/Clone/Sized and comparisons (CHARS.md). Character casts `char as int`/`u8 as char` are implemented (CHAR-CASTS.md).
- [x] Exact ASCII byte literals and byte escapes with rigid `u8` typing, pattern/range and macro forwarding support (BYTE-LITERALS.md).
- [x] Raw UTF-8 strings and normal/raw byte strings with exact `&'static [u8; N]` typing, native static data, issued literal provenance, array-to-slice coercion and worker-1/8 stock differential gates (RAW-BYTE-LITERALS.md). C strings and explicit named-lifetime local annotations remain explicitly Unsupported. This advances G1/G2/G3/G5/G6/G10 without closing their remaining full-Rust work.
- [x] Character literal patterns and integer/char range patterns with exact interval exhaustiveness (SCALAR-PATTERNS.md).
- [x] Canonical primitive and alias associated-call owners plus ordinary source `From<u8> for char`, with all 256 bytes stock-differential at workers 1/8, namespace/invalid controls and exact checked-call value authority (PRIMITIVE-CONVERSIONS.md). Implicit primitive trait-argument inference requiring coercion remains explicitly Unsupported. This advances G1/G2/G3/G5/G10, not the full conversion library.
- [x] Source-backed `str::parse` and `FromStr` for all twelve integer types,
      including full-width u128, contextual generic results, custom implementations,
      owned receiver drop order and static error-carrier rejection. Stock-differential
      CHECK/BUILD/run at workers 1/8: `justrust test --test primitive_methods`.
      Float parsing and the rest of the standard parsing surface remain open
      (PRIMITIVE-METHODS.md).
- [x] Source-backed `from_str_radix` for all twelve integer types, checked
      accumulation in bases 2 through 36, exact error kinds and invalid-radix
      abort behavior. Stock CHECK/BUILD/run at workers 1/8 and panic comparison:
      `justrust test --test primitive_methods integer_radix`.
- [x] Source-backed boolean and Unicode-scalar `FromStr`, public bool/char error
      types and exact error formatting. Stock CHECK/BUILD/run at workers 1/8:
      `justrust test --test primitive_methods bool_and_char_from_str_match_stock`.
      Floating parsing remains open. Contextual generic-free-call nominal
      receivers are covered by `generic_inference_graph::contextual_generic_call_receivers_match_stock`,
      including conflicting and unconstrained input rejection.
- [x] Width-correct primitive integer `.leading_zeros()` and unchanged original
      core exponent-estimator module acceptance under LEADING-ZEROS.md. This is
      not the original numeric method provider, a core crate artifact or CTFE.
- [x] `mut` by-value function and method parameters bound as mutable locals,
      with immutable-parameter reassignment rejected (E0384) even in unused
      bodies. 9 stock-frozen sources: 5 native positives byte-identical to stock
      in debug and release at workers 1/8 (including a Drop owner replaced
      through a `mut` parameter), 3 rejections, 1 explicit boundary (`mut`
      inside a tuple parameter pattern). Promotes the frozen Drop
      `parameter_paths.rs` to native positive.
- [ ] Complete char, str/string literals, raw pointers, function items/pointers,
      remaining type-alias inference/generic forms, never coercions and literals.
      Bounded canonical source aliases are delivered under TYPE-ALIASES.md.
      Extend float/library behavior without confusing private scalar transport
      with platform layout or profile-correct arithmetic behavior.
- [ ] Complete empty/unit/tuple structs, construction/update forms, unions,
      discriminants, recursive types, alignment/layout and repr attributes.
      Bounded nongeneric unit and nonempty tuple forms are delivered under
      UNIT-STRUCTS.md and TUPLE-STRUCTS.md, not full aggregate compatibility.
      Positional-pattern inference now distinguishes canonical tuple-struct
      declarations from enum variant owners before propagating field types.
      Canonical source `mem::align_of<T>` now retains sealed symbolic queries
      until substitution and observes target scalar/array/tuple/struct alignment,
      independently of private storage stride (TARGET-ALIGNMENT.md). Enum,
      closure and unsized layouts fail explicitly; public size/niches and
      over-alignment attributes remain open.
- [x] General typed shared `&[T]` parameters/locals, exact array unsizing,
      element-count len and authoritative Copy reads, including rigid generic,
      wide and Copy aggregate elements, under TYPED-SLICES.md. Checked native
      stride, bounds/overflow, independent values and original loan origins are
      verified; this is not general reference/DST or platform ABI support.
- [ ] Complete remaining slice/DST metadata, mutable/range indexing, slice returns,
      nested reference-bearing elements and borrowed indexed-place capabilities,
      retaining bounds/evaluation order, concrete admission and size limits.
      Delivered: nested runtime array reads and writes (`m[i][j]`, `= / OP=`,
      through fields) with per-level stock order and bounds checks
      (NESTED-INDEX.md). Reference-rooted and non-scalar-leaf forms remain
      Unsupported.
      Delivered: shared subslices `&s[a..b]` of all range kinds with stock
      order and checks (SUBSLICE.md). Local-rooted mutable range reborrows
      and shared views retain conservative whole-source loans
      (MUTABLE-RANGES.md); str slicing remains Unsupported.
- [ ] Complete nested/or/range/ref/@/slice patterns, guards, match ergonomics,
      exhaustiveness/usefulness, conditional pattern forms beyond delivered
      IF-LET.md/WHILE-LET.md/LET-ELSE.md slices and destructuring assignment.
      Delivered: match ergonomics (default binding modes, `ref`/`ref mut`,
      `&pat`, enum matches through dereferences), see MATCH-ERGONOMICS.md.
      Delivered: Copy/shared `name @ pattern` bindings (including nested/or
      subpatterns) and edition-2024 if-let/while-let chains with short-circuit
      evaluation and then-only scopes. Non-Copy by-value @ and runtime slice
      patterns remain explicit Unsupported boundaries.
      Assignment/compound-assignment expressions now produce unit in match arms,
      parentheses and other expression positions, reusing ordinary checked place
      operations for fields and indexed targets (`tests/assignment_expressions.rs`).
      Unparenthesized top-level or-patterns in let/let-else bindings are syntax
      errors; parenthesized patterns retain ordinary checking.
      Guarded ordinary/macro or-patterns retry overlapping alternatives with
      Copy bindings and binding-sensitive side effects. Explicit checked groups
      share one source guard/body; canonical call authority and cross-arm fresh
      bindings are retained. Borrowed payload retries now use an all-path
      kill-before-rebirth proof over solved live origins, including transitive
      reborrow dependencies. Unknown returned-carrier origins retain conservative
      boundaries. The retained borrowed fixture runs natively at workers 1 and 8.
- [ ] Extend bidirectional/local inference, coercion sites, partial type holes,
      omitted method arguments and control-flow joins, preserving rigid universal
      parameters and diagnostics for unresolved constraints.
      Static associated calls now constrain their canonical written owner before
      nested return inference (`STATIC-OWNER-INFERENCE.md`): focused stock/native
      acceptance and invalid controls at workers 1/8 plus CHECK frontier and graph
      gates pass. The disjoint-inherent-candidate regression is now repaired:
      known ordered receiver arguments filter candidate impl patterns without
      guessing unresolved arguments. The unchanged receiver-order/symbol test,
      all 109 backend tests, focused stock differential workers1/8 controls and
      all 14 graph-integrity tests pass (ASSOCIATED-ITEMS.md, 2026-10-20).
      Delivered source `_` holes in nested local annotations and explicit
      free/inherent method type arguments retain their concrete written constraints
      and canonical nominal identities (`tests/partial_annotations.rs`), while
      item-signature holes get E0121. Named generic arguments coexist with inferred
      anonymous argument-position `impl Trait` parameters. Source never types
      retain their distinct semantic representation under the pinned stock oracle.
      Direct `-> !` function/closure returns remain admitted, while unstable never
      types in parameters, fields, locals, generic arguments and nested or
      parenthesized returns are rejected with E0658, including unused items.
      Eight stock-pinned negative sources pass CHECK/BUILD at workers 1/8;
      direct-return execution controls, 78 focused tests and graph14 pass
      (`data/runs/rust-support-followup-20261020/never-source-boundary.jsonl`).
      Deferred written annotation holes reuse solved declaration facts; anonymous
      `impl Trait` parameter identities cannot collide with source-spellable names.
      Return-position opaque identities, declared-bound source calls and defining
      hidden-type agreement have an isolated source-CHECK implementation
      (OPAQUE-RETURNS.md). Owner-local admission checks unused declarations without
      demanding MONO_PLAN, hidden representation or code generation. Sealed CFG
      packs reject unproved reference-carrier transport. A first nongeneric,
      carrier-empty nominal `impl Read` return and bound call now passes actual
      hosted stock-differential CHECK/BUILD/link/run at workers 1 and 8. Exact
      per-call graph authorization gates physical CFG replay, ownership and the
      existing emitter, without a hidden MONO map or closed-facts relabel.
      Current-parent all-test compilation, representation/physical authority,
      unused associated admission and unchanged graph gates pass. A bounded
      call-free generic defining body additionally passes stock-differential
      CHECK/BUILD/link/run at workers 1 and 8 with distinct `u8`/`u16` actuals:
      exact declaration-owned substitution precedes hidden representation and
      physical ABI mapping while CHECK remains representation/codegen-free.
      Public associated assignments and `impl Fn(...) -> ...` bounds now support
      source CHECK through authenticated candidate discovery and a dedicated
      declaration-bound normalization origin. Worker-1/8 provider tests retain
      zero representation/dispatch/MONO/codegen demands; foreign owner/site/capture,
      Output and caller world/env/phase replay controls reject. The later owned
      generic callable increment listed above adds executable scalar-capturing
      closure-backed opaque values passed by value to generic `Fn` consumers.
      Later increments above complete nested opaque closure composition and
      the covered borrowed callable forwarding paths. Reference-bearing hidden
      representations and realized destructor dependencies remain incomplete,
      not approximated.
      Partial method inference substitutes canonical enclosing impl bindings.
      Delivered body-local structural union-find constraints: later inherent
      receiver uses, assignments and call parameters solve nested generic local
      bindings before lowering, with i32/f64 defaults and E0282 for unresolved
      nominal bindings (`tests/body_inference.rs`). Nominal payload patterns,
      Option assignment and nested returns propagate constraints, with delayed
      method results preserving reference-pattern binding modes and authoritative
      Deref coercions. Solved hints do not create source reborrow contexts:
      unique-reference moves through locals, aggregates and generic constructors
      remain moves, while declaration-written reference fields and written actuals
      retain reborrows (`tests/inference_move_authority.rs`). Partial nominal
      applications and newly inferred generic reference consumers retain explicit
      unsupported boundaries. Canonical generic-call inference represents slice
      elements structurally, preserving callee/caller binder substitution through
      shared and mutable slice signatures and raw-pointer forwarding. Array-loop
      constraints preserve owned, shared and
      mutable binding modes, and resolved non-array receivers request the canonical
      IntoIterator::Item projection. Immediate-for fixed-array `.iter()` now
      defers item inference until the exact receiver can be selected, preserves
      real method priority, and lowers through existing shared element loans with
      replayed original method/loop/borrow correspondence. Unchanged full-program
      08 passes stock-differential CHECK/BUILD/run at workers 1/8, alongside all
      14 graph-integrity gates (`tests/iterable_literals.rs`, `FOR-LOANS.md`).
      First-class array iterator values, `.iter_mut()` and general adapter
      activation remain separate work. Scalar/unit
      stored callable parameters and results now share body-local constraints from
      later calls, expected results and canonical callable bounds, with lexical
      binding identities preserved through nested blocks and shadowing. Solved
      signatures remain hints, not source-written coercion or receiver authority
      (`tests/callable_inference.rs`, `LOCAL-INFERENCE.md`). Native scalar
      constraint correction mirrors syscall seven-`usize` operands/result and
      raw-pointer place pointees, with parser/solver and unsafe/mutability
      rejection controls; it does not activate nominal realization or relax
      graph gates. Reference-bearing
      result carriers have explicit early Unsupported admission with no image;
      reference-parameter invocation, result transport, direct generic closure
      arguments and general iterator-item projections remain open.
      Loop and labeled-block result terms now preserve later numeric evidence,
      lexical break targets and diverging operands. Fourteen valid and eight
      invalid stock-differential cases pass CHECK/BUILD at workers 1/8, with
      native execution for finite cases and complete failure graph capture
      (`LOCAL-LITERALS.md`, run `20261020-044246688-3961792`).

**Next:** enumerate remaining type/pattern forms and freeze one end-to-end slice,
such as general slice metadata or nested patterns, with resource accounting.
**Acceptance:** source execution and invalid typing agree with the pinned oracle,
including effect order, overflow profile, empty values, unreachable bodies and
malformed typed IR. Large scalar arrays need a measured analysis representation
that removes the recorded 44,417-byte Adler2 resource blocker without weakening
move/initialization checks. Adapted Adler2 remains distinct from unchanged-crate
acceptance.

### G3. Ownership, regions, destruction and memory validity

- [ ] Complete reference-containing nominal fields/aggregates and lifetime type
      parameters, variance, subtyping, implied/outlives bounds, higher-ranked
      regions, `'static`, placeholder lifetimes and general region inference.
      Delivered: ordered nominal lifetime syntax and declaration-owned per-carrier
      region rows preserve references returned through temporary receiver borrows,
      including iterator-style `Option<&'a T>` results. Typed shared reborrow
      boundaries retain referent provenance without borrowing the containing
      handle; mutable chains retain every unique-access dependency. Exact named
      carrier rows govern field replacement and reject unrelated-lifetime stores.
      Stock differential and invalid escape controls: `tests/nominal_regions.rs`.
- [ ] Extend dynamic borrowed places, temporary scopes/lifetime extension,
      aggregate carrier flows, general reborrows and loop/SCC-carried loans.
      Preserve the delivered current-cell and per-occurrence provenance rules.
      Conditional reference replacement now preserves alternative origin worlds
      instead of conflating them with simultaneous aliases. Explicit-loan pair
      proofs fail closed for unknown origins, incomplete evidence and exhaustion.
      Delivered: assignment/compound assignment through call-returned `&mut`
      places with stock value->carrier->index order (TEMP-PLACES.md); by-value
      temporary fields and plain runtime index through a call stay Unsupported.
      Explicit `&*call()` / `&mut *call()` now retain the original returned loan
      rather than copying its referent, with single evaluation and invalid
      alias/escape rejection (`indexed_borrows::returned_reference_reborrows_match_stock`).
- [x] Normal-path canonical `Drop` for nongeneric scalar-field named structs
      under DROP-NORMAL.md: runtime drop flags, reverse lexical/parameter order,
      moves, conditional init/move, RHS-first replacement, loops, labeled exits,
      statement/`if`/lazy-boolean/`while` condition temporaries, protected
      return/break values, implicit destructor instance demand and sealed CFG
      proof. 56 stock-frozen sources: 25 native-positive with exact stock output
      in debug and release, 4 semantic rejections, 27 explicit boundaries.
      Aborting panic only; no unwind cleanup, aggregate/generic glue or dropck.
- [x] Structural enum payload cleanup on normal paths: enums without their own
      `Drop` target follow the active variant and canonical per-payload move flags,
      including nested payload moves and replacement after a partial move.
      `tests/result_map.rs` compares CHECK/BUILD/run with stock at workers 1/8;
      `tests/drop_normal.rs` now admits the unchanged owned-match and owned-if-let
      fixtures (40 native-positive cases). Forged cleanup and moved-field evidence
      reject. Moving a non-Copy payload out of an enum with its own `Drop` rejects
      with E0509; borrowed, Copy-only and wildcard patterns are not misclassified.
      This does not add unwind cleanup or general dropck. The isolated owned
      patch passes 1,809 library tests, 17 graph-integrity tests (one existing
      ignore in each), and six stock/drop integration tests. Check and clippy
      succeed with baseline warnings; scoped formatting passes. Receipts:
      `data/runs/xps14/structural-enum-drop-2026-10-23.jsonl`.
- [ ] Implement real Drop checking/elaboration, conditional drop flags, partial
      initialization, assignment replacement and exact destruction order across
      returns, branches, loops, breaks, panic/unwind and closure/async captures.
      Delivered slices include the scalar-field owners and structural enum moves
      above. General aggregate/generic/reference-bearing ownership, field moves
      out of Drop owners, broader owned patterns and unwinding remain open.
- [ ] Specify unsafe validity, raw-pointer operations, alias/provenance rules,
      interior mutability and UnsafeCell interactions. Track genuinely unresolved
      language-model questions rather than inventing permissive defaults.
      First slice landed (RAW-POINTERS.md): `*const/*mut T`, `unsafe {}`,
      E0133-checked deref read/write, pointer casts and equality; pointee
      storage is untracked by ownership/loan analysis.
      Added ordinary-source mutable slice `as_mut_ptr`, including generic callers:
      typed descriptor data extraction preserves element identity and never reads
      an element. Exact implementation-selected associated results carry immutable
      proof dependencies replayed against the caller's world and environment;
      bound-only projections remain symbolic. Empty/ZST/non-Copy cases and forged
      source/type/evidence controls are covered in `tests/slice_methods.rs` and
      existing IR/trait tests. `split_at_mut` and general tuple-reference origin
      transport remain open (RAW-POINTERS.md). Nominal pointer-cast targets now
      use the authenticated source realization rather than a detached source-slot
      identity; unchanged raw-slice and heap transport fixtures pass, with wrong
      owner/length/mutability casts still rejected. The exact correction passes
      1,812 library, 17 graph and 65 related integration tests.

**Next:** agree a shared drop-obligation/cleanup IR contract and freeze observable
Drop-order plus rejected-escape fixtures. In parallel classify aggregate-region
and loop-loan boundaries. **Acceptance:** required drops occur exactly once on
supported normal or exceptional paths, invalid escapes and live-alias conflicts
reject even in unused bodies, and CHECK/emission consume the same verified
ownership facts. Explicitly permitted leaks/forgotten values are not compiler
cleanup failures.

### G4. Traits, generics, methods and suspended computations

- [ ] Trait-based autoderef (TRAIT-OBJECTS.md): explicit `*`/`&*`, one-step
      method/field autoderef through Deref/DerefMut and the `&dyn` recipe
      regression pass stock/native at workers 1/8. Open: Box<dyn Trait>
      unsizing/structural drop, multi-step chains, format-arg autoderef.
- [x] Closure bodies inside graph-realized nominal callables share the parent's
      nominal inputs and propagate suspension; nominal operator/trait calls in a
      closure body (`fold(V{..}, |acc, &p| acc + p)`) use the validated mapped
      signature without a parent-arena receipt. Stock CHECK/BUILD/run at workers
      1/8 and missing-impl rejection in `tests/iterator_fold.rs`. Unchanged
      `19_operator_overloading` now passes: full-program corpus 8/20.
- [x] Stock index autoderef through references to nominal containers:
      `r[i]` / `r[i] = x` / `r[i] += x` with `r: &Vec<T>` or `&mut Vec<T>` use
      `Index`/`IndexMut` on `*r`; constant-index operands through `&[T]` in
      generic trait comparisons (`v[0] < p`, `T: PartialOrd`) take the runtime
      bounds-checked place; `Vec<String>`/`Vec<&str>` `join` (prelude
      `SliceJoin`, pending `Vec: Deref<Target=[T]>`). Nominal-declaration schema
      field walks charge work, not retained clone bytes. Stock-differential at
      workers 1/8 plus shared-reference assignment rejection in
      `tests/vec_index.rs`. Corpus stays 8/20: 16 now stops at `split_at_mut`
      (returning two `&mut [T]` in a tuple is unsupported), 06 at
      `row.iter().map(..).collect()` over arrays, 04 at a per-task proof-node
      limit from repeated trait proofs in `iter().enumerate()` over `Vec`.
- [x] Bounded source trait type parameters and complete checked applications in
      impls, bounds, supertraits, UFCS, unique dot dispatch and defaults under
      GENERIC-TRAITS.md. Same Self with distinct trait actuals retains separate
      coherence, proof and native instance identity. General traits remain open.
- [x] Bounded closed associated-type declarations, assignment pairs and exact
      normalization for ordinary signatures, aliases and locals under
      ASSOCIATED-TYPES.md, with general Sized reference-free normal forms,
      immutable raw schemas, scoped source authority and debug/release native
      evidence. This first vertical does not by itself supply rigid caller projections.
- [x] Bounded projected required signatures, exact owning-impl normalization and
      universally checked implementations plus closed calls under PROJECTED-METHODS.md.
      Per-application required carrier evidence preserves source regions; direct calls
      recheck all associated assignments after substitution. Arbitrary rigid caller
      projections and projected defaults remain open beyond the equality slice below.
- [x] Bounded closed-RHS associated equality in function/impl bounds under
      ASSOCIATED-EQUALITY.md, with genuine rigid caller parameter carriers, immutable
      raw obligations and exact post-substitution discharge. Debug/release native
      and authority gates pass; general equality remains open.
- [x] Bare caller-parameter equality RHSs under PARAMETER-EQUALITY.md, with exact
      ParamId preservation, contextual Sized, mandatory body/direct/required result
      certificates and post-substitution reference-free checks. Generic T-to-U
      results, moves and genuine Copy facts pass debug/release native gates.
      Composite symbolic RHSs and reference-bearing results remain outside this
      bounded slice.
- [x] Canonical rigid associated projection families in universal callables and
      provided methods under UNIVERSAL-PROJECTIONS.md: exact declared membership,
      opaque ownership/carrier identity, sealed pattern comparison and closed
      normalization replay. Identity and Iterator `last` match stock CHECK/BUILD/run
      at workers 1/8. The canonical blanket `IntoIterator for I: Iterator`
      assignment and concrete `for` consumer also pass stock/native 1/8;
      generic consumers retain associated declaration obligations as described below.
      Full Iterator adapters remain open.
- [x] Impl parameters determined by associated output equalities use a
      dependency-ordered fixed point, then canonical closed normalization to
      infer missing output actuals. Unused and cyclic unconstrained impls are
      rejected even when never called (`iterator_associated`).
- [x] Generic trait method calls retain separate owner and method binders,
      alpha-rename provider signatures and predicates exactly, and select
      overrides or provided defaults with ordered method actuals
      (`generic_trait_methods`). Universal projected signatures remain open.
- [x] Stored closure bounds with projected argument tuples retain exact canonical
      membership through inference and closed carrier equality replay
      (`PROJECTED-CALLABLE-BOUNDS.md`). The untyped stored closure consumer and
      independent Fn Output equality noninterference pass stock/native CHECK,
      BUILD and run at workers 1/8. Numeric projection receivers default to
      i32/f64 only after constraint quiescence, preserving later explicit widths
      (`tests/projection_defaults.rs`). General reference-bearing callable support,
      named function-item values and full Iterator adapter execution remain open.
- [x] Closed checked stored closures accept shared/mutable reference argument tuples
      with carrier-free results (`CALLABLE-REFERENCE-INPUTS.md`). Exact closed proof,
      canonical Fn trait/mode and checked adapter ABI replay retain structural input
      carriers. Shared Fn and mutable FnMut CHECK/BUILD/run match stock at workers
      1/8, with forged proof/site/world/tuple, strengthening, alias and escape
      rejection controls. Reference results and generic method discovery are separate.
- [x] Source UFCS calls retain explicit method actuals separately from ordered
      trait-owner actuals. Omitted method arguments use canonical schema-based
      inference, including zero-argument expected-result inference; final trait
      selection still checks arity and predicates. Generated generic/associated
      callable products retain exact immutable owner inventories, and genuine
      postfix invocation retains the actual callee expression and evaluation
      order (`GENERATED-CALLABLES.md`). Full Iterator source activation remains
      separate and is not established by these helper gates.
- [x] Generic dot method arguments are inferred at the canonical receiver
      candidate, preserving bounded receiver depth/mode and inherent-before-trait
      precedence. Final selection still discharges exact owner/method actuals and
      predicates (`generic_dot_methods`). Shorthand associated qualifiers retain
      the original complete bound separately from its constraint-free qualifier
      (`shorthand_bound_equalities`). These compiler helpers do not establish full
      native Iterator library support.
- [x] General associated declaration trait applications and equality bounds under
      ASSOCIATED-BOUNDS.md retain exact source/owner/ordered argument identity and
      prove every assignment, including unused generic impls. Transitive symbolic
      RHS reduction and closed predicate normalization replay support the standard
      `IntoIter: Iterator<Item = Self::Item>` generic loop, matching stock native
      CHECK/BUILD/run at workers 1/8. This does not invent Copy for equality-bound U;
      `?Sized`, GATs and associated defaults remain separate work. Closed call
      selection now separately replays raw substitution and exact closed projection
      normalization, fixing applied/inherited associated-bound BUILD failures.
      Four focused stock-differential tests (workers 1/8), forged replay controls,
      and all 14 graph-integrity checks pass on hosted (2026-10-20,
      `20261020-035437685-3791131`), without relaxing structural validation.
- [x] Inherited associated type lookup through supertraits canonicalizes the owning
      declaration and ordered actuals. Generic `IndexMut<I>: Index<I>` implementations
      normalize inherited `Output` using an independently proved superclass impl,
      with CHECK/BUILD/native-run tests at workers 1 and 8 (`inherited_associated`).
- [x] std-shaped `Hash::hash<H: Hasher>` and `Hasher` (`write` plus provided
      `write_*` matching core dispatch: one native-width `write` per unsigned
      width, signed delegate to unsigned, isize to usize) in the prelude, with
      `#[derive(Hash)]` generic over a fresh hasher binder (HASH-PROTOCOL.md).
      `resolve_trait_call` now treats a concrete-Self trait call as closed only
      when its ordered method actuals are closed too; forwarded caller type
      parameters keep the caller environment and phase. `tests/hash_protocol.rs`
      compares recorded byte streams and write-call dispatch with stock at
      workers 1/8, plus missing-bound and non-std-form rejections; check
      --tests and graph_integrity 14/14 on the committed tree. Runs:
      `data/runs/xps14/2026-10-19-hash-protocol.jsonl`. No HashMap,
      BuildHasher/RandomState, str/String/slice Hash or general std claim.
- [x] Generic trait methods (own method binder) implemented for a
      reference-typed Self (`impl<T: Tr> Tr for &T`) with SIZED ordinary
      actuals: the impl's expected signature from `alpha_rename_method_schema`
      now realizes per-layer reference topology with the existing
      `realize_ref_contract` kernel after all owner/arity/raw-contract/RHS/
      predicate checks, as ordinary call instantiation already did.
      `tests/reference_self_generic_methods.rs` (contract: REFERENCE-SELF-GENERIC-METHODS.md)
      covers `&T`/`&&T` forwarding for a sized T, unchanged zero-layer Self and
      a reference method actual through a typed-local receiver;
      `tests/reference_self_generic_method_authority.rs` rejects forged
      contract rows on the real validated-facts path. Observed rejections kept
      as explicit tests, not support: unsized `T := str` ("slice generic
      arguments are unsupported"), methods returning `&Self` with Self = `&T`
      (source-contract result-elision debt) and borrow-expression receivers
      `(&r).m()` ("unsupported unresolved receiver type"). No str/String Hash
      or unsized support. Validated on nightly-2026-10-08; the
      `trait_ref_self` dyn-array case fails on the unpatched parent too
      (baseline, see the feature doc).
- [x] Struct and enum type-parameter bounds and `where` clauses are retained and
      resolved in their declaration scope. Constructors and function signature
      applications prove user and builtin obligations, including nested nominal
      actuals and unused functions (`deref_index_traits`).
- [x] General callable where-predicate subjects retain source types and authoritative
      obligations, including `Self` and qualified associated projections (`where_subjects`).
      Higher-ranked predicates, type-outlives bounds and projected subjects in nominal
      declaration bounds remain unsupported.
- [ ] Add trait type/lifetime/const parameters, generic methods, where clauses,
      associated types/constants, projection normalization, GATs, defaults and
      supertrait obligations beyond the bounded delivered source-default slice.
- [ ] Complete coherence/orphan rules across crates, auto traits/negative impls
      where stable, recursive obligation convergence and precise ambiguity.
      Resource exhaustion must not become NotProven or successful dispatch.
- [ ] Implement trait-based autoderef, overloaded operators/indexing, coercions,
      dyn compatibility/vtables, unsizing and `?Sized` with authoritative layout
      and receiver adjustments rather than name- or pointer-based permissions.
      Delivered: `&dyn`/`&mut dyn` vtables, unsizing, E0038 (TRAIT-OBJECTS.md).
      Dynamic principals retain ordered generic actuals and canonical associated
      equalities, including inherited bindings, through native vtable dispatch.
      Missing/wrong equalities reject. `Box<dyn>` structural drop remains open.
      Comparison autoref preserves original nested operand calls instead of
      cloning their AST, with all six comparison operators checked/built/run
      against stock at workers 1 and 8 (`tests/comparison_source.rs`).
      Body-local operator terms retain independent receiver/RHS/Output types and
      select canonical trait signatures, including nested overloaded results and
      compound RHS widths (`OPERATOR-INFERENCE.md`). No complete Rust operator
      or reference-capability support is implied.
- [ ] Complete trait-based for desugaring. Concrete canonical IntoIterator/Iterator
      loops have stock-matching CHECK/BUILD/run coverage at workers 1 and 8,
      including char/tuple items, original method expressions and labeled control.
      Blanket/universal projected iteration and source-library adapter activation
      remain blocked; this is not complete Iterator support (`iterator_loop`).
- [ ] Complete closures and capture modes, callable traits, function values,
      iterator/for desugaring, async/await/Future state machines and pinning.
      Include capture lifetimes, suspension-safe moves, cancellation and Drop.
      Immediate explicitly typed noncapturing callables and shared/mutable scalar
      parent-parameter captures are foundations only (CLOSURES.md), not
      completion of this item.
      Stored noncapturing closures are checked in their parent body and published
      as immutable per-parent executable facts, with direct and generic Fn calls
      (`tests/stored_closures.rs`). Stored value/move capture environments and
      FnOnce consumption use canonical receiver-mode-specific executable facts
      (`tests/closure_captures.rs`), including stock-matching move rejection.
      No-call closure CFGs also retain exact parent-body/mode lowering proof for
      owning Drop scratch locals (`tests/closure_drop_proof.rs`), without
      weakening destructor or source-authorization gates.
      Stored shared/mutable scalar and owned aggregate-field captures retain
      disjoint field identity, ordinary loan/move validation, unused-body rejection
      and owning Drop order
      (`tests/closure_borrows.rs`, `tests/closure_capture_places.rs`). Inferred
      signature hints do not grant written coercion authority. Reference-bearing
      closure results explicitly reject pending exact carrier-origin contracts.
      A bounded symbolic root result now retains an immutable source witness for
      its exact required-call equality origin (`CLOSURE-RESULTS.md`). Authentic
      compose-shaped source checks, lowers and ownership-validates in Fn/FnMut/
      FnOnce modes, with foreign source/context and reference-substitution controls.
      Witnesses retain only canonical IDs and finite parameter/tuple recipes;
      each actual owner substitution and equality-result certificate is replayed.
      This is a source-validation prerequisite, not engine worker-1/8, native
      opaque composition, reference-result support or successful corpus 11.
      Genuine prelude compose returning `impl Fn(A)->C` now also discharges
      its hidden closure's symbolic Output equality through existing checked-
      closure normalization, preserving actual caller world/environment/phase.
      Wrong RHS/member and foreign provider/schema/world replay controls reject;
      this remains source proof support, not executable opaque composition.
      Defining opaque packs now replay exact declaration-owned formal capture
      eligibility and complete ordered by-value ParamTail fields, transporting
      actual ownership origins into the public OpaqueTail. Genuine compose
      returning impl Fn passes engine CHECK at workers 1 and 8 without hidden
      representation or codegen demand. Publication/occurrence/row forgeries
      reject; repeated-family fields, fixed/nested reference substitutions and
      native captured opaque composition remain explicitly unsupported
      (`CLOSURE-RESULTS.md`). No full corpus-11 or full Rust claim follows.
      Ordinary callable membership additionally requires a parent-minted source
      recipe/final-shape seal. Finalized parent correspondence replays every
      receiver mode's immutable typed payload before child execution, rejecting
      raw or copied-seal metadata, foreign recipes and changed body payloads.
      This authority prerequisite does not authorize arbitrary callback-based
      generic substitution. Generic parent providers now retain the exact ordered
      owner prefix plus their own receiver/tuple/result binders. Canonical template
      substitution replays the original shape seal and discharges inherited owner
      predicates in the caller's real environment; symbolic closure Output retains
      sealed normalization evidence rather than pretending an open goal is closed.
      Five stock/native owner cases cover CHECK/BUILD/run workers 1/8, including
      unused move/bound rejection (`closure_generic_owners`), with ordered-prefix,
      copied-shape and foreign-recipe authority controls. Reference results,
      closure-backed opaque returns and the parent-solved layout frontier remain
      open. A borrowed Closed-facts reconstruction helper now replays the
      original sealed owner template under canonical ordered actuals and discharges
      its real predicates, retaining the same seal. Three source-publication
      authority units cover bound/phase/arity/template misuse, foreign recipes
      and changed sealed results. Memory-effect/key decoder integration remains
      open and still requires independent exact parent publication/body replay.
      Unqualified closure calls now resolve genuine lexical
      bindings before item/import lookup, preserving local/parameter shadowing
      and rejecting unused non-callable or invalid item calls
      (`tests/closure_lexical_calls.rs`, explicit worker-1/8 CHECK/BUILD/run).
      Unique canonical trait-method bounds now constrain closure input/result
      inference before checked metadata exists, including exact owning associated
      input projections. Required predicate rows and outer equalities remain
      obligations; executable ABI projections still reject. Concrete/associated
      positives and unused ambiguous/wrong-result calls have explicit stock/native
      worker-1/8 acceptance (`closure_trait_method_inference`), with owner/binder
      replay controls. A cyclic assignment control rejects at the earlier closed
      projection boundary, not a cycle-specific proof. Unchanged matrix corpus
      E0282 remains open at its iterator-map closure (`CLOSURES.md`).
      Corpus 11 no longer stops at E0282. Its first stop is
      `fold(i32::MIN, i32::max)`: `i32::MIN` itself works in generic calls; the
      blocker is function items (`i32::max`, `S::f`, bare `f`) used as values,
      which resolution misreads as associated constants (no FnDef type yet).
      Source `copied`/`cloned` adapters and `any`/`all`/`position` land, and
      primitive operators forward one shared reference layer (`x*x` for
      `x:&i32`), stock-differential in `tests/iterator_associated.rs` and
      `tests/operator_inference.rs`. `map`/`filter` remain blocked: closure
      types have no nominal realization coordinate (`coordinates.rs`,
      `nominal_inputs.rs` closure fence), and `FnMut(&I::Item)` bounds hit
      "projection in nominal metadata". `(1..=10).collect()`, `sum`, `zip`,
      `rev`, `chunks` are also missing. Source
      `Iterator::count` and bound-owner generic trait method actuals
      (`B::mk(..)` under `B: Mk<A>`) pass worker-1/8 stock differential tests
      (`tests/iterator_associated.rs`). Still missing: source `map`/`filter`/
      `cloned`/`collect`/`sum` adapters, which are blocked by `&i32 * &i32`
      reference operators and by closure metadata in nominal realization.
      Stored parameter patterns retain actual source binders and ordinary checked
      entry destructuring. Three positives and seven unused invalid cases pass
      explicit stock/native CHECK/BUILD at workers 1 and 8, with successful runs
      for valid sources and CHECK no-codegen assertions. Source/binder/projection
      forgery controls and all 14 graph gates pass
      (`CLOSURE-PARAMETER-PATTERNS.md`). Unchanged corpus 11 advances to the
      associated-equality admission boundary, not successful program execution.
      Unauthenticated cross-function macro capture occurrences remain unsupported.
      Generic-parent closures, opaque returns and iterator
      integration remain open.
      Implementation/default closure syntax now retains canonical owner ordinals
      and selected-body occurrence authority, with final resolver publication
      after all existing body rewrites. Ordinary associated/shared-self/equal-name
      method closures and unused invalid bodies have worker-1/8 acceptance
      (`tests/closure_method_parents.rs`). Generic/default owner metadata is
      covered separately, not executable generic-parent support.
- [ ] Extend monomorphization/recursive instance planning to the new forms while
      retaining universal checks, binder-qualified substitution and sealed native
      instances. Optimization must not bypass unused-body validation. Shared
      snapshot-local target codec (`INSTANCE-KEYS.md`) retains ordered type frames
      and validates decoding. Associated-constant/opaque consumers and their
      sealed use authority remain separate open work.

**Next:** extend equality to composite symbolic relationships and
unconstrained projection facts, then projected defaults and general associated
bounds, with authoritative carrier and Copy proofs before an Iterator-like source client. Closure
capture design can proceed independently against the agreed place/region API.
**Acceptance:** generic bodies prove obligations without favorable concrete
witnesses, ambiguous/overlapping impls reject, and concrete dispatch, closures,
iteration and suspended-state destruction each have executable oracle cases.

### G5. Expansion, compile-time evaluation and generated work

- [ ] Implement token trees, macro_rules matching/repetition, hygiene and mixed
      expansion/name resolution, including crate identity, spans and recursion.
- [ ] Complete attributes/cfg/cfg_attr, derives and function/attribute/derive
      procedural macros with host/target separation and compatible token/span APIs.
- [ ] Implement const/static/const-fn evaluation, promoted values, const generics,
      target-aware arithmetic/layout, allocation/provenance and diagnostic rules.
      The bounded shift evaluator is not general const evaluation.
      Delivered: `usize` source constants with u64 semantics pinned to the
      frozen 64-bit native target (CONST-EVAL-SCALAR.md); `isize` stays
      Unsupported. Array lengths remain literal-only.
      Body inference now reads the same sealed scalar constant type as lowering
      before numeric fallback; named/qualified `usize` range endpoints and unused
      incompatible stores pass stock/native controls at workers 1 and 8, plus
      graph14 (CONST-EVAL-SCALAR.md). The unchanged matrix still rejects on closure
      parameter inference; G0-G12 and full const/Iterator compatibility stay open.
- [ ] Make newly expanded/generated declarations explicit graph discoveries;
      track observed files/environment and conservatively disable reuse for
      uncontrolled effects. Enforce execution/resource/isolation policies.

**Next, following the published native lexer:**

- [x] Migrate the actual root-context AST, resolver and retained source replay to
      checked, store-owned semantic identifiers and structured paths. Preserve
      the existing source corpus and resource fences. A name side table beside
      authoritative string lookup does not close this gate. SEMANTIC-NAMES.md
      records the delivered owned stores/paths, actual loop-label integration,
      retained-plan owner checks and explicit fixture builders. The complete
      root-context vertical migration is accepted under MODULE-IDENTITY.md;
      mixed-site/expanded semantic names remain a separate open gate.
- [x] Replace IR restricted-visibility and nominal/module prefix String authority
      with checked module identities retained by the source program/catalog.
      Source modules predate fresh declaration snapshots, so a DeclId or a
      rendered path is not a substitute for a checked module owner.
  - [x] Complete the extracted root resolver using checked path/name keys,
        canonical module identities and typed catalog trait bindings. Preserve
        namespace separation, alias cycles, lexical shadowing and reexport privacy.
  - [x] Universally validate every name/path/type/binder and module restriction,
        including unused bodies and public metadata, with charged iterative
        traversal before declaration or body authority is published.
  - [x] Retain the same immutable name/module owners through Program, catalog,
        planning and replay. Reject foreign owners even for empty programs,
        public declarations and same-spelling unused metadata.
  - [x] Serialize persistent instance identities from checked structural crate,
        module and item descriptors, never snapshot-local numeric handles or
        diagnostic paths. Preserve source-order and alpha-renaming invariance.
- [x] Add checked owning token slices, logical macro cursors and opaque literal
      fragment transport without flattening forwarding opacity or granting
      semantic publication authority. The actual checked utility and signed
      forwarding representation are documented in MACRO-RULES.md.
- [x] Validate a pure ordered `tt`/`ident`/`literal` matcher/transcriber, including
      partial-literal commitment, repetition cardinality, lazy selected-arm
      transcription failures and monotonic resource accounting. Independent
      public tests include 81 stock cases and 14 utility tests; complete
      coordinated acceptance is 1,951 tests/98 binaries with strict lint/fmt.
      Utility output alone is not native source macro acceptance.
- [x] Deliver the bounded function-local `tt`/`ident` expression client with
      owner-checked definition/invocation frontiers, actual lexical BindingId
      resolution, expanded token ingress and retained original-source replay.
      SOURCE-MACROS.md records 17 stock cases / 48 native workers-1/8 CHECK/BUILD
      cells, including real introduced/captured binders. The complete frozen
      hosted suite passes 2,070 tests / 102 binaries, strict lint and formatting.
      Root public admission stays closed; private expansion proofs do not grant
      body or publication authority.
- [x] Consume opaque `literal` fragments at the native expression boundary without
      flattening matcher forwarding. Preserve signed minima and existing numeric/
      bool semantics, normal-string println only, typed suffix phases and retained
      output capacity. OPAQUE-LITERALS.md records 21 stock cases / 76 native cells
      plus the unchanged first source corpus; the complete frozen hosted gate
      passes 2,089 tests / 103 binaries, strict lint and formatting.
- [x] Admit multiple original function-local definitions, textual shadow/redefinition
      and nested/recursive expression calls with exact selected-definition and
      parent-proof ownership. NESTED-SOURCE-MACROS.md records 17 added stock
      sources / 68 native cells, unchanged earlier corpora, actual stack-frame
      repair and the complete 2,105-test / 104-binary hosted gate.
- [x] Admit original module-level definitions and item-position calls emitting
      one supported free function, with explicit module proof, real generated
      function scopes, sealed inactive syntax and retained source publication.
      ITEM-SOURCE-MACROS.md records 29 stock sources / 116 native cells, thirteen
      public authority controls and the complete 2,156-test / 106-binary hosted
      gate. Exact original-use certificates preserve generated use authority
      through canonical resolution. Generated cfg/modules, exports and general
      item expansion remain outside this bounded client.
- [x] Implement zero/many nongeneric free-function outputs for original module
      item macros, with complete native-token consumption, distinct function
      occurrence/scope/source-callable identities and explicit empty close.
      Completion is an immutable scalar after publication, added inline to the
      existing paid anchor, with exact sealed-product/function coverage. The
      detached `942b537dc` lane passes 42 stock cases / 168 native workers-1/8
      CHECK/BUILD cells, unused bad first/second/repeated sibling type checks,
      18 authority controls, unchanged macro regressions and graph-integrity
      gates. See ITEM-SOURCE-MACROS.md and data/runs/macro-many/. Composed
      primary-branch acceptance/integration remains coordinator-owned. This
      adds no exports, proc macros, new fragments or general hygiene claim.
- [x] Validate source-owned macro routing across native prelude and mounted
      dependency crate boundaries. SOURCE-MACRO-BINDINGS.md freezes the bounded
      G0-G12 acceptance contract, immutable per-file crate ownership, common
      probe/import/selection policy and workers-1/8 stock differential gates.
      This repairs lexical binding isolation, not general macro imports/exports
      or full Rust hygiene. No new graph join or mutable fact state is admitted.
- [x] Preserve sealed bare receiver occurrences and exact callable receiver
      binders through macro parsing, resolution, name replay and typed local
      lookup. Authenticated imported module definitions use an empty local
      definition frontier, never invocation bindings, while metavariable captures
      retain call-site identity. MACRO-RECEIVERS.md records stock/native workers
      1/8 execution, unused invalid receiver/local leakage rejection, forged
      occurrence/scope/source controls and no-codegen graph acceptance. Generated
      callable container/header authority remains independently required.
- [ ] Generalize nested/recursive expansion and namespace-specific hygiene,
      generated declarations, cfg-before-child-IO and graph discovery closure.
      Add real item/export/import/cross-crate clients and remaining fragment/AST
      consumers before claiming general macro support.

The macro specification input is Rust Reference commit
`f1e8ba23e46236877fe90c86e4cb7075ad37b776`,
`src/macros-by-example.md` (27,548 bytes, SHA-256
`32e55cf4b4f62b969d1348b15d30b6f807501e32cfa04133e2b0bc5e944dcc67`).
This document pin is independent of the executable stock compiler revision,
which each oracle run must record. No source-matched compiler claim follows.
General target-aware const-value contracts and array-length const evaluation
remain separate open work.

- [x] Original-source free immutable fixed-width integer/bool constants use
      per-declaration CONST_DECL/CONST_BODY/CONST_VALUE graph facts (118/119/120),
      checked scalar evaluation, all-reference qualification and lazy executed
      value reads. Unused invalid items fail CHECK; exact source-use replay and
      immutable paid publication receipts prevent detached value substitution.
      Frozen `dd403c1cd` passes 3,472 full-suite tests plus strict hosted lint and
      formatting. See CONST-EVAL-SCALAR.md and data/runs/const-source/. Target-sized
      types, array lengths/repeats, const patterns, generated/associated consts,
      statics and general const-fn execution remain unsupported; corpus stays 3/20.
- [x] Source-backed `i32::MIN` and primitive aliases use loader-issued immutable
      source/assembly seals, exact original impl occurrences and canonical owner
      registration before ordinary associated-constant facts (134/135/136).
      Native stock differential CHECK/BUILD/run passes at workers 1 and 8;
      user primitive impls reject E0390 and unused invalid bodies reject E0308.
      Transport/issuer forgery controls, CHECK without codegen and graph 14/14
      pass unchanged. See PRIMITIVE-CONSTANTS.md and six authentic receipts in
      data/runs/xps14/2026-10-19-primitive-min.jsonl. Other primitive constants,
      general const evaluation and G0-G12 completion remain open.
- [x] Embedded nongeneric free-function occurrence authority binds original
      loader-issued source, complete supported header/body recipes, authenticated
      resolver call correspondence and canonical declaration registration.
      This is a G0/G1/G2/G9/G10/G12 prerequisite only, not completion of any gate.
      Six focused authority filters pass across recorded checkpoints; the full
      graph attempt passed 13 gates, then its sole new test-owner violation was
      repaired and the affected authority control/scanner passed separately.
      See EMBEDDED-FUNCTION-AUTHORITY.md and 12 authentic receipts under
      data/runs/embedded-function-authority/. Packed-byte query admission remains
      inactive; no allocation, native-memory, String/Vec or full-Rust claim follows.
- [x] Ordinary source `Ord::max(self, other: Self) -> Self where Self: Sized`
      uses canonical trait/default selection and returns the second argument on
      equality, including non-Copy values. Direct method, UFCS, primitive alias
      and generic calls pass stock/native CHECK/BUILD/run at workers 1 and 8;
      user overrides remain canonical. Membership/type/unused-body/move and
      unrelated-member controls reject; genuine selected-signature, retained
      Sized-predicate and foreign-member/world replay units pass. Graph 14/14
      remains unchanged. See ORD-MAX.md and eight authentic receipts in
      data/runs/xps14/2026-10-19-ord-max.jsonl. First-class function-item values,
      Ord::min/clamp and unchanged corpus-11 acceptance are not delivered here.
- [ ] Complete associated constant evaluation and lookup (ASSOCIATED-CONSTANTS.md).
      The scalar draft supports trait defaults/overrides, inherent generic owners,
      qualified and bound-authorized symbolic uses, and exact initializer member
      dependencies through per-use header/selection/value facts (134/135/136).
      Twenty-three stock cases pass native CHECK without artifacts and BUILD/run
      at workers 1 and 8, including skipped-reference cycles and unused bad types.
      General const-fn execution and unchanged time-core acceptance remain open.
- [ ] Complete generated-item metadata and derive expansion (GENERATED-METADATA.md).
      The bounded unit-struct slice retains exact original metadata/item/derive
      occurrences and emits real Copy/Clone/Debug implementations. Canonical
      source fmt signatures and derived executable scopes have separate replayed
      authority. One original combined derive consumer passes stock and native
      CHECK/BUILD/run at workers 1 and 8. Four exact metadata/member provenance
      units and the unchanged graph-integrity gate pass on the current-parent
      candidate. Generic/field/enum recipes and unchanged time-core remain open.

**Acceptance:**
expansion order/hygiene, const failures and generated diagnostics match pinned
cases, invalid unused expanded items reject, and changed cfg or observed inputs
cannot reuse stale expansions. Proc-macro acceptance executes a real native
host macro and compiles its generated target source.

### G6. Runtime, FFI, linking and target expansion

- [x] External Linux x86-64 C ABI slice: original extern C declarations and
      definitions, repr(C) scalar/nested aggregate layout separate from private
      carriers, INTEGER/SSE register classification with complete aggregate
      rollback, stack arguments, mixed/two-register and hidden-pointer results.
      Direct ELF64 relocatable emission and native object/archive/library linking
      support real C callbacks and C-main executables. See `C-ABI.md` and the
      worker-1/8 native, authority and graph gates. G6 remains open for variadics,
      unwind, TLS, atomics, aggregate pointers, further repr/ABI forms and targets.

- [x] Native primitive `{:?}` for integers/bool/unit and recursively Copy
      tuples/fixed arrays, with immutable per-argument modes, shared place loans,
      stock differential streams/evaluation and forged IR/CFG authority gates.
      See `FORMAT-SCALARS.md`. This is not source `std::fmt` trait dispatch.
      Char/string escaping, float conversion, nominal/user Debug and specifiers
      remain open.

- [x] Bounded debug assertion macros use an explicit native program setting,
      independent of the bootstrap profile, with checked disabled operands and
      zero disabled evaluation. Frozen 53-source enabled/disabled debug/release
      CLI/graph matrices, seven authority controls and full 3,158-test/162-binary
      regression passed. See `DEBUG-ASSERTIONS.md`; general formatting, trait
      equality and full Cargo profile support remain open.

- [x] Bounded guarded `matches!` evaluates guards only after a successful pattern,
      with genuine Copy proof and immutable-binding restrictions. The 23-source
      debug/release corpus, twelve authority controls, promoted original witness,
      and full 3,140-test/160-binary suite pass. See `GUARDED-MATCHES.md`; general
      match guards and non-Copy/mutable binding guards remain open.

- [x] Bounded `matches!` lowers once-evaluated scrutinees through existing checked
      match/pattern machinery, preserving Bool results and macro/source authority.
      Frozen 21-source debug/release CLI/graph matrices, six public authority
      controls, and complete 3,116-test/158-binary suite passed. See
      `MATCHES-MACRO.md`; guards, ranges, character literals and general macro
      execution remain outside this slice.

- [x] Bounded `unreachable!`, `todo!`, and `unimplemented!` reuse checked literal
      panic lowering with exact default/custom prefixes, source locations and
      Never control flow. Frozen 27-source debug/release CLI/graph matrices and
      complete 3,103-test/156-binary suite passed. See `DIVERGENT-MACROS.md`;
      formatting, qualified/module macro execution and unwinding remain open.

- [x] Bounded literal `print!`, `eprint!`, and `eprintln!` preserve exact separate
      stdout/stderr bytes with checked macro/source authority and metered payloads.
      Frozen 20-source debug/release CLI and graph matrices, seven public authority
      controls, eleven private controls and complete 3,088-test/154-binary suite
      passed. See `LITERAL-OUTPUT.md`; general formatting, buffering and platform
      IO compatibility remain open.

- [x] Native bare `{}` scalar print formatting for integers through 128 bits,
      bool, char and shared str, with once-only left-to-right argument evaluation
      and shared place loans. Frozen scalar stock corpus and graph-integrity
      gates pass; first full-program corpus source `01_hello` executes with exact
      stock stdout. See `FORMAT-SCALARS.md`; general Rust formatting remains open.

- [x] Bounded `assert_eq!`/`assert_ne!` use existing Unit/Bool/integer/float
      comparisons, evaluate operands once in source order, and retain checked
      literal panic locations. Stock-first 27-source debug/release CLI and graph
      matrices, public authority controls, and the complete 3,065-test/152-binary
      local suite passed. See `EQUALITY-ASSERTIONS.md`; general PartialEq/Debug,
      references, aggregate assertions and value formatting remain open.
- [x] Bounded `assert!` evaluates a Bool condition once, yields Unit on success,
      and uses checked literal panic on failure. Parenthesis/bracket/brace and
      trailing-comma forms, lexical shadowing, grammar fences, stock/native
      debug/release and public authority controls are covered. Full local suite
      passed 3,049 tests/150 binaries. See `ASSERTIONS.md`; formatting assertions,
      general equality/debug assertion semantics and complete canonical flow replay remain open.
- [x] Bounded literal/empty `panic!` now preserves checked invocation locations,
      exact Never completion and terminal CFG control flow, and emits a native
      Linux x86-64 stderr diagnostic followed by SIGABRT. Stock-first 47-source
      debug/release acceptance, public authority and hostile-signal/syscall
      controls passed; complete local suite 3,024 tests/147 binaries, hosted
      check/strict lint. See `PANIC.md` for receipts and the hosted stock-abort
      environment limitation. Formatting, unwinding/Drop cleanup, core-file
      compatibility and general runtime support remain open below.
- [ ] Define target data layouts/calling conventions, object/relocation/symbol
      formats, crate artifacts and linking, repr(C)/transparent/packed/aligned,
      extern calls/statics, native libraries, variadics and stable ABI boundaries.
- [ ] Implement allocation/deallocation, startup/termination, panic abort/unwind,
      cleanup/personality support, TLS, threads, atomics/memory ordering, intrinsics,
      target features and inline/global assembly for the declared target contract.
- [x] First Linux x86-64 heap foundation: exact unsafe raw syscall ABI primitive,
      ordinary per-file Rust-source native_heap library with checked byte-address
      arithmetic/null helpers, bounded mmap layout/error policy and private
      non-Copy allocation ownership with consuming munmap. Real mapping-boundary
      scalar access tests require native memory widths rather than qword local
      carriers. See NATIVE-HEAP.md. This does not close allocation compatibility:
      general layouts, pointer methods, full Box/Vec/String behavior
      and full alloc/core/std remain open.
- [x] Standalone private storage stride extraction: authenticated typed carrier
      extent replaces fake public-size pointer arithmetic in ordinary source
      Box allocation/free, with existing raw allocator and entry graph unchanged.
      Standalone stock/native workers1/8, source/query authority and graph14 passed
      (582233krek); see PRIVATE-STORAGE-STRIDE-FEATURE.md.
      Public size/layout/niche, native pointer compatibility and full Box remain open.
- [x] Bootstrap raw mapping realloc uses Linux mremap with relocation allowed,
      preserves old storage on failure, and drives Vec/String capacity growth.
      Stock-differential page-crossing scalar and owning-element growth runs at
      workers 1 and 8 (`tests/runtime_realloc.rs`). Allocation exhaustion now
      diagnoses before installing a null backing pointer.
- [x] Ordinary source `Allocation` Drop reuses canonical normal-path cleanup,
      disarming the handle before shared munmap policy so explicit release does
      not free twice. Raw-pointer scalar fields and ordinary owner borrows use
      normal authority/loan checks. Moves, early returns, conditional cleanup,
      replacement/loop flags, borrow escapes and panic-abort controls are tested.
      Generic/aggregate owner Drop glue and Box<T> remain open.
- [x] Generic functions carrying supported nongeneric Drop owners demand each
      concrete canonical destructor and freshly lower/seal the closed cleanup
      shape. Stock/native exact-once counter and mmap lifetime tests cover moves,
      branch/early return, replacement and break transport. Missing cleanup,
      stale universal CFG shape and missing destructor demand reject. Generic
      owning structs, nested aggregate glue and typed drop-in-place remain open.
- [x] Bounded generic named-field owners containing only scalars/raw pointers
      retain distinct concrete canonical Drop targets and symbols per type
      arguments. Stock/native exact-count execution and Copy, explicit drop,
      move, borrowed replacement and invalid specialization controls pass.
      Additional nominal/impl bounds, owned parameter fields, nested glue,
      typed layout/drop-in-place and actual source Box<T> remain open.
- [x] Main-thread literal/formatted panic-abort diagnostics include the runtime
      process ID, original source location, message and stock default backtrace
      hint. PID-only-normalized exact stderr compares with stock at workers 1/8
      (`tests/runtime_panic_header.rs`, RUST_BACKTRACE=0). Backtrace generation,
      unwind mode remains open. Source indexing diagnostics are covered separately
      by the bounded `RUNTIME-BOUNDS.md` contract.
- [x] Native scalar/wide integer overflow, division/remainder by zero and invalid
      shifts retain original source locations through checked graph facts, CFG,
      SSA inlining and constant folding, including location-sensitive cache keys.
      Stock panic=abort stderr and SIGABRT agree for 13 arithmetic forms at
      workers 1/8 with optimization off/adaptive/final (`tests/runtime_traps.rs`).
      Unwinding and compound-assignment arithmetic site coverage remain open.
- [x] Source-aware builtin array/slice bounds aborts retain exact original sites,
      full-width unsigned index/length and stock effect order. Nonmoving probes
      cover shared/mutable non-Copy borrowing, ignored values and checked writes,
      with canonical source/CFG forgery controls (`RUNTIME-BOUNDS.md`). This advances
      G0/G2/G3/G6/G9/G10/G12 without closing any full-Rust gate. Synthetic descriptor
      faults, range panic diagnostics and unwinding remain separate/open.
- [ ] Provide debug/source information and runtime failure/exit diagnostics.
      Expand architectures/OSes only with target-specific executable evidence.

**Next:** freeze the initial Linux x86-64 ABI/layout matrix and bidirectional
native C harness, including aggregates and callbacks. **Acceptance:** platform
harnesses link and execute both directions, layout/ABI match the platform,
unwind/abort and thread/TLS/atomic tests have defined outcomes, and unsupported
targets reject explicitly. The current private aggregate ABI does not pass this
gate, nor does successful direct ELF emission alone.

### G7. Native core, alloc and std bring-up

Native-prelude progress: `Vec<T>: Index<usize> + IndexMut<usize>` supports value
reads, assignments/compound assignments, shared/mutable element borrows and
indexed formatting through ordinary source trait calls. `tests/vec_index.rs`
compares CHECK/BUILD/output at workers 1 and 8 with stock Rust, checks
read/write/borrow out-of-bounds aborts, immutable-place rejection, and exactly-once
non-Copy indexed formatting. Range indexing remains a separate compiler/library
blocker, not full Vec or std acceptance.

`Vec<T>` additionally implements source-level `contains`/`dedup` for `PartialEq`
and stable `sort`/`binary_search` for `Ord`. `tests/vec_algorithms.rs` exercises
empty inputs, insertion positions, generic non-Copy elements and stable equal-key
ordering against stock Rust at workers 1 and 8. Source-level `Vec::as_slice`
and `as_mut_slice` delegate to the canonical raw-slice constructors, including
aligned non-null empty pointers and generic non-Copy descriptor transport.
`tests/vec_slices.rs` covers stock CHECK/BUILD/native effects at workers 1 and 8,
conflicting aliases, local escape, and CHECK graph isolation. See
`VEC-SLICES.md`. Iterator-dependent methods and further slice delegation remain
open, as do native-memory pointer-layout compatibility and the rest of G7.

Active source `String::trim` recognizes Unicode White_Space at UTF-8 scalar
boundaries. Prefix/suffix checks compare bytes without creating an invalid
intermediate `&str`, and zero-capacity strings retain non-null dangling storage.
`tests/string_scalars.rs` compares CHECK/BUILD/output at workers 1 and 8 with
stock Rust over all 25 whitespace scalars, non-whitespace controls, multibyte
prefix/suffix boundaries, empty strings and growth after zero capacity. This
is not full `str`, casing, parsing or Iterator acceptance.

- [ ] Establish pinned library source manifests, language-item/intrinsic inventory,
      sysroot identity and library feature/target configuration without embedding
      rustc or pretending catalog recipes compile the libraries.
- [ ] Compile and exercise no_std/core first, then alloc with an actual allocator,
      then target std with platform runtime support. Feed every new blocker into
      G0-G6 with a minimized reproducer and dependency owner.
- [ ] Cover canonical library trait semantics, collections, strings, formatting,
      I/O, synchronization, panic handling and library tests in executable clients.

**Next:** attempt an inventoried pinned core discovery/check run and publish the
first unsupported blockers rather than claiming a successful library build.
**Acceptance:** each library stage compiles on the native path, links real clients
and runs the declared tests. No stock-compiled library artifacts may conceal
missing native compilation. Catalog Copy/Clone support is not core acceptance.

### G8. Existing Cargo projects and source-to-test workflows

**Test-outcome correction (2026-10-20):** default library test requests now
fail explicitly as Unsupported while native doctest discovery/execution is
unimplemented, rather than reporting invented successful zero-test results.
Explicit unit/integration/binary/all-target selections and `--no-run` exclude
doctest execution. `[lib] doctest = false` is honored and nonboolean values
reject. This closes a false-success path, not the doctest compatibility gate.

**In progress (2026-10-20):** `TARGET-DEPENDENCIES.md` defines native-target
conditional dependency admission, inactive-edge filtering and feature propagation
across repeated conditional declarations. `PROJECT-CONFIGURATION.md` defines
explicit edition/profile validation. These changes require stock-differential
acceptance and do not close G8, provide build scripts/proc macros, or establish
self-hosting. Final validation evidence belongs in the feature contracts.

- [ ] Parse existing manifests/workspaces, targets/profiles and configuration;
      implement lockfile-compatible package/version resolution, registries/git/path
      sources, patches, features/resolver versions and host/target dependency kinds.
- [ ] Integrate multiple crates, generated inputs, build-script directives,
      environment/rerun rules, native tools/libraries and proc-macro compilation
      and execution as graph operations, not a Cargo-wide phase barrier.
- [ ] Support unit/integration/doc tests, test discovery/filtering and exit status,
      examples, benches and requested outputs with compatible diagnostics.
- [ ] Keep explicit stock fallback separate and labeled. Never send private
      formats to stock tools or count fallback as native compatibility.

**Progress (2026-10-19, rough draft):** `jr-rust check|build|test|run` drives
Cargo projects natively (`src/project/{manifest,driver,cli}.rs`): workspaces
(members globs, exclude, default-members, `workspace.package`/`dependencies`
inheritance), target discovery (lib, main, `src/bin`, `tests/`, `examples/`,
explicit tables), path dependencies mounted as extern crates
(`SourceOptions::externs`, SOURCE ordinal `[2+i]`, `crate::` re-rooted inside
each mount), features with `dep:`/`pkg/feat`/optional deps unified across the
build, `dev`/`release`/custom profile `debug-assertions`, `#[test]`/`#[ignore]`/
`#[should_panic(expected)]` discovery under `cfg(test)` with a native fork-per-
test harness (`libraries/prelude/test.rs`) matching libtest output and exit 101.
`tests/projects.rs` compares a two-crate workspace with stock Cargo. Native
offline locked registry packages now parse from `Cargo.lock`, resolve only from
the local Cargo cache with pinned source/checksum identity, and fail explicitly
for cache misses or checksum mismatches instead of path-rewriting or stock
fallback. Archive checksums now authenticate every extracted regular file;
foreign registries, forged cache IDs, modified/extra source files, symlinks,
escaping targets, ambiguous lock identities and incompatible manifest requirements
reject explicitly. Tests seed unchanged licensed archives into isolated cache
homes, without depending on hosted HOME cache contents. On private parent
`d98eb5aea`, the unchanged `utf8-width 0.1.8` positive currently stops at the
unsupported `no_std` crate attribute. CHECK/BUILD/run stdout `3` remains a pending
acceptance gate, not a result of this registry-only patch. `nix-base32 0.2.0` and
`time-core 0.1.2` are diagnostic probes, never rewritten to clear boundaries. Open: git
sources, build scripts, proc macros, doc tests, test filters/harness flags,
parallel units, per-crate graph identity (dependency crates are currently
re-checked inside each dependent unit).

**Independent no_std prerequisite (2026-10-19):** `NO-STD.md` freezes source-owned
crate policy, canonical core-prelude selection, defining-crate inheritance and
explicit std opt-in boundaries. Original metadata is parsed completely before
private policy issuance. Public attribute audit records cannot grant policy;
original-source and snapshot identities are checked on consumption. Mixed
ordinary/no_std dependencies and unchanged utf8-width have passed CHECK/BUILD/run
at workers 1/8 on earlier private snapshots. Final rebased-tree gates remain
pending. The standalone frontend `lex` owner entry is removed, tightening the
graph ratchet. This is not full core/alloc/std support, nor a hosted release claim.

**Progress (2026-10-18):** `src/project/toml.rs` is a native TOML 1.0 reader
(offsets, line/column errors, duplicate/redefinition/inline-seal rules, bounded
depth) with unit tests (hosted run `20261018-142549166-1772846`). It is not yet
checked against an external TOML conformance suite. No manifest, workspace,
lockfile or feature model exists yet.

**Next:** freeze a tiny unchanged workspace containing path dependencies, features,
a build script, a proc macro and tests, plus manifest/resolution fixtures that can
run before native compilation is available. **Acceptance:** lockfile/feature and
host/target selections are correct, unchanged project commands execute native
outputs/tests, failures remain visible, and edits to generated/native inputs
invalidate the necessary graph. Progress from this fixture to pinned independent
real crates and multi-crate applications, not just adapted source snippets.

### G9. Integrated graph, persistence, security and operations

Construction-final declared equality reductions now move from explicit local
sinks into immutable contexts before world publication (DECLARED-REDUCTIONS.md).
The actual reductions mutex removal tightens lexical debt by two tokens and
reachable debt by exactly ten entries, 72 to 62. PlanCtx, nominal mint/interner,
world proof/projection memo and budget debt remain; no wide activation is implied.

Bounded private source metadata extraction is documented in SOURCE-SCHEMAS.md:
five final tables move once into an immutable paid owner, with exact parameter
storage and authoritative compact callable projections. The unchanged graph
floor and strict lint pass on af5c06a69. This is partial progress only: mutable
PlanCtx/interner/source-context debt and wide adapter activation remain held.

- [ ] Split measured coarse source/resolve/signature work into independently
      publishable facts with complete declaration-discovery prerequisites and
      supported recursive convergence protocols, not hidden phase barriers.
- [ ] Define stable cross-edit identities and versioned full semantic keys,
      dynamic/negative-input validation, fine-grained invalidation and exact
      build/check/test variant reuse verified against clean recomputation.
      Source-file foundation: existing per-file facts now retain bounded immutable
      observations of consumed bytes, canonical paths, file-kind checks and both
      module lookup candidates (including absence), plus an exhaustive framed
      identity of supplied `SourceOptions`. `justrust test --lib source_inputs`
      covers mutation, negative lookup, symlink retargeting, configuration, limits
      and real CHECK/BUILD/run at workers 1/8. Revalidation is not an atomic
      snapshot or cache authorization. Native project loading additionally
      records manifest/lock bytes, canonical and negative path probes, sorted
      directory listings, initial manifest search, and supplied `CARGO_TARGET_DIR`,
      `CARGO_HOME` and `HOME` values/absence. Its immutable discovery receipt is
      supplied through `SourceOptions` into the existing per-file graph fact
      (`justrust test --lib project::inputs` and `--lib source_inputs`). The
      existing project loader is not yet graph-scheduled. Native path dependency
      identities are frozen from observed loading, so feature and visibility
      planning no longer reopen or redirect those paths (regression:
      `path_planning_uses_observed_identity_after_symlink_mutation`). Reviewed identity
      probe records also retain bounded lock/provider path and byte observations in
      the existing prepared-source fact (`justrust test --lib source_probe` and
      `--test core_identity_graph`), independently revalidated from client inputs.
      Embedded prelude units retain domain-specific byte/path receipts replayed
      only against the compiled source table, and embedded provenance issuance
      and validation require those receipts (`--lib source_inputs` and
      `--lib source_embedded`). Host files cannot satisfy the embedded domain.
      Source compilation entry points also capture bounded full running-executable
      bytes through a pinned Linux process-image handle. The immutable compiler
      image digest enters source configuration identity and existing prepared-source
      graph output, from which compilation results return it (`--lib procfs`).
      Caller-supplied image stamps are replaced; absent/oversized images remain
      unproven rather than authorizing reuse. Synthetic Program fixtures do not
      claim image coverage. Executable file-backed runtime mappings now add bounded
      content identities, inode/device checks and whole-capture revalidation; deleted,
      unknown or writable executable mappings fail closed (`--lib procfs`). Current
      directory, argument order, raw environment values and Linux resource limits
      also enter the digest and are rechecked after capture. Environment order is
      canonical, duplicate names and bounded-capture exhaustion reject. Frozen
      CHECK/BUILD roots consume the graph's runtime receipt and revalidate it
      before publishing success, rejecting absent/forged runtime evidence.
      Legacy discovery observes both Cargo config spellings through invocation
      ancestors and CARGO_HOME, including negative probes; present unsupported
      configuration disables input identity (`--lib project::inputs`).
      These identities do not seal arbitrary in-memory code or process state.
      Caller-owned `SourceSnapshot` namespaces can now supply both source reads and
      native manifest/path-dependency discovery through `Driver::from_snapshot`.
      Directory/target discovery and explicit environment values use that same
      immutable namespace; graph source receipts reject a different discovery
      snapshot. Frozen projects reject Cargo configuration files, git/network
      acquisition, uncontrolled environment settings and build scripts, while
      native linking rejects uncontrolled external linkers. Checksum-locked
      crates.io packages in canonical local caches now enter the same snapshot:
      selected archive files and source directories are captured together under
      the aggregate namespace limits, without sweeping unrelated cache entries.
      Authentication compares every source byte with the owned SHA-256-locked
      archive using bounded in-process gzip/tar decoding, never external tar.
      Archive decoding admits regular files/directories in basic GNU/ustar headers,
      caps compressed bytes at 16 MiB, decoded bytes at 32 MiB and entries at 4,096,
      and explicitly rejects links, sparse/PAX/long-name extensions, duplicate or
      escaping paths, corruption and trailing payloads. Unauthenticated Cargo
      extraction markers are absent from captured source namespaces and cannot
      supply module bytes. Frozen planning reauthenticates only snapshot bytes.
      `project::registry_archive`, `project::inputs::immutable_registry_capture_survives_source_archive_and_manifest_deletion`
      and `project::capture::selected_archives_are_owned_without_sweeping_the_cache`
      cover corruption, limits, forged archive/marker facts, cross-pass mutation,
      negative probes and transitive CHECK/BUILD/run after host inputs disappear.
      `authenticated_registry_capture_checks_builds_runs_transitively_without_tar`
      compares production CLI CHECK/BUILD/run at workers 1/8 with stock execution,
      with no external tools in the native process PATH. Noncanonical cache aliases
      retain an explicitly unproven legacy path, chosen before acquisition.
      Ordinary project
      CLI commands now acquire a bounded regular-file namespace twice, reject
      observed byte/metadata changes, revalidate discovery and compile only the
      owned snapshot. Symlinks/special files reject; target and cache-tagged
      output directories are excluded and cannot satisfy source reads. Unsupported
      configuration and registry acquisition paths remain explicitly unproven. Standalone
      `jr-rust SOURCE` also captures native source directories and explicitly supplied
      extern source directories, including outside-root siblings. Project capture
      admits the discovered native path-package roots, including transitive siblings,
      rather than widening to their common ancestor. The immutable namespace binds
      its canonical deduplicated root set in its digest, with at most 64 supplied
      roots and aggregate 4,096-entry/16-MiB acquisition limits. Reviewed probe and
      external-link compatibility paths explicitly remain unproven. A
      source/runtime/acquisition failure never retries live.
      Acceptance: `justrust test --test compiler` includes stock/native standalone
      CHECK/BUILD/run at workers 1/8 and missing/symlink rejection.
      `justrust test --lib project::capture` and `--test projects` cover ordinary
      CLI CHECK/BUILD/run at workers 1/8, missing inputs, symlinks and mutation
      between capture passes. The `sibling_path_dependencies_are_captured_for_cli_check_build_run`
      and `standalone_cli_captures_sibling_extern_roots_at_workers_one_and_eight`
      tests compare production CLI results with stock for sibling/transitive path
      dependencies and standalone externs. Capture regressions cover cross-root
      mutation/deletion/addition, aggregate limits, negative answers, root aliases,
      unauthorized siblings and compilation after both host roots disappear.
      `justrust test --lib source_snapshot` includes stock/native CHECK/BUILD/run
      at workers 1/8 after host files are changed or removed, workspace globs,
      missing/changed inputs and unsupported configuration/tool boundaries.
      This is not an atomic capture of a live host directory, runtime-process
      sealing or authorization to share artifacts. Network/unlocked/alternate-registry
      acquisition, noncanonical cache aliases, unsupported configuration sources,
      in-memory/process state, external tool
      identity and uncontrolled build scripts/proc macros remain explicit gaps.
      Exhaustion or uncontrolled discovery cannot issue an input identity.

      | Input boundary | Supported capture / remaining status |
      | --- | --- |
      | Native projects, path dependencies, canonical checksum-locked cached crates.io packages, standalone source/extern roots | Bounded repeated acquisition into immutable owned bytes, closed discovery/source/archive reads; CHECK/BUILD/run tested at workers 1/8. |
      | Missing candidates and paths outside authorized roots | Immutable absence / rejection, never ambient host fallback. Symlinks, special files and exhausted limits reject. |
      | Network/unlocked/alternate registry acquisition, noncanonical cache aliases, Cargo configuration, reviewed probes, external tools/linkers | Legacy compatibility remains explicitly unproven or frozen mode rejects. Archive links and unsupported header extensions reject. |
      | Compiler runtime evidence on Linux/x86-64 | Executable/mapped code and bounded process configuration. Canonical MXCSR/x87 control state is required and rechecked across capture; sticky exception status is excluded. Unsupported floating environments cannot mint evidence. Arbitrary mutable process state is not sealed. |
      | Build scripts, proc macros, arbitrary process/in-memory state, atomic host snapshots | Not closed by this namespace capture. Complete-input-capture remains incomplete. |
      | Artifact reuse and global sharing | Explicit opt-in trusted-local reuse is available for supported owned lowering, optimization, machine-object and native-link operations. It does not restore graph authority or authorize global sharing. Global artifact sharing remains disabled. See `GLOBAL-CACHE-CONTRACT.md`. |

      Controlled GNU/glibc native linking now captures the linker, ELF interpreter,
      transitive shared libraries, startup objects and selected c/m libraries into
      owned bytes. Its versioned execution policy invokes the captured interpreter
      directly with a captured-only library path and disabled loader cache, never
      the host PT_INTERP target. Child argv uses stable captured-relative paths,
      with a private staging working directory that cannot be queried through
      getcwd. Linux x86-64 enforcement uses unprivileged
      Landlock ABI >=5 plus seccomp, rather than relying on user namespaces that
      the hosted AppArmor policy does not permit. Inputs are read/execute-only,
      writes are confined to the output directory, inherited descriptors are
      closed on exec, and process/network/namespace operations are denied.
      Because Landlock does not mediate pathname metadata, seccomp also denies
      stat/access/readlink/chdir and O_PATH opens. Descriptor fstat is brokered
      into canonical x86-64 metadata: virtual device/inode identities derive from
      captured relative paths, owners/times/padding are zero, and regular-file
      lengths remain exact. Only regular captured/output files and registered
      stdio/null descriptors are admitted. Directory descriptors, enumeration,
      physical extent queries, O_DIRECT and mincore are denied. Child memory and
      descriptor inspection use pinned proc-directory/mm handles, never a later
      numeric-PID lookup. Bounded mapping checks preserve unreadable-path and
      readonly-destination faults despite proc-memory access privileges.
      Landlock alone does not hide
      host existence through open error codes, so native seccomp notification
      mediation copies bounded open/openat paths and resolves them only beneath
      pinned captured/output directory handles using openat2, without symlinks
      or parent traversal. Writes are admitted only beneath the output handle.
      The broker injects descriptors atomically rather than continuing inspected
      pointer-bearing requests. Only the trusted initial exec transition is
      continued, by a bootstrap receiver started before Command::spawn waits for
      exec; later exec requests and all other pathname mutations are denied
      uniformly. Existing/missing outside paths and outside aliases to captured
      files cannot supply distinct lookup results. The notification ABI is
      checked before native buffers are used. Link supervision bounds elapsed
      time and request count and kills/reaps on failure. Pre-exec capability
      sets are cleared, memory advice is allowlisted, and compiler death kills
      the linker. A parent-owned native exec stop holds the image before its
      first instruction while pinned process handles and stdio identities are
      bound. Bounded initial-stack normalization replaces AT_RANDOM bytes,
      clears the stack AT_SYSINFO_EHDR value and replaces the AT_EXECFN string,
      including erasing its old private-path suffix. Present auxiliary UID/GID
      credential value slots are validated and replaced with fixed identities;
      AT_SECURE and hardware-feature values are not changed. Trusted injected munmap
      operations remove actual vDSO/vvar mappings; exact instruction/register
      restoration and pinned-map readback precede image execution. Failed setup
      kills/reaps the stopped child, never detaches a partially normalized image.
      Initial notification waits and captured diagnostics are bounded as well.
      Execution remains parent-traced with child-only timestamp-counter faults.
      Exact native RDTSC/RDTSCP faults receive an operation-local increasing
      virtual counter and fixed RDTSCP CPU identifier, with register readback;
      other faults retain their original signals. A trace-stop budget and
      tracer-death kill supplement the existing deadline and teardown bounds.
      Tools cannot reenable direct timestamp reads through prctl. The boundary
      also denies set_tid_address, whose return reveals a kernel thread ID, and
      rseq registration, which would admit asynchronously kernel-written CPU
      identity fields into child memory. Architecture control admits only the
      loader's ARCH_SET_FS TLS setup, not feature queries, address queries or
      unrelated architecture-control changes. Raw poll/select/pselect6/ppoll
      are denied: they expose pipe readiness, and the timed variants also
      rewrite timeout buffers from live clocks even with captured-file inputs.
      Hosted acceptance includes real captured GNU ld, stock/native ABI parity
      at workers 1/8, canonical metadata, first-instruction ordering, kernel-map
      removal, deterministic timestamp probes, legacy vsyscall probes, floods
      and parent-death cleanup. Other CPU instructions, address-layout
      observations and arbitrary mutable runtime state are not sealed by these
      checks. Unsupported
      kernels or tools fail closed, without changing host policy or retrying unconstrained.
      Parent output acquisition rejects symlinks and special files, preventing a
      tool-created symlink from escaping confinement when its result is read.
      `justrust test --lib controlled_link` exercises adversarial file-content,
      metadata, mutation, symlink, inherited-descriptor, process and network
      boundaries. Frozen libc linking also runs hosted at workers 1/8. These
      bounded contracts do not establish arbitrary process-state sealing, build-script or
      proc-macro support, durable graph persistence, or trusted sharing authority.
      Global artifact sharing remains disabled.
- [ ] Add bounded asynchronous persistence, atomic manifests, durable checkpoint
      contracts, recovery, eviction and corruption/crash/cancellation tests.
- [ ] Specify authenticated provenance, artifact integrity and authorization for
      lookup/read/publish. Prevent poisoning and private-derived data/metadata
      leaks. Implement multitenant service policy only in the cloud repository.
- [ ] Sandbox build scripts/macros/tools/tests, enforce resources and fairness,
      and measure local/remote placement, transfer, backpressure and retry costs.
- [ ] Retain exact structural accounting, incomplete-capture markers, redaction,
      wait causes and critical-path explanations with measured overhead.

**Next:** freeze an edit/invalidation matrix and cache threat model while compiler
owners expose finer facts. **Acceptance:** worker/cache/schedule variations agree
with clean results, untracked inputs prevent unsafe reuse, crashes never publish
partial success, adversarial cross-tenant/forged-artifact tests fail closed, and
shared physical work is counted once. In-memory snapshot memoization alone does
not establish persistent, distributed or authorized cross-tenant reuse.

### G12. Graph-native optimizer and code generation

Decided 2026-10-18: a from-scratch optimizer embedded in the graph engine, no
LLVM/Cranelift. One adaptive, incremental pipeline with no user-facing levels:
the profile chooses behavior (overflow checks, debug assertions, panic strategy),
never speed. Design in `OPTIMIZATION.md`. Nothing implemented.

Phase 1 (now, alongside language work):
- [x] `OptMode` off|adaptive|final as an explicit compile input
      (`SourceOptions::opt`), with `--opt` and `JR_OPT` handled only in the CLI.
      `off` demands no optimizer node. `adaptive` currently equals `final`: the
      adaptive background scheduler is not built yet.
- [ ] Preserve borrow/alias, validity/range/niche and drop facts in lowered
      instances (coordinate with G2/G3).
- [x] SSA core as a pure library (`src/compiler/ssa/`, no graph state): block-
      parameter IR, verifier (dominance, types, order-independent typing of
      unreachable code), stable versioned content hash invariant to numbering
      and dead blocks, fuel-bounded reference interpreter, and `opt::local`
      (folding, identities, branch folding, block merging, panic-preserving
      DCE). Tests: eval vs native Rust on all widths, exhaustive 8-bit
      `x op c` soundness, 1,500 random functions differential vs interpreter,
      fixpoint and numbering-independence. A planted unsound identity was
      caught by the exhaustive test. Integer/bool subset only.
- [x] Lower sealed concrete `cfg::Function` (ANALYZE output) to SSA
      (`ssa::lower`) for the scalar subset: integers up to 64 bits, bool, unit,
      whole-local places, arithmetic, comparisons, casts, branches, direct
      scalar calls, explicit panics. Everything else returns Unsupported and
      keeps the stack emitter for that function.
- [x] Per-instance graph nodes `SSA` (130), `LOCAL_OPT` (131), `SUMMARY` (132),
      `IPO_OPT` (133) as typed facts. OBJECT reads IPO_OPT unless the mode is
      off. CHECK never demands them. No whole-program optimizer node.
- [x] Differential acceptance (`tests/optimizer.rs`): real programs compiled
      off and on must match in exit status, stdout and stderr, including
      overflow, division, shift and explicit-panic aborts. Workers 1 and 8
      give byte-identical executables.
- [ ] Compile time, runtime and size harness versus stock `opt-level=0/3`.
      Only an off-versus-on runtime report exists so far.

Phase 2 (with the real G6 ABI):
- [x] Linear-scan register allocation (`ssa::machine`) on the existing private
      ABI, so optimized and stack-emitted functions link together. Values live
      across calls are spilled, constants are rematerialized, compare-and-branch
      is fused, block parameters use parallel moves.
- [ ] Move to the real G6 ABI. New features lower only to SSA. The stack
      emitter becomes a reference oracle.

Phase 3 (alongside core/std bring-up):
- [x] Function summaries and leaf inlining: IPO_OPT reads only its direct
      callees' SUMMARY facts, then requests the LOCAL_OPT body of each inlined
      callee as a recorded edge. Deterministic size threshold.
- [ ] Early cutoff on unchanged summaries (needs cross-build persistence),
      recursion fixed-point protocol, non-leaf inlining, generic-template
      optimization and instance folding.
- [ ] Incremental linker patching per-function fragments, with profile-guided
      hot/cold code layout under the `build` root.
- [ ] Specialization on request: `SPECIALIZE(g, constant args)` nodes keyed by
      callee body hash plus canonical arguments, with a per-callee limit.
- [ ] Reachability-driven codegen: demand machine code only for functions
      reachable from requested entry points (today BUILD emits every planned
      instance). Unused bodies are still checked.
- [ ] Profile-guided speculative devirtualization with guard and fallback.
      Closed-world per-trait implementor sets are deferred and need user approval.
- [ ] Measure the stock LLVM share of debug/release/cold build time.

Phase 4 (once real crates compile):
- [ ] Final-goal interprocedural, loop, vector and target-tuned passes with
      reproducible output at workers 1/N.
- [ ] Adaptive iteration scheduler (volatility, hotness, budget) that never
      delays foreground work. Complete backtraces including inlined frames.

**Next:** the `JR_OPT` switch, fact preservation and the SSA core in new files
under `src/compiler/ssa/`. **Acceptance:** differential conformance per mode,
behavior settings preserved per profile, deterministic final output, edits dirty
only recorded dependents, and compile/runtime/size receipts versus `off` and stock.
