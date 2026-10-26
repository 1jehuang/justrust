# Full Rust implementation tracker

The requested outcome is full Rust support, not completion of a sequence of toy
subsets. This checklist stays open until compatibility evidence supports closing
it. `RUST-COMPATIBILITY.md` defines the contract and dependency ordering.

## Completed foundations, not full compatibility

- [x] Reference-input call replay uses one immutable constraint trace across
  multiple generic slots and authored parameter schemas. Recursive source evidence
  supports shared/mutable references, tuples, arrays/repeats, and record/tuple/enum
  nominal inputs while preserving opaque-parameter moves versus explicit-reference
  reborrows. `callable_reference_inputs` covers stock CHECK/BUILD/run at workers 1/8
  and ownership rejection, with frontend source-replay forgery controls. Independently
  known deferred assignment types reuse no-fallback inference. Direct aggregate
  operands, general scoped/producer evidence and returned-reference inference remain
  separate boundaries; see `GENERIC-INFERENCE.md`.

- [ ] Complete all twenty unchanged full programs. The current
  `justrust test --test full_programs -- --nocapture` gate checks/builds every
  fixture at workers 1 and 8 and runs successful outputs against stored stock
  stdout: 16/20 currently pass. Remaining boundaries are recursive nominal
  layout (12), generic mutable-slice `split_at_mut` (16), nominal source
  constraints (17), and owned trait objects (20). See `FULL-PROGRAMS.md`.

- [x] Source `Option` and `Result` lazy combinators preserve branch selection,
  error types and FnOnce capture cleanup. `option_map` and `result_map` lazy
  combinator tests compare CHECK/BUILD/run with stock at workers 1 and 8.
  `method_owner_inference` covers enclosing-owner bindings inside nominal
  callable outputs. `stored_closures::owned_closure_captures_drop_exactly_once`
  and the normal-drop forgery test cover capture ownership and glue identity.
  `Option::is_some_and` and `is_none_or` use the same source-level FnOnce path;
  `option_map::option_predicates_match_stock` covers skipped calls, payload and
  capture destruction, invalid callbacks and use-after-move rejection.
  `option_map::option_filter_matches_stock` covers `Option::filter` shared
  predicate borrowing, retained/rejected payload drops and skipped capture cleanup.
  `option_map::option_reference_values_match_stock` covers `copied` and `cloned`
  on shared/mutable reference Options, exact clone/drop behavior, and missing
  Copy/Clone bound rejection at workers 1 and 8.

- [x] Nested closure templates read their outermost source callable's immutable
  publication while retaining immediate lexical owner/site authentication.
  `inline_closure_arguments::nested_scalar_closures_use_outer_source_publication`,
  the nested publication forgery test and `stored_closures` graph-edge checks
  cover execution at workers 1 and 8 and source-authority rejection. Nested
  `Result` collection still hits the original typed-cell-cover boundary.

- [x] `Iterator::nth`, `last`, and `find` preserve iterator position and
  non-fused stopping behavior. `iterator_fold::iterator_consumers_match_stock`
  checks stock/native execution at workers 1 and 8 and invalid predicates.
- [x] `Result<Vec<T>, E>` collection stops at the first error and preserves
  payload, iterator and partial-vector drop order. The `result_map::result_vec_*`
  tests cover success, empty input, errors, non-Copy values and invalid types.
  Unchanged full-program 15 is now a passing corpus requirement;
  `result_map::unchanged_error_handling_program_matches_stock` verifies its
  CHECK/BUILD/run behavior against stock at workers 1 and 8.

- [x] `Option::insert` returns the stored mutable reference and drops the old
  value exactly once. `option_map::option_insert_returns_stored_mutable_value`
  checks stock/native execution at workers 1 and 8, mutation through the result,
  replacement drop order, alias exclusion and local-reference escape rejection.

- [x] Ordinary one-parameter generic functions can consume inferred scalar
  references with reference-free results, using original root-binding evidence
  and replayed input constraints. `callable_reference_inputs` covers stock
  CHECK/BUILD/run at workers 1 and 8 and borrow rejection controls; the private
  scalar-input replay test rejects source, place, type and capability mutations.
  Closed string/bool/char literals, nested named structs and shared borrowed
  temporaries now replay source-only constructor constraints for one-input
  generic calls with reference-free results. Literal values, field order and
  original temporary storage identities are checked independently of checked IR.
  `callable_reference_inputs::{closed_literal_generic_inputs_match_stock,unchanged_generic_container_matches_stock}`
  cover stock CHECK/BUILD/run at workers 1 and 8; the literal forgery test covers
  changed payloads, fields, borrow kinds, projections and redirected storage.
  Other nested-reference nominal expressions remain a source-inference boundary.

- [x] Source `String` implements `PartialOrd` and `Ord` with bytewise lexical
      ordering. `operator_inference::owned_string_ordering_matches_stock`
      covers empty/equal/prefix strings, embedded NULs, and multibyte UTF-8
      through operators and ordering methods. Unchanged full-program 14 now
      passes and is ratcheted in the corpus manifest after literal-call replay.
- [x] Open nominal iterator input projections normalize through the selected
      implementation under the caller's bounds, including closure argument
      tuple checks. Rigid parameter projections remain symbolic. Coverage:
      `iterator_associated::generic_nominal_iterator_closure_inputs` and
      `open_nominal_normalization_retains_caller_authority`, including wrong
      input types and forged result/environment/phase rejection. Root
      normalization now proves membership once through its selected assignment,
      while independently authenticating input metadata and nested projections.
      `generic_iterator_format_collect_reuses_membership_proofs` covers generic
      formatting/collect execution without increasing resource limits.
- [x] Numeric-only nominal arguments permit the quiescent defaulting round
      without masquerading as missing graph inputs. Integer and float generic
      comparisons, later narrowing constraints, and invalid comparisons are
      covered by `operator_inference::nominal_comparisons_allow_quiescent_numeric_defaulting`.
- [x] Inherent method inference consumes callable `Output` equalities after
      binding the receiver's implementation parameters. `Result::map`,
      `Result::map_err` and custom callable methods infer their result parameters
      without a result annotation; final method selection still checks the full
      signature and obligations. Stock/native CHECK/BUILD/run at workers 1/8
      and invalid unused-body controls pass. All 11 closure tests, 1,824 library
      tests and 17 graph checks pass, with one ignored test in each latter suite;
      clippy exits successfully. Receipts are retained in
      `data/runs/xps14/inherent-output-inference-2026-10-23.jsonl`.
      Unchanged full-program 15 now reaches a slice-family source-channel
      rejection instead of the unresolved `map_err` inference error; it remains
      unsupported and no full-program completion is claimed.

- [x] Declared generic shared-slice element families retain source lifetime
      authority through substitution, trait headers and borrowed element returns.
      Exact input/path/family checks reject forged mappings, escapes and live-loan
      violations. Independent mutable-slice pairs may additionally discard only
      families proven empty for their concrete element actuals; this does not
      authorize splitting one input. All 32 supported typed-slice authority tests,
      two generic-family tests, seven parameter-equality tests and 17 graph checks
      pass (`20261023-120923162-2075321`, `20261023-121020069-2078619`,
      `20261023-121031830-2079535`, `20261023-121040417-2080239`). See
      `TYPED-SLICES.md`. Nested-reference associated-type equality and unchanged
      full-program 15 remain open, rather than accepted through relaxed guards.
      The exact isolated library run passed 1,818 tests with one executable-busy
      infrastructure failure; that test passed on retry. All 21 slice/corpus
      integration tests and library/test clippy then passed. Receipts, including
      the failed launch, are retained in
      `data/runs/xps14/declared-slice-families-2026-10-23.jsonl`.

- [x] Unknown closure parameters defer body coercions until callable context
      supplies their shape; standalone closures retain body-driven inference at
      genuine graph quiescence. Missing producers never authorize this fallback.
      Stock/native CHECK/BUILD/run at workers 1/8 covers contextual shared-reference
      autoderef, standalone parameters and rejection in unused invalid bodies
      (`20261023-114842690-2005176`). All 14 neighboring closure/result tests and
      17 graph checks pass (`20261023-114913941-2006684`,
      `20261023-115024428-2005173`). This removes one full-program-15 inference
      blocker, not its remaining projected-reference and lifetime-proof gaps.

- [x] Associated-type normal forms retain their canonical executable nominal
      identity through prepared graph reads, including nested tuples. Missing
      graph inputs remain pending and forged or unsealed representations reject.
      All 40 unchanged associated-type sources now pass the 160-cell stock/native
      matrix: 28 valid programs execute at workers 1/8 and 12 invalid programs
      reject in CHECK/BUILD (`20261023-113805836-1968809`). The exact isolated
      change passes 1,816 library tests and 17 graph checks
      (`20261023-114356101-1988541`, `20261023-114727828-1988540`), plus all eight
      nominal-manifest tests (`20261023-114312064-1986015`). This closes the
      projected Copy-record identity gap, not full associated-type or Rust support.

- [x] A flat pair of mutable slices can retain two independently borrowed
      input origins through a function result. The bounded affine proof requires
      two distinct fixed input roots with exact result-field mappings and no
      write effects. It does not infer a disjoint partition from one input.
      Concrete and generic carrier-free element cases match stock CHECK/BUILD/run
      at workers 1/8; overlapping inputs, duplicated handles, wrong lifetime
      mappings and escaping results reject. The exact isolated source passes
      all 18 slice-method tests, 1,814 library tests and 17 graph checks
      (`20261023-113035449-1943515`, `20261023-113352768-1953841`,
      `20261023-113713166-1943514`). Library/test clippy also passes
      (`20261023-114158554-1981406`), with pre-existing warnings and unrelated
      global formatting debt left intact. This does not implement `split_at_mut`,
      mutable reference-containing element families or full Rust compatibility.
      Additional declared shared element-family transport is covered above.

- [x] Node-local authority replay avoids rebuilding parameter-name bindings on
      nominal cache hits, repeating same-module ancestry lookups, and discarding
      call/proof scratch within a source body. Successful closure and associated
      normalization checks retain exact immutable allocation identity, caller
      environment, phase and staging; forged evidence and changed contexts still
      reject. The isolated library/graph suite passes 1,821 tests
      (`20261023-084646034-1382688`), with targeted nominal and source-replay
      rejection checks and the final root graph gate passing
      (`20261023-090042025-1432621`). Library/test clippy succeeds with existing
      warnings (`20261023-090235525-1437867`). This is not an iterator-program
      completion or a measured speedup: unchanged fixture 11 still reaches its
      authority resource boundary (`20261023-085826114-1425185`), and the full
      compatibility program remains open. No graph or resource ceiling changed.
      Follow-up reuse retains independent closedness flags, subtree height and
      exact root/environment identity, never memoizing nominal backedges.
      Equality-domain and call-evidence checks retain their existing local
      proof scratch. The isolated follow-up passes 1,807 library tests and all
      17 graph gates (`20261023-092829346-1524370`,
      `20261023-093225000-1523509`). Standalone collection and closure-authority
      tests pass, but broader integration attempts time out, including the
      associated-equality matrix's 240-second bound. Those gates are unresolved,
      and unchanged fixture 11 still rejects at workers 1 and 8.
      Raw associated-type validation now also retains the current proof's
      nominal metadata replay scratch throughout recursive validation. The
      regression verifies reduced repeated validation work, stable retained
      entries, and rejection of a forged associated owner and excessive depth.
      The exact isolated library passes 1,813 tests
      (`20261023-110447737-1852049`); both isolated and active checkout graph
      gates pass all 17 checks (`20261023-110452213-1852644`). This preserves all raw projection
      admission checks and does not solve fixture 11's work-limit rejection.
      The related iterator/authority run passes 43 tests; two historical
      associated-type exclusions fail because their sources are now accepted.
      Both failures reproduce with this change removed
      (`20261023-111105524-1871854`), so their expectations are left unchanged
      here rather than counted as passing gates. Library/test clippy passes
      with existing warnings (`20261023-111142490-1874078`).

- [x] `str::is_empty` is supplied by the source prelude using the existing byte
      length operation. Empty, Unicode, embedded-NUL and borrowed-String inputs,
      plus a nominal-error decimal parser using `ok_or` and `?`, match stock
      CHECK/BUILD/run at workers 1/8; wrong arity, type arguments and return types
      in unused functions reject (`20261023-083338029-1330664`). The six-target
      string/primitive/Result/graph suite passes 30 tests
      (`20261023-083206616-1324130`), and the root graph gate and scoped clippy
      pass. The unchanged error-handling program now advances past `parse_u32`
      to unresolved source constraints in `main`. The full corpus stays 11/20
      (`20261023-083331529-1328667`), not complete Rust compatibility.

- [x] Borrowed callable forwarding retains exact source provenance, receiver
      reference chains, and physical carrier/lifetime boundaries. The five
      standard `Fn`/`FnMut`/`FnOnce` reference implementations support shared
      opaque callables, nested shared references, and mutable stateful closure
      forwarding without moving the referent. `&mut F: Fn` is not invented:
      stock-invalid shared mutation/consumption and mutable-reference `Fn`
      calls reject. All nine owner/forwarding cases pass stock CHECK/BUILD/run
      at workers 1/8. Deferred generic write envelopes are filtered against
      concrete writable destinations; an additional closed-body proof admits
      only exact identity transport of distinct fixed cells, not arbitrary
      multi-cell mutation. Forged source bindings, reborrow timing, receiver
      keys, carrier rows and physical boundaries reject. The exact owned
      snapshot passes 1,963 library and integration tests with zero failures
      and three existing ignores (`20261023-081622283-1266672`), including
      stored generic callable references after canonical bound selection through
      autoderef. Library/test clippy passes with existing warnings
      (`20261023-081617008-1266673`); hosted check also passes
      (`20261023-080205036-1221650`). The final root graph-integrity gate passes
      (`20261023-082018931-1279668`). Hidden reference-bearing opaque families
      and nonidentity declared-lifetime transport remain unsupported. The
      unchanged full-program corpus remains 11/20, not full Rust support.
      The broader replay also exposed three pre-existing test expectation
      failures in `borrowing_graph` and `associated_types`; all three reproduce
      before this increment (`20261023-080948247-1246200`). Their graph-count
      and formerly unsupported parse/CHECK expectations are not changed here.

- [x] Mutable reference arguments reached through local-rooted dereference
      chains are reborrowed rather than moved. Typed carrier witnesses and CFG
      two-phase loans retain every dereference, and every crossed reference
      must be mutable. Repeated calls through `&mut &mut i32` run as stock does;
      mutation through `&&mut i32` is rejected even in an unused function.
      Stock CHECK/BUILD/run at workers 1/8, forged cyclic carrier and shortened
      place rejection, and all 35 reference call/graph tests pass on the isolated
      committed baseline (`20261023-073946397-1146951`,
      `20261023-074029277-1149326`). Graph integrity passes 17 tests with one
      existing ignore (`20261023-074102975-1146943`). The isolated full library
      passes 1,792 tests with one existing ignore (`20261023-074130548-1152705`).
      This does not complete borrowed opaque callable forwarding or full Rust
      compatibility.

- [x] Nested owned opaque callable composition supports concrete and generic
      argument/result types and deeper composed owners. Closure-bearing opaque
      keys replay canonical graph templates. Physical closure descriptors are
      authenticated once per consumer against their source seals and all
      ordered actuals, then reused through an immutable local mapping. Physical
      validation similarly retains one independently replayed immutable CFG,
      while checking every candidate and call against its exact authority.
      Stock CHECK/BUILD/run at workers 1/8 passes all three composition cases
      (`20261023-064617783-963157`); unused wrong Output and insufficient public
      bounds reject. Copied-seal descriptor, foreign provider and changed
      physical CFG/ABI controls remain rejection gates. No graph kinds,
      resource ceilings or source-type equality rules were relaxed. The exact
      snapshot library and fourteen integration targets pass 1,874 tests with
      zero failures and two ignored (`20261023-064739519-969367`); library/test
      clippy passes with existing warnings (`20261023-065119948-980785`). The
      supplementary corpus, slice and borrow/trait authority suite passes 42
      tests with one ignored (`20261023-065145657-982178`). Borrowed
      opaque callable forwarding and general hidden nominal/reference carriers
      remain incomplete; the unchanged full corpus is still 11/20.

- [x] Owned closure-backed opaque values can cross generic `Fn` call
      boundaries without exposing their hidden type to source CHECK. Public
      callable argument and Output equalities drive local inference, while
      per-occurrence opaque dispatch proofs add exact physical targets to the
      existing INSTANCE frontier. Callers retain public instance identities
      and replay physical ABI/carrier normalization from graph-read facts.
      Scalar captured closures, generic argument/result inference, unused
      wrong Output and insufficient public `FnMut` bounds are covered at
      workers 1/8. Nested owned composition is covered by the follow-up above;
      borrowed callable forwarding remains incomplete.
      The exact-snapshot library, graph, authority, inference, iterator, slice
      and corpus suite passes 1,926 tests with zero failures and three ignored
      (`20261023-062249866-883524`). All-target clippy passes with baseline
      warnings (`20261023-062229413-870425`). A stale rigid-projection identity
      rejection also failed unchanged HEAD (`20261023-061830581-870003`);
      its replacement now runs stock-differential identity and distinct
      projection rejection at workers 1/8 (`20261023-062002505-874656`).
      Full corpus status remains 11/20, not full Rust support.

- [x] Contextual numeric range receivers retain their inferred scalar type
      through graph preparation and ordinary checked method lowering. Signed
      and unsigned collection, nested scopes, literal overflow and unused
      mismatched-result rejection pass stock CHECK/BUILD/run at workers 1/8.
      Candidate inference requires a unique closed associated output and never
      grants executable proof authority. The final hosted library, graph,
      body-inference, iterator, slice and corpus replay passes 1,844 tests with
      zero failures and three ignored (`20261023-054904139-767929`). All-target
      clippy passes with existing warnings. The unchanged iterator program now
      reaches opaque callable equality and generic composition boundaries;
      the full corpus remains 11/20, not full Rust support.

- [x] Contextual result types propagate through unique, unspecialized nominal
      trait implementations as local inference equations, never executable
      proof authority. Nested chunk `map`/`sum` and `map`/`product` collection
      infer their result types without turbofish annotations; a custom generic
      `FromIterator` implementation follows the same path. Stock CHECK/BUILD/run
      at workers 1/8 and unused wrong-result rejection pass. The full library,
      graph, body-inference, iterator, slice-method and corpus suites pass 1,843
      tests with three ignored; graph ratchets and all-target lints pass.
      A nested generic collection equality-authority case remains open.
      The unchanged corpus is still 11/20.

- [x] Shared chunk iteration and physical aggregate slice descriptors pass
      stock CHECK/BUILD/run acceptance at workers 1 and 8. General heap-stored
      borrowed elements are not complete: collecting chunks into `Vec<&[T]>`
      requires lifetime retention distinct from raw-pointer storage and exact
      projected result authority. Raw storage containing slices is explicitly
      unsupported, including an escaped collection that stock rejects with
      E0597, rather than incorrectly accepted. Positive heap support remains
      tracked by `slice_methods::collected_slice_references_require_heap_lifetime_authority`.
      These gaps may not be hidden by weakening issued provenance checks or
      reported as full Rust support. The final hosted
      replay passes 1,874 tests across the full library and seven integration
      suites (slice methods, typed slices, typed-slice authority, slice returns,
      graph integrity, full programs and iterator fold), with zero failures and
      three explicit ignored gates. `clippy --all-targets` passes with existing
      compiler warnings. The unchanged full-program corpus remains 11/20.

- [x] Source `Iterator::zip` supports existing and custom `IntoIterator` inputs,
      unequal lengths, conversion at construction, and non-fused polling order.
      Projected generic method constraints now wait for their input slots before
      normalization rather than treating an open projection as a rigid result.
      Stock CHECK/BUILD/run at workers 1/8 covers values, wrong projected item
      rejection and borrowed-owner escape. All 1785 library and 48 integration
      tests pass, including graph integrity. The general `Iterator` to
      `IntoIterator` blanket implementation still needs coherence support;
      concrete source conversions remain explicit. Zip reverse/exact-size
      implementations are also open.

- [x] Source `skip`, `take` and `step_by` preserve lazy consumption, non-fused
      iterator behavior and owner loans. `iterator_fold` compares chained values,
      zero limits, maximal steps and borrow escape rejection against stock at
      workers 1/8, plus matching runtime panic behavior for a zero step.
      All 41 graph, iterator, slice and full-program integration tests pass.
      Reverse/exact-size adapter implementations and other adapters remain open.

- [x] Shared slice iterators implement `DoubleEndedIterator` through ordinary
      source. `slice_methods::double_ended_slice_iteration_matches_stock`
      checks mixed front/back consumption, reversed collection, empty and
      zero-sized slices, singleton exhaustion, owner mutation and escape
      rejection at workers 1/8. Existing iterator, corpus and graph gates pass.
      Mutable slice iteration and remaining iterator adapters are still open.

- [x] Source `Iterator::product` dispatches through `Product`, with owned and
      borrowed implementations for all twelve integer types and custom result
      types. `iterator_fold` exercises empty identity, zero factors, mapped
      inputs and a generic custom implementation using `fold` against stock
      CHECK/BUILD/run at workers 1/8. Generic required-method carrier evidence
      retains ordered method arguments; caller projection equalities and closure
      body substitutions are replayed from their original authority. Forged
      method arguments are rejected by the projected-carrier unit tests.
      Floating-point and other standard Product implementations remain open;
      the full-program corpus remains 11/20.

- [x] Source `RangeInclusive` iteration and double-ended integer ranges cover
      all twelve integer types, including MIN/MAX endpoints, mixed front/back
      consumption, exhausted and reversed-empty ranges, and collection.
      Stock-differential CHECK/BUILD/run at workers 1/8 passes
      (`20261023-033622145-326677`); graph, iterator and full-program regression
      gates pass (`20261023-033958180-338483`). The corpus remains 11/20.

- [x] The unchanged string-processing program passes native CHECK/BUILD/run
      at workers 1 and 8, raising the full-program manifest to 11/20. Source
      implementations cover character reversal, splitting, containment, Unicode
      uppercase and heterogeneous `String`/`&str` equality. Uppercase data is
      generated from the acceptance toolchain's Unicode 18 tables and checked
      against every changed scalar. This is a scoped library milestone, not
      complete Unicode or standard-library compatibility. Nested inferred
      adapters retain node-local owner and projection proofs without increasing
      resource limits. The staged snapshot passes 1,784 library tests, the
      11/20 corpus ratchet, and graph, Unicode and heterogeneous-comparison
      gates (`20261023-031503215-265249`, `20261023-031756111-273302`).
      Slice/range iteration and nested collection pass differential checks at
      workers 1 and 8 (`20261023-031340786-261093`), including empty and zero-sized
      slices without constructing an uninitialized element reference.
      Post-commit replay of `9a02506f1` confirms 1,784 library tests and all
      41 tests across the six integration suites pass together
      (`20261023-032048200-282537`, `20261023-032338997-282531`):
      `iterator_fold` checks unchanged matrix/string output, nested inference,
      reverse iteration, collection and generic dispatch against stock at
      workers 1 and 8; `slice_methods` checks array/range/slice integration,
      empty/ZST iteration, owner escape and conflicting-mutation rejection;
      `string_scalars` checks Unicode mappings, UTF-8 and helper privacy;
      `comparison_source` checks heterogeneous equality and reference contracts;
      `full_programs` retains 11 successful programs and nine explicit rejections;
      `graph_integrity` retains all 17 passing structural gates without raising
      any ceiling. The two previously ignored tests remain ignored.

- [x] Source-level `Iterator::sum` dispatches through `Sum`, with owned and
      borrowed implementations for all integer widths and user-defined result
      implementations. Explicit trait arguments retain the validated caller's
      source-projection context, and transplanted occurrences remain rejected.
      The unchanged matrix program now passes, raising the full-program corpus
      to 10/20. Floating-point and other standard sum implementations remain open.

- [x] Body inference rejects producer-free unresolved nominal bindings with
      E0282. Owned rvalue method autoref uses full-expression temporary storage
      and its checked receiver type, preserving destructor order and rejecting
      escaping borrows. All eight `body_inference` tests pass against stock at
      workers 1/8, including nested generic returns and mutable temporaries.
      General temporary promotion and lifetime compatibility remain open.

- [x] Native `Iterator::collect` and `FromIterator` source implementations for
      `Vec<A>` and character/borrowed-character `String` collection, including
      empty, custom and mapped iterators at workers 1/8. Generic result inference
      honors contextual types and exact caller projection equalities. Local
      proof reuse preserves canonical receipt and normalization checks without
      raising resource limits (`PROJECTED-METHODS.md`). The unchanged corpus
      reached 9/20 at this milestone. The subsequent integer `Sum` integration
      admits matrix (10/20); additional library operations and recursive layouts
      remain open.

- [x] Generic trait-method inference substitutes exact owner projections before
      constraining arguments and expected results. Closed projected signatures
      validate raw owner/method substitution before normalization, and required
      signatures use only the exact implementation's assignments for conformance
      (`PROJECTED-METHODS.md`). This repairs the iterator `find` acceptance cases.
      Local projection annotations retain source identity and normalize only after
      graph-backed receiver/argument realization. The frozen projected-method
      corpus now has 17 executable cases, five invalid cases and two explicit
      reference-related boundaries, with workers 1/8 CHECK/BUILD/run coverage.
      The full library rerun passes 1,781 tests with one ignored. The unchanged
      full-program corpus remains 9/20, with collection, recursive layouts and
      missing library APIs still blocking programs. No full-Rust gate is closed.

**Audit follow-up in progress (2026-10-20):** `OVERFLOW-PROFILES.md` defines
explicit runtime overflow policy across stack and SSA lowering, independent of
assertions and optimization. `NOMINAL-RECIPE-SELECTORS.md` separates Index/IndexMut
obligations from nominal field recipes, removing internal metadata failures without
claiming that the three affected full programs now compile. Final acceptance and
graph-integrity evidence are recorded in the feature contracts. G0-G12 remain open.

- [x] Nominal by-value cycle checks memoize completed immutable subtrees and
      retain subtree heights, preserving recursion and depth rejection without
      exponential shared-DAG expansion. Source carrier collection only reuses
      completed, nongeneric, region-free, carrier-free subtrees at the same depth.
      The unchanged 40-level binary nominal fixture, negative depth/back-edge
      controls and all 14 graph-integrity gates pass. The earlier full-library
      run was interrupted after its exponential traversal stalled, not counted
      as a pass. Evidence is retained in
      `data/runs/rust-support-followup-20261020/bounded-nominal-dag.jsonl`.
      The full-library rerun now terminates. The first completed replay reported
      53 failures (`20261020-062040239-4112197`). After the exact iterator/nominal
      identity and constructor-demand repairs, a frozen replay reports 1,737
      passing tests, 20 failures and one ignored test
      (`20261020-064450881-4149572`). Remaining failures include lexical adapter
      contracts, local-call diagnostics, projection/carrier assertions and one
      invalid self-glob import acceptance. It is not a green compatibility gate.
      The unchanged full-program corpus remains 6/20
      (`20261020-064454678-4149689`). Full-library revalidation and G0-G12 remain open.

- [x] Const declarations reject non-const calls and possibly initialized generic
      destructors even when unused, using sealed callable identities and ordinary
      ownership analysis. Full const/core identity and scalar-source suites pass
      (26 and 8 tests). Approximate lazy fixed-address mutable-static lowering is
      withdrawn and explicitly Unsupported pending real storage and constant
      initialization semantics (`CONST-EVAL-SCALAR.md`). This closes correctness
      holes, not general const or static support.

- [x] Generic opaque return inference preserves defining owner, return site,
      ordered arguments and public bounds. Physical direct/bound calls replay
      their exact original occurrence before canonical specialization. Forty
      focused tests and unchanged graph-integrity gates pass
      (`OPAQUE-RETURNS.md`); closure-backed, carrier-bearing and general opaque
      runtime support remain open.

- [x] Empty braced record declarations retain exact nominal constructor shape,
      immutable source-field authority and normal ownership. The pinned stock
      extension covers 21 sources / 84 native CHECK/BUILD cells, including unused
      generic/lifetime rejection and explicit empty C ABI boundaries
      (`UNIT-STRUCTS.md`). Public zero-sized layout and G0-G12 remain open.

- [x] Connect body-local primitive method inference to canonical exact signatures,
      including typed-call `.leading_zeros()` results. Fixed scalar receivers,
      ordinary nominal/trait method precedence and exact invalid-input rejection
      are retained. Ten stock/native sources pass CHECK/BUILD at workers 1/8
      (`LEADING-ZEROS.md`, `PRIMITIVE-METHODS.md`); generic/reference receiver
      expansion and full Rust compatibility remain open.

- [x] Repair the default-stack CFG unary/cast recursion regression without
      raising stack size or lowering resource limits. Separate unary materialization
      keeps recursive operands out of the large aggregate/control frame. The original
      cast/cycle test, a new 120-unary chain, all 103 CFG tests and all 14 graph-integrity
      gates pass on hosted (`INTEGER-CASTS.md`). This bounded G2/G9/G10 robustness
      repair does not close any full-Rust gate.

- [x] Keep validated closed-call nominal conversion inventories occurrence-local.
      Enclosing body substitutions remain single-valued and reject conflicting
      targets. The unchanged format-macro fixture now matches live stock
      CHECK/BUILD/run at workers 1/8, with 14 graph-integrity gates passing
      (`20261020-031050478-3603331`). This compatibility fix closes no full Rust gate.

- [x] Packed-byte query operation authority binds genuine loader-issued function
      syntax, exact original query occurrence and canonical callee through a
      default-deny borrowed schema service (`PACKED-QUERY-AUTHORITY.md`). It is a
      query-admission prerequisite only: allocation/runtime memory origins,
      String/Vec activation and G0-G12 completion remain open. Exact-parent519a
      enum alignment CHECK acceptance is documented as an existing limitation,
      not a green alignment suite or BUILD-support claim. Final c0-parent
      check--tests and immutable-facts scanner passed on treef794d06b
      (20261020-020020285-3274461, 20261020-020112811-3287086); other gates
      retain the exact split-tree provenance documented in the feature contract.

- [x] Native array `.len()` uses ordinary shared borrowing/reborrowing, checked array-to-slice unsizing and canonical slice-length source authority, preserving non-Copy local storage and receiver effects (`ARRAY-LEN.md`). G0-G12 remain partial; destructor-bearing temporary receivers remain explicitly unsupported.

- [x] Indexed native format arguments retain checked shared place borrows, evaluate source indices once and reject conflicting later mutations (`INDEXED-FORMAT.md`). G0-G12 remain partial.
- [x] Mutable slice parameters and local carriers with same-kind array unsizing, generic reference-free elements, shared read reborrows and Copy-element checked writes (`MUTABLE-SLICES.md`). G0-G12 remain partial: non-Copy replacement remains open.
- [x] Mutable range borrows and shared views use ordinary local-rooted reborrow authority and conservatively retain the whole source loan, with stock range-order/trap, lifetime and canonical-source tests (`MUTABLE-RANGES.md`). G0-G12 remain partial.
- [x] Canonical unsafe raw slice constructors support generic shared/mutable descriptor transport and non-Copy elements, with native stock fixtures, unsafe/forged-origin rejection and unchanged graph-integrity ceilings (`RAW-SLICES.md`). G0-G12 remain partial.
- [x] Shared read-only slices admit one shared-reference element layer, including `&[&str]`, with typed all-element carrier paths, independent inner lifetimes, checked loads and provenance-preserving argument dereference (`TYPED-SLICES.md`). Stock/native CHECK/BUILD/run at workers 1/8, forged facts and unchanged graph-integrity ceilings pass. Mutable/static-annotated and deeper reference element cases remain explicit boundaries. This is only a prerequisite for program 15: the unchanged full-program corpus remains 6/20 and `15_error_handling.rs` now stops at unresolved nominal source constraints, not a completed error-handling implementation.
- [x] Root shared and mutable slice returns use caller-owned descriptor storage and preserve source lifetime/alias authority, including generic non-Copy element transport (`SLICE-RETURNS.md`). G0-G12 remain partial: nested slice carriers remain open.

- [x] Independent graph runtime with shared immutable facts and lifecycle telemetry.
- [x] Direct native Linux x86-64 executable generation for the initial subset.
- [x] Inline modules, qualified calls, simple imports and namespace/privacy checks.
- [x] External module source trees with bounded loading and observed source facts.
- [x] Per-file SOURCE graph nodes (child-ordinal keys), assembled in order by PARSE.
      Source producers now use typed file-ancestry keys and discover a bounded
      child frontier independently of the assembly consumer. Ordered diagnostics
      and later/deeper admission are pinned by `source_error_order` tests.
      This is discovery scheduling only: it does not remove the PARSE barrier.
- [ ] Per-file parse, per-module declaration index, per-item resolve and
      signatures (PARSE/RESOLVE/SIGNATURES remain whole-program; parsing shares
      one NameStore/ModuleArena, and resolve memoizes imports/aliases program-wide).
      SIGNATURES split progress (2026-10-22): source `DeclId`s are canonical
      (`SOURCE_BIT | module-path key | kind | per-module ordinal`, 519041cad),
      so any node can derive a declaration's owning module from its identity;
      registry and schema storage are identity-keyed, not positional
      (2285fbe2f, bec557ef2, 0c0a82c85, 4f44c29f1). Step a (392d04626):
      `frontend::module_declarations` derives one module's canonical ids
      from RESOLVE's Program alone (test proves equality with the
      registry); not yet a graph node (hog: MODULE_INDEX kind 122 reserved,
      PARSE_FILE kind 121 keyed by file ordinal). Blocker for deleting
      SIGNATURES: PlanCtx (registry path maps, ValidatedTraitWorld,
      SharedTypeEnv, contexts) is whole-program state behind ~227 call
      sites; each `facts.types()` arm needs per-item replacements for
      those maps first. Next: per-module
      declaration fact keyed by module-path key, TRAIT_IMPLS from it, move
      `facts.types()` consumers kind by kind, then delete SIGNATURES.
      Module-work split (2026-10-25): RESOLVE publishes its exact immutable
      parsed snapshot/namespace prerequisite before `RESOLVE_MODULE` (148)
      resolves ordinary free-function bodies per module. SIGNATURES publishes
      finalized schema inputs before `SIGNATURE_MODULE` (149) computes source
      callable schemas per module. Outputs bind their exact input snapshot;
      source errors are selected by original callable order, not scheduling.
      Namespace/header preparation and final assembly remain explicit root debt.
      Gates: `semantic_resolution::resolver_module_facts_read_only_exact_namespace_snapshot`,
      `nominal_graph` module-signature tests and `graph_integrity`.
      Closed-source native instances reuse SOURCE_ADMISSION's exact CFG and
      successful ownership proof through typed outputs, with canonical
      publication, target, trait-world, const-context and CFG identity checks.
      Generic instances still require substituted analysis and CHECK never
      demands executable lowering. Gates: `compiler` graph acceptance,
      `owner_route_tests::closed_source_proof_reuse_requires_exact_publication_cfg_and_context`
      and `root_cleanup_graph`.
      PARSE split activation (2026-10-25): default-prelude, no-prelude,
      library and extern inputs now demand typed `PARSE_FILE(SourceFile)` nodes
      using the existing file-view, child-mount and crate-merge helpers. Each
      file reads its own SOURCE, an immutable admission/render-id manifest and
      its immediate parent's syntax (attachment roots read the input root).
      PARSE still performs all-file admission and merge, and children wait for
      the complete parent. Syntax/unsupported failures may use the original
      assembled parse to preserve diagnostic order; resource, contract and
      infrastructure failures fail closed without retry. Test harness and probe
      transport remain on the assembled path. This is not PARSE-root removal.
      Coverage: `source_error_order` pins exact file dependencies, stock/native
      CHECK/BUILD/run at workers 1/8 and child-before-late-parent diagnostics;
      `compiler::owner_route_tests::parse_file_rejects_unadmitted_identity`
      rejects forged file keys. Historical resume notes below predate activation.
      PARSE split resume notes (2026-10-22, after steps 1a-1h, A, 7341a8948;
      steps B-D not started). Keys: `PARSE_FILE` kind 121 keyed by SOURCE
      ordinal words; `MODULE_TREE` (new kind, keyed `[0]` = crate root
      ordinal) merges PARSE_FILEs depth-first; RESOLVE reads MODULE_TREE.
      Blocker found for step B: the child site needs the parent's names,
      modules and scope arena as frozen `Arc`s (`link_crate_files` takes
      frozen arenas), but `parse_syntax_unit` (frontend.rs) only freezes
      them after the whole file. Needed first: a checkpoint API on
      `NameStoreBuilder`, `ModuleArenaBuilder` and `ScopeArenaBuilder` that
      publishes an immutable prefix (records `0..len` behind an `Arc`
      receipt, later records appended to a fresh tail) so the parent can
      publish output `1 + i` at `mod child;` without finishing. Simpler
      alternative: two-pass, the parent PARSE_FILE publishes everything at
      its end and children wait on it (sequential along the ancestor chain
      only, siblings and unrelated subtrees still parallel); this loses
      early child start but keeps the graph per file. Parser entry points
      to change: `Parser::with_purpose` (take parent ModuleId/frontier and
      joined builders), `items()` at the `mod` branch (frontend.rs ~10999:
      on an empty group at a recorded `FileView::child_sites` offset, record
      the child ModuleId and `module_frontier` instead of recursing), and
      closure serials (`closures.rs:134`, `closures/stored.rs:69`) must be
      prefixed with the file ordinal (`$closure${file}_{serial}`;
      `tests/closures.rs:440` and `sema/adapter.rs:715` match on prefix).
      Consumers of PARSE output 1 (`PREPARED_SOURCE_OUTPUT`, mod.rs ~2800
      source accounting) must move to SOURCE facts summed by MODULE_TREE.
- [x] Scalar checked-function IR shared directly by checking and native emission.
- [x] Unlabelled loop values, break/continue and exact native temporary cleanup.
- [x] Separate semantic Never typing from completion, including typed dead tails.
- [x] Nested Copy tuples, field projections/mutation and by-value aggregate calls.
- [x] Explicit cached CFG facts shared by check and native emission, with provenance.
- [x] Cached storage/initialization/move-path dataflow shared by check and emission.
- [x] Nonempty nominal structs, field privacy and source-level whole/partial moves.
- [x] Admit nongeneric unit-struct constructors/patterns and native nominal
      transport with an explicit initialized private carrier. UNIT-STRUCTS.md
      records 26 stock sources / 104 native cells in debug and release bootstrap
      profiles, five schema authority controls and the complete 2,179-test /
      108-binary hosted gate. Scoped bare constructor patterns and full zero-sized
      public layout/FFI remain separate open work.
- [x] Nongeneric nonempty tuple structs with certified constructors, positional
      privacy, nominal let patterns and existing native aggregate transport.
      TUPLE-STRUCTS.md records stock36/native144 debug/release bootstrap cells,
      eleven public schema controls and complete 2,209-test / 110-binary gates.
      Empty forms, constructor values and numeric brace syntax remain
      explicit unsupported boundaries. Generic tuple coverage is recorded below.
      The later aggregate match milestone
      promotes the exact retained tuple-match source without changing its bytes.
- [x] Generic nonempty tuple structs with exact declaration-qualified nominal
      applications, rigid universal checking, instantiated fields and conditional
      pattern plans. GENERIC-TUPLE-STRUCTS.md records stock32/native128 cells per
      bootstrap profile, 23 public authority controls and complete 2,586-test /
      128-binary hosted acceptance. Nested constructor evidence retains literal
      classes until contextual solving, without new generic function inference.
- [x] Irrefutable nested aggregate match destructuring with explicit checked
      aggregate domains, genuine arm locals, plain Field projections and selective
      moves. AGGREGATE-MATCHES.md records stock26/native104 debug/release cells,
      28 public malformed-IR controls and complete 2,247-test / 112-binary gates.
      Nested refutable leaves are extended by the Decision milestone below.
      Guards, or-patterns and implicit reference ergonomics remain unsupported.
      Positional rest is admitted below.
- [x] Enum tuple-payload positional rest with genuine variant coverage,
  exact-owner Downcast/physical Field plans and single-variant irrefutable lets.
  See `ENUM-REST-PATTERNS.md` for stock24/native96, public authority27 and
  complete 2,348-test / 118-binary acceptance. Refutable payload matrices remain open.
- [x] Positional rest in nonempty structural tuples and nongeneric tuple structs,
  with honest prefix/suffix source nodes, checked selected-field projections,
  constructor privacy and partial moves. See `POSITIONAL-REST-PATTERNS.md`.
- [x] Named record-rest match patterns with selected-member privacy, omitted-field
      nonconsumption and unchanged genuine binding/replay identities.
      RECORD-REST-MATCHES.md records stock27/native108 debug/release cells, twelve
      public selector controls and complete 2,268-test / 114-binary hosted gates.
- [x] Irrefutable tuple/struct let patterns with selective moves and wildcards.
- [x] Genuine if-let conditional patterns with then-only checked binding scopes,
      actual source identity and shared genuine match-IR lowering. IF-LET.md
      records stock27/native108 debug/release cells, twenty public authority
      controls and the complete 2,447-test / 122-binary hosted gate. Universal
      source admission checks live sealed BindingIds before planning; chains
      and refutable payload matrices remain explicit open boundaries.
- [x] Genuine statement-position while-let with body-only checked source bindings,
      actual repeated scrutinee calls, real loop/match IR, success-edge moves and
      backedge reinitialization. WHILE-LET.md records stock33/native132 cells per
      bootstrap profile, 23 public authority controls and the complete 2,495-test /
      124-binary hosted gate. Actual trailing semicolon syntax preserves source
      Unit obligations separately from intrinsic runtime loop divergence.
      Expression-position while-let, chains and refutable payload matrices remain open.
- [x] Range expressions in ordinary expression positions, including call arguments,
      parenthesized receivers and open/full/inclusive bounds. They lower to ordinary
      native `std::ops` range values. `tests/range_exprs.rs` compares stock output.
      Native bounded integer `for` ranges also accept `.rev()` and `.into_iter()`
      chains, preserving bound evaluation order, empty ranges and inclusive MIN.
      RangeInclusive keeps private bounds/exhaustion state and provides standard
      accessors, into_inner and is_empty plus internal fused-iteration hooks.
      General Iterator/adapters remain a separate library workstream.
- [x] Genuine statement-position integer range for loops (`start..end` and
      `start..=end`) through existing checked while/match IR. Bounds evaluate
      once, left to right, before body-only source bindings. Pre-body advancement
      preserves break/continue/labels and never increments an inclusive maximum.
      `tests/loops.rs` compares stock execution and invalid type/scope/control
      cases with native check/build at one and eight workers. By-value Copy-array
      iteration snapshots its iterable once, advances before the body, and supports
      existing irrefutable scalar/tuple/array patterns. Referenced fixed arrays
      (`&array` and `&mut array`) now yield genuine scalar/unit element loans
      through typed runtime Place projections, with one source body and canonical
      whole-array overlap/initialization authority. Shared scalar/unit slice
      iteration uses the same canonical runtime loans, genuine captured descriptor
      data/length, and declaration-owned lifetime-qualified result origin envelopes.
      FOR-LOANS.md records the contract and explicit boundaries. Structured
      referenced elements and mutable-slice iteration,
      SCC-born item loans retained across backedges, non-Copy by-value
      elements, adapters and expression-position for loops remain open.
- [x] Genuine let-else with old-frontier initializer/else validation, dedicated
      persistent-binding IR and checked failure divergence. LET-ELSE.md records
      stock33/native132 cells per bootstrap profile,26 public authority controls
      and the complete2,546-test/126-binary hosted gate. Successful bindings live
      in the enclosing block, not a synthetic match arm. General pattern forms
      and other unsupported boundaries remain open.
- [x] Ordered nested Bool/Int/enum pattern decisions across match, if-let,
      while-let and let-else, with exact prior-tag proofs and all tests before
      successful binding moves. PATTERN-DECISIONS.md records stock30/native120
      cells per bootstrap profile, 26 public authority controls, nine unchanged
      historical witness promotions and complete 2,636-test / 130-binary hosted
      acceptance. Exact admitted product coverage is extended below; ordinary
      refutable lets, guards and reference ergonomics remain open.
- [x] Exact sparse usefulness/exhaustiveness for existing validated Decision
      products, with symbolic integer complements, ordered query refinements
      and transactional metered rows. PATTERN-USEFULNESS.md records stock32 /
      native128 cells per profile, 24 public authority controls and complete
      2,678-test / 132-binary hosted acceptance. Explicit inhabitance boundaries
      retain conservative coverage; cyclic metadata and resource failures fail
      closed. This is not general Rust pattern grammar or full compatibility.
- [x] Fieldless nominal enums and scalar/enum match with exhaustive coverage.
- [x] Scoped local numeric literal constraints from later scalar uses, returns,
      assignments and monomorphic calls, before existing peer-aware fallback.
      `LOCAL-LITERALS.md` records 28 stock-differential witnesses and unchanged
      Collatz exact native check/build/run acceptance. A further 27 iterable
      differential cases plus an empty-array ambiguity control propagate loop
      item evidence into range bounds and direct array/repeat literals,
      unlocking unchanged GCD/Fibonacci exact output. General structured and
      reference inference remain open, with no new graph kinds or ratchet changes.
- [x] Typed deferred local declarations, flow-sensitive first writes and shadowing.
- [x] Payload enums, typed downcasts, bounded selection proofs and native tagged values.
- [x] Fixed arrays, repeat effects, literal indexing and private by-value array ABI.
- [x] Nonempty fixed-array irrefutable let/match patterns with actual prefix/suffix
      syntax, typed ConstantIndex plans and facts-checked Copy-only selected
      bindings. ARRAY-PATTERNS.md records stock27/native108 debug/release cells,
      33 public authority controls and the complete 2,400-test / 120-binary gate.
      Non-Copy extraction and empty/slice/reference patterns remain explicit
      unsupported boundaries, with existing index fences intact. Nested
      refutable children are extended by PATTERN-DECISIONS.md.
- [x] Genuine 64-bit usize and ordered, bounds-checked runtime array operations.
- [x] Nonescaping local references, canonical provenance and last-use loan analysis.
- [x] Explicit borrows of trivial-drop value temporaries (`&P { .. }`, `&f()`,
      `&mut 4`, and unit constructors), with statement temporary cleanup and
      syntactic let-initializer lifetime extension. `tests/generic_references.rs`
      compares runtime output with stock Rust and rejects statement-temporary
      escapes. Destructor-bearing temporaries and projected value borrows remain
      explicit unsupported boundaries, not full temporary-lifetime compatibility.
- [x] Noescape reference parameters and genuine expected-site carrier reborrows.
- [x] Shared/mutable static input-path returns with explicit or elided regions.
- [x] Builtin reference field projections and capability-sensitive mixed inputs.
- [x] Reference-free generic templates with universal move checking,
      application-qualified nominal identity and sealed concrete native instances.
- [x] Bounded first-order omitted call/constructor arguments with deferred integer
      evidence, rigid caller parameters and fallible no-fallback peer hints.
- [x] Hosted acceptance, stock differential cases and qualified benchmark receipts.
- [x] Native f32/f64 arithmetic, IEEE comparisons, saturating numeric casts and
      canonical Copy/Clone recipes, with bounded first-order float inference.
- [x] Lexically resolved labeled loops/while/blocks and target-specific cleanup.
- [x] Universally checked source trait default bodies, override-aware native
      dispatch and explicit Sized proof for owned trait Self values.
- [x] Nested grouped imports, named aliases, type-only self leaves and validated
      empty prefixes, with global logical expansion bounds.
- [x] Named public reexports with canonical identity, inherited variant export
      eligibility and namespace-specific original/intermediary visibility.
- [x] Normalized lexical restricted visibility for supported declarations,
      nominal/field/method access and non-widening named reexports (VISIBILITY.md).
- [x] Canonical monomorphic/first-order generic source type aliases with eager
      unused validation, bounded hygienic expansion and fixed/explicit generic
      constructor/pattern arguments (TYPE-ALIASES.md). Inferred alias constructor
      and pattern arguments remain explicitly unsupported.
- [x] Native i128/u128 arithmetic, exact numeric casts and scalar transport under
      INTEGER128.md, without claiming platform ABI compatibility.
- [x] Configured text/file SOURCE preparation, bounded cfg/cfg_attr and inert
      attribute policy, cfg-before-child-IO discovery and original syntax spans
      under SOURCE-PREPARATION.md. Full inactive syntax and expansion remain open.
- [x] Original-source rustdoc URL-string and `test(attr(...))` metadata grammar,
      preserving byte provenance and inert doctest attributes. Focused stock/native
      CHECK/BUILD/run at workers 1 and 8, malformed controls and graph gates pass
      (tests/doc_metadata.rs). This does not implement rustdoc or doctest execution.
- [x] Original documentation on noncallable associated type/const declarations
      retains exact member spans and accepts no other semantic attributes.
      Stock and native CHECK/BUILD/run workers 1/8 cover the unchanged time-core
      `Per<T>` declaration excerpt and unused invalid members
      (`tests/associated_member_docs.rs`), not full time-core compatibility.
- [x] Explicit source-locked original attributed identity declaration at runtime,
      retained constness with universally checked forwarding-only bodies and
      scalar-root shared/mutable inference replay under CORE-IDENTITY.md. This
      is not CTFE, a library crate artifact or whole-core compilation.
- [x] Rough-draft consts (tests/const_items.rs): const and immutable static
      items of any type (non-scalar ones inlined at each use, still checked when
      unused), `const fn` with ordinary bodies (E0015 for non-const calls),
      inline `const { }` blocks, const-item expressions as array lengths.
      Canonical `const unsafe fn` declarations preserve both qualifiers and unsafe
      call obligations. Duplicate/reversed qualifiers reject; async/extern stay unsupported.
      Open: const generics, static mut, unsafe/raw-pointer ops, unions,
      extern "C", constant patterns, real CTFE of non-scalar values.
- [x] Immutable production parser cursor over the existing lexer, exact type-only
      punctuation views and checked restoration, without macro admission
      (CURSOR-MIGRATION.md).
- [x] Checked structural token/session/tree/provenance utilities and shared
      expansion accounting under TOKEN-PRIMITIVES.md, without claiming a
      hygienic resolver or macro-expansion path.
- [x] Native edition-aware lexical atoms, exact literals/adjacency/doc records,
      sealed prepared-source projection and production parser cutover under
      NATIVE-LEXER.md. New lexical forms do not imply new semantic admission.

## Remaining implementation program

The categories below cover the known program, not an exhaustive decomposition of
Rust. Having every category named is not the same as having every semantic rule,
integration edge and acceptance case in executable todos. Remaining unknowns need
discovery through a pinned semantic inventory, library builds, real projects,
independent review and minimized failures. Add newly discovered work here before
claiming the affected gate is complete. Full Rust support is **not complete**.

Delivered contracts, including later extensions, take precedence over historical
first-client exclusions. In particular, bounded associated dispatch, canonical
Copy/Clone, generic reference carriers, source trait defaults, floats, labels and
shared byte slices are foundations, not absent categories and not category-wide
completion. `GENERIC-CARRIERS-ACCEPTANCE.md` distinguishes design freeze from real
native acceptance. Preserve the checkpoint evidence below without treating its
historical test counts as the current total.

### Gate rules and dependency map

Each gate needs a scoped contract, named owner, dependency/API agreement, and
executable positive/invalid/legal-unsupported regression cases. Run the public
CHECK/BUILD/run checks against the changed revision, using current test-runner or
CI results rather than manually retained success receipts. Keep test names and
commands discoverable; update this tracker only when capabilities or blockers
change. Historical performance comparisons and hard-to-reproduce failures may
need selected retained records (see AGENTS.md and data/runs/README.md). Existing
checkpoint logs below are historical, not a template for new per-run narratives.
Invalid unused declarations and dead metadata
must still reject. CHECK must not demand code generation. Failed correctness,
unsupported input, resource limits and infrastructure faults remain distinct and
must never publish a successful executable. Internal graph tests supplement,
not replace, actual source acceptance. Every supported extension needs review of
its interactions with prior types, ownership, substitution and runtime behavior.

The dependency labels below constrain individual slices, not whole-program phase
barriers. Independent design, oracle classification and harness work starts now.
A downstream native client starts as soon as its exact prerequisite slice passes:

- G0 (contract/discovery) and G10 (conformance) run continuously across all gates.
- G1 (source/names) and G2 (typed data) extend the delivered semantic foundation.
- G3 (ownership) and G4 (abstraction) co-design shared type/region interfaces on
  G1/G2. Neither waits for the entire other category to finish.
- G5 (expansion/const) uses G1/G2 and applicable G3/G4 semantics. Its discovery
  can add declarations and dependencies before other bodies are available.
- G6 (runtime/targets) consumes validated G2/G3/G4 facts. ABI design and external
  harnesses can proceed before those consumers are implemented.
- G7 (libraries) advances incrementally on G1-G6, beginning with pinned core
  blockers rather than waiting to attempt the entire standard library.
- G8 (projects) starts manifest/discovery work now, then integrates G5-G7 for
  native dependency, build-script and proc-macro execution.
- G9 (graph/cache/security) is cross-cutting. Persistent/distributed reuse needs
  explicit identity, validation and isolation gates beyond in-snapshot sharing.
- G11 (performance/completion) measures each accepted slice, but broad promotion
  waits for the declared language, library, target and project gates.

### G0. Versioned compatibility contract and discovery

- [ ] Pin the stable language/toolchain oracle revision, Rust Reference/library
      source revisions, edition matrix, target triples and supported target
      features. Edition 2024/Linux x86-64 is the initial track, not evidence for
      Rust 2015/2018/2021, other platforms or all future stable releases.
- [ ] Build a traceability ledger linking semantic requirements and known
      unsupported fixtures to a contract, implementation owner, dependencies,
      acceptance cases and evidence. Audit lexical grammar, attributes, lints,
      edition differences and cross-feature interactions as well as feature names.
- [ ] Maintain a discovery queue for unspecified/ambiguous behavior, unsupported
      library requirements and new failures. Separate stable-contract obligations
      from unstable features without silently narrowing the full Rust goal.

**Next:** freeze the initial version/edition/target matrix and classify every
current legal-unsupported boundary against it. **Acceptance:** each matrix cell
has an explicit supported/unsupported/not-yet-investigated status and linked
cases. An empty discovery queue or a long checklist is not proof of exhaustiveness.

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
      claim image coverage. Registry planning IO, unsupported configuration sources,
      dynamic libraries and process state, external tool identity, registry inspection
      tools and uncontrolled build scripts/proc macros remain explicit gaps.
      Exhaustion or uncontrolled discovery cannot issue an input
      identity. No persistent or global private cache is enabled by this foundation.
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

### G10. Continuous conformance, soundness and diagnostic evidence

- [ ] Maintain versioned positive, invalid, legal-unsupported, wrong-code,
      resource-limit and infrastructure outcomes with source/oracle identities.
- [ ] Import or independently implement applicable language/library conformance
      cases, differential generators and fuzzing with minimization and permanent
      regression retention. Audit licensing/attribution of external test inputs.
- [ ] Test cross-feature combinations, not only individual frozen corpora:
      Drop with generics/unwind, macros with privacy/editions, projections with
      regions, async with captures and cfg/features with cache invalidation.
- [ ] Exercise real CLI packaging, artifacts, diagnostics/spans, exit status,
      time/resource limits and all-unused checks at single/multiple workers.

**Next:** keep a machine-readable requirement-to-case index in the test system and
prioritize soundness/wrong-code and library/project blockers over accumulating
small corpus counts. **Acceptance:** every claimed contract row maps to a
rerunnable automated check whose current result passes, known valid unsupported
programs remain gaps, and discovered failures acquire minimized regression tests
before the corresponding completion claim is restored. Ordinary passing runs need
no additional receipt archive or prose report.

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

### G11. Representative performance and final compatibility audit

- [ ] Measure reproducible debug/release, cold/no-change/edit, build/check/test
      sequences, selected/full tests and single/concurrent-project workloads.
      Include CPU/span, memory, cache/transfer, instrumentation and service costs.
- [ ] Retain comparable before/after session-weighted suite receipts, inspect
      regressions by command/change, and measure actual server scaling/topology.
      Synthetic graph wins and subset timings are not full-Rust speed evidence.
- [ ] Audit every declared version/edition/target row, native core/alloc/std,
      independent unchanged projects and self-hosting. Publish remaining gaps,
      unknowns, exclusions and evidence coverage before any full-support claim.

**Next:** define the representative project/workload roster now, mark currently
blocked cases honestly, and benchmark accepted slices without generalizing them.
**Acceptance:** correctness gates pass on the native path and the broad benchmark
comparison is reproducible. Self-hosting is necessary evidence for the intended
compiler workflow, not sufficient proof of full Rust support or optimality.

### Parallel execution plan: six bounded workstreams

A swarm is sufficient only when these streams have owners, independent next
artifacts, agreed shared APIs and reviewers, not merely many active agents.
This is a staffing recommendation, not a claim about currently running workers.

1. **Source, editions and expansion (G0/G1/G5):** own source/span/lookup/token
   contracts and source corpora. Start the source-map/import and hygiene inventory.
2. **Ownership and dataflow (G2/G3):** own place/region/drop contracts and cleanup
   acceptance. Start Drop-order fixtures and the aggregate/SCC region gap map.
3. **Abstraction and typed lowering (G2/G4):** own trait normalization, inference,
   instance/capture contracts. Start associated-type and Iterator-like cases.
4. **Platform and libraries (G6/G7):** own ABI/runtime and pinned core blockers.
   Start the C ABI matrix and library language-item/intrinsic inventory.
5. **Projects and secure reuse (G8/G9):** own manifests, host/target orchestration,
   cache/input/isolation contracts. Start the workspace fixture and invalidation
   threat matrix, coordinating cloud policy in its separate repository.
6. **Independent conformance and measurement (G0/G10/G11):** own frozen oracle
   classification, cross-feature reviews, real-project roster and benchmark
   receipts. Start the traceability index and minimize the highest-risk gaps.

Use explicit file ownership and coordinator-reviewed shared IR/API changes,
especially in the common frontend, type world, CFG and ownership analysis. Do
not let several agents concurrently rewrite those files without an agreed merge
boundary. Four streams are possible by combining source with project discovery
and platform with measurement, but retain an independent correctness reviewer.
The six streams expose useful parallel work now, not six independent full
compiler implementations. Reassess staffing against ready tasks, blocked APIs
and measured integration throughput after each accepted slice.

## Active work and evidence limits

Known failing suite (2026-10-24): `justrust test --test prelude_traits` fails
5 of 6 at 786937658, ef3ce0e00 and HEAD with `missing or duplicated Key {
kind: 110 (TEMPLATE_BODY), params: [1] }` from `assert_graph_authority`: the
test expects every callable's TEMPLATE_BODY to be evaluated exactly once, and
one is now never evaluated. It predates this session's prelude library commits
(reverting them does not change it). Unresolved: either a scheduling regression
or a stale expectation after per-module ITEM_INVENTORY fan-out.

Also failing at 98807706e (before 2fc0cf11b): `subslice_authority`
(`mutable_subslice` `&mut a[1..]` now compiles where the test expects an
explicit unsupported diagnostic; `non_usize_bound` reports E0308 without the
word `usize`) and `mutable_slices_authority` (1 test). Both look like stale
expectations after mutable subslice support landed; not yet reconciled.

Handoff (2026-10-21, semantic-core lane): native `for` over a source iterator
value (`for x in vec.iter()`, `for c in s.chars()`, a user `IntoIterator`)
is rejected on main 136ffab82 and every commit checked back to 1b43e6a71, with
distinct errors per form ("nominal source constraints cannot progress",
"reference-bearing projected method signature", "closed associated projection
must normalize before execution"). `iterator_associated`
`native_vec_shared_iteration_matches_stock` and `unchanged_sieve_matches_stock`
fail. `while let Some(x) = it.next()` over the same iterators works. This is the
known blocker of 8767ed720 and gates programs 04, 06, 09, 10, 15 and 17 plus a
ready `str::chars` (`StrMethods`, Self = `str`; works through `.next()`). The
uncommitted `libraries/prelude/iter.rs` edit seen at 03:46 fails
`graph_integrity::optimizer_nodes_add_no_whole_program_barriers` ("trait method
argument arity mismatch"); HEAD's iter.rs passes. Semantic-core status and the
cutover design are in `SEMANTIC-CORE.md`.

Closed nominal field realization (`NOMINAL-PROJECTIONS.md`): associated factory
receiver hints now reuse full canonical Call signature realization before result
extraction, preserving strict method selection and original/mapped source receipts.
Explicit-owner struct/enum factory chains pass stock/native graph CHECK-no-image,
BUILD and run at workers 1 and 8; unused wrong arguments/missing members and genuine
caller-facts missing/wrong declaration/ordered actual metadata controls reject.
Frozen `b758c40e` on `519a7ff4` passes check, library clippy (existing warnings) and
all 14 graph gates (`20261020-013907002-3182685`). Exact static-owner parent carry
onto `59f91c4b` passes 345-target checking and the explicit positive again
(`20261020-014327809-3192995`, `20261020-014347945-3196974`); the earlier authority,
clippy and graph gates retain their original-parent provenance. Nine authentic
summaries are in `data/runs/associated-result-receivers/2026-10.jsonl`. This does
not claim full NonNull, Iterator or Rust acceptance.
The previously accepted closed realization evidence follows unchanged: the unchanged
nested closed `Peek<I: It>` struct and closed enum consumers pass stock/native
CHECK, BUILD and execution at workers 1 and 8. Typed graph reads preserve source
declaration/ordered actuals separately from engine-issued representation identity.
Unused-invalid, typed-capability, source-call omission and constructor-ingress
source/snapshot/actual/foreign-engine controls pass. Closed generic BODY consumers
now independently require proof/engine mode from authenticated schema arity and
target/provider authority. Whole-proof/engine-plus-inventory removal and changed
source-application bypasses are rejected by preserved real-consumer controls.
Closed call receipts replay the complete original/current occurrence inventories,
ordered substitution, selection, resolved target, contract and current-engine
conversion evidence. The unchanged forwarded Hash consumer passes stock/native
CHECK/BUILD/run at workers 1 and 8 (`20261019-231256541-2703029`), with strict call,
borrow and contract-identity guards unchanged. The final receipt mutation control,
341-target check and all 14 graph gates pass (`20261019-232217144-2733864`,
`20261019-232315842-2737020`, `20261019-232409929-2742857`) without ratchet or stack
changes. After exact embedded-function authority parent carry, 341-target checking
and its genuine helper/mutation integration control pass (`20261019-233202311-2762780`,
`20261019-233242152-2767358`); the feature doc retains the earlier graph/Hash frozen
provenance rather than claiming those were rerun on that later parent. This is a
bounded closed acyclic non-closure slice, not full G0-G12 completion. Full Iterator
activation, recursive or closure realization, reference-carrier/result admission
and full Rust compatibility remain open.

Full-program target corpus (FULL-PROGRAMS.md, `tests/full_programs.rs`): 20
realistic std-only programs with stock oracle stdout and a pass/fail ratchet
manifest. The committed manifest ratchet records 5/20 passing programs, as
reported in FULL-PROGRAMS.md. This is not a fresh corpus measurement after the
latest scoped changes. FULL-PROGRAMS.md ranks the recorded blocking features.
The current work advances `11_closures_iterators.rs` through CHECK. Opaque
specialization now retains current-engine nominal reads and exact call ABI
identities (`closure_generic_owners::opaque_callable_with_realized_nominal_local`).
Opaque bodies also retain unchanged source destructors through their canonical
INSTANCE cleanup demands, including root and generic-owner drop order
(`closure_generic_owners::opaque_callable_preserves_existing_cleanup`). Cleanup
introduced by hidden representation changes remains unsupported. Physical
reference results now retain exact immutable boundary authority instead of
replaying ordinary source-signature checks against physical rows. Shared and
mutable results, escaping-local rejection, and dormant outer opaque bindings
are covered by `closure_generic_owners`. Unused outer-binding lookup slots no
longer introduce opaque representation dependencies into independent closures.
Nominal schema authentication now checks one definition at a time within the
existing recursive type-validation walk, retaining nested-payload rejection
without revalidating every subtree at every ancestor. The source `Filter`
implementation preserves `I::Item` directly instead of erasing its provenance
through a separately constrained type parameter. The unchanged
`11_closures_iterators.rs` now passes differential CHECK, BUILD and execution at
workers 1 and 8 (`iterator_associated::unchanged_closures_iterators_match_stock`),
with escaping filtered-reference rejection covered by
`filtered_reference_items_preserve_their_source_loans`. This promotes that
single full-program fixture, not full Rust compatibility.

Shared slice iterators retain their typed backing slice rather than erasing
element-reference provenance through a raw pointer. Exact template substitution
can supply shared aggregate covers, and slice roots use only their issued
summary cells. `typed_slice_authority::reference_iterator_collection_matches_stock`
and `reference_iterator_borrow_rules_match_stock` exercise collection and source
loan preservation at workers 1 and 8. Mutable-reference slice elements and the
generic trait-owner slice inference boundary remain unsupported.
- [ ] **Soundness blocker:** `Vec<&T>`/`Vec<&mut T>` erase element loans because
  `carrier_locations` gives `*mut T` no channel. `v.push(&a); a.push('x');`
  and `let r=&mut a; v.push(r); let r2=&mut a; v.push(r2);` are accepted (stock
  E0502/E0499). Naively walking raw pointer pointees breaks the prelude's own Vec
  bodies (ResourceLimit, affine copy-cell cover). Needs a typed element-provenance
  channel for raw-pointer owners (like the typed slice backing above).
Opaque backing owners retain unresolved summaries rather than acquiring invented
element cells; generic `Bag<T>` formatting/collection is covered by
`iterator_associated::generic_iterator_format_collect_reuses_membership_proofs`.

The source-type-alias checkpoint passes all 1,649 hosted tests in 86 binaries
(`20261017-043356389-4183342`), all-target checking
(`20261017-043354163-4183206`), strict all-target lint
(`20261017-043405146-4184072`) and formatting. Its independent executable stock
corpus classifies 31 supported valid, 21 invalid and 10 legal unsupported sources
before 248 native CHECK/BUILD cells at workers 1 and 8. Fixed/explicit constructor
and pattern arguments retain canonical target constraints, while omitted generic
alias argument inference remains unsupported. Two graph tests check native image
identity and unused invalid aliases, and an external-file test exercises generic
alias definition context through imports. The unchanged historical visibility and
match alias sources are promoted after exact executable stock/native acceptance.
An iterative dependency schedule fixes the discovered stack overflow without
increasing thread stacks: a 40-link alias chain succeeds on 128 KiB and growing
cycles fail diagnostically. TYPE-ALIASES.md defines the bounded contract. Full
source organization, abstraction, libraries, projects and full Rust remain open.

The restricted-visibility checkpoint passes all 1,641 hosted tests in 84 binaries
(`20261017-041943076-4105659`), all-target checking
(`20261017-041939306-4105527`), strict all-target lint
(`20261017-041953248-4107714`) and formatting. Its executable stock corpus has
25 supported valid sources, 43 invalid sources and one explicitly legal but
unsupported type alias. All 69 sources run native CHECK/BUILD at workers 1 and 8.
Three graph tests check canonical image identity, nested trait implementation
owner scope and unused invalid declarations. A filesystem stock/native test
covers restricted module/type/field access across real external files. Ordinary
test-thread stack checks accept module depth 128 and reject 129/300. Scope
clone budgets and path normalization bounds have direct regressions. Both
unchanged historical restricted-visibility sources are promoted only after
exact-source stock/native execution. VISIBILITY.md defines the bounded contract.

The named public reexport checkpoint passes all 1,630 hosted tests in 82
binaries (`20261017-040632890-4019165`), all-target checking
(`20261017-040631810-4019111`), strict all-target lint
(`20261017-040637221-4019712`) and formatting. The 44-source independent
stock oracle distinguishes 15 valid from 29 invalid sources. Fourteen valid
sources execute natively and the legal imported-main boundary stays explicitly
unsupported. All 44 are checked in native CHECK/BUILD at workers 1 and 8.
Two graph tests check canonical executable identity and unused private
reexports, and filesystem acceptance covers public aliases from a private
external module. Both unchanged historical public-use sources are promoted
after exact-source stock/native execution. REEXPORTS.md defines the bounded
contract, not full source organization or full Rust support.

The grouped-import checkpoint passes all 1,622 hosted tests in 81 binaries
(`20261017-035808751-3962286`), all-target checking
(`20261017-035807953-3962199`), strict all-target lint
(`20261017-035813817-3962848`) and formatting. Independent stock/native fixtures
cover type-only self aliases, empty groups, trait scopes, namespaces, privacy,
unused imports and cycles. Two graph tests check canonical native image equality
and all-unused rejection. Filesystem tests cover nested external modules around
use-group braces. The two unchanged historical grouped sources now execute
natively after separate exact-source stock execution. Normal-stack resource
tests exercise per-tree and global expansion limits. GROUPED-IMPORTS.md defines
the bounded contract. Named reexports are the following delivered slice above.
Source maps, glob resolution, anonymous imports, full source organization and
full Rust stay open. Restricted visibility is the later bounded slice above.

The full requested outcome is not reached. The native float and labeled-control
checkpoint passes all 1,576 hosted tests (`20261017-031332671-3543206`), all-target
checking (`20261017-031201245-3527437`), strict all-target lint
(`20261017-031343109-3542975`) and formatting. The executable float and label
corpora compare stock and native output at workers 1 and 8. Five float and three
label graph tests additionally check shared facts, malformed IR, contextual
break values, moves, loans and unused invalid bodies. Normal-stack parser nesting
acceptance passes after isolating labeled-expression parsing into a small frame
(`20261017-031112594-3517995`). `FLOATS.md` and `LABELED-CONTROL-FLOW.md` define
these bounded contracts. The subsequent source-default checkpoint passes all
1,608 hosted tests (`20261017-033939643-3821254`), all-target check
(`20261017-033936448-3820907`), strict all-target lint
(`20261017-033944415-3822910`) and formatting. `TRAIT-DEFAULTS.md` defines
universally checked source default bodies and override-aware dispatch. Independent
review exposed potentially-unsized owned Self positions. Executable signatures
and shared IR owned-value gates now require real Sized proof, while borrowed
places retain their original authority. Invalid unused and overridden defaults,
including diverging/deferred Self locals, reject against the stock oracle.
Canonical catalog placeholders remain explicitly unavailable. No category-wide
or full Rust completion is claimed.

Earlier bounded generic inference passes 1,270
hosted tests, strict all-target lint, its unchanged 28 exact source cases and four
instance/trust graph tests. One historical generic-enum source is promoted only
after unchanged stock/native runtime parity. The tracked 27-corpus campaign
verifies 956 native cases on revision 6cef2b5c6feb16ea30692d94b0edb759fd2d91d2,
including 577 accepted parity, 35 declared traps, 266 invalid rejections,
77 legal boundaries and one distinct bounded-instantiation resource outcome.
All raw receipts and the actual 96-worker example are retained. There is no successful
core/alloc/std build, broad existing-project acceptance or representative full-Rust
performance comparison. INTEGER-TYPES.md now records delivered signed/unsigned
primitives through64 bits, typed literals/patterns and existing checked operations.
Integer casts under INTEGER-CASTS.md now pass exact32 stock/native cases and
four graph tests, with soft source inference grounded in primary semantics.
The tracked22corpus native campaign verifies all816cases.
Additional operators remain separate follow-through work.
Full Rust completion remains the goal, not this bounded milestone.
Native Integer128 is delivered under INTEGER128.md. Broader float library/ABI
behavior, broader inference/traits/methods, Drop,
reference-containing values, libraries and project integration remain open.

## Delivered milestones, newest first

Numbers below are evidence at each checkpoint, not cumulative full-Rust coverage.

### Unchanged Adler2 2.0.1 test suite

Hosted `no_std` test harnesses are now admitted without relaxing the source
crate's core-only policy. Generated runtime calls bypass user `std` aliases
and shadowing. The unchanged Adler2 probe passes check/build/consumer run and
upstream tests in both configurations at workers 1/8 (5 no_std tests, 6 std
tests). Gates: `experiments/adler2_repo_probe.py`,
`no_std_tests_use_hosted_harness_without_importing_std_into_user_code`,
`no_std_graph`, `no_std_authority`, and `graph_integrity`. Bare no_std
executables and explicit std opt-in remain unsupported.

Unchanged Adler2 `test --lib` passes 6/6 natively at workers 1 and 8, matching
the recorded stock run. Delivered for it: `extern crate <dependency>` (crate-root
aliases join the extern prelude), block-local `fn`/`use` items hoisted under
hidden names, `&mut R` forwarding readers (identity carrier writes are not
affine) with `R` inferred from `f(&mut r)`, and stock static promotion of
`&[byte_literal; N]` of u8 beyond 65535 elements (`ByteRepeat`). Type-level
array walks use `MAX_ARRAY_TYPE_LEN`; per-element expansion keeps `MAX_ARRAY_LEN`.
Other large arrays (non-literal, `&mut`, wider elements, owned locals) remain
explicit unsupported boundaries. Doc-tests are not executed natively. Tests:
`target_dependencies extern_crate_items_name_dependencies_like_stock`,
`const_items block_local_fn_and_use_items_match_stock`,
`std_io buffered_byte_readers_match_stock`,
`slice_methods promoted_large_byte_repeats_match_stock`,
`slice_methods unpromoted_large_arrays_remain_explicitly_unsupported`.

### Edition 2021 admission and unchanged Adler2 2.0.1

Edition 2021 sources compile natively; 2015/2018 stay unsupported. Constructs
whose 2021 meaning differs from the implemented 2024 rules reject explicitly
(Drop-carrying block-tail and if-let scrutinee temporaries, default binding
mode resets, `impl Trait` return capture, let chains). Inactive items accept
block-local `use`/`fn` items and oversized array lengths as syntax only.
Unchanged Adler2 2.0.1 passes native check/build (std and no_std check) and a
stock-checked consumer runs at workers 1 and 8. Tests:
`project_configuration edition_2021_is_admitted_with_stock_semantics_or_explicit_boundaries`,
`inactive_block_local_items_are_syntax_checked_only`,
`inactive_array_lengths_skip_the_semantic_length_limit`. Adler2's own test
suite now also passes (see the checkpoint above).

### Native i128/u128 scalars

INTEGER128.md adds canonical two-limb integer values, checked native arithmetic
including multiplication/division/remainder, all 144 integer casts and direct
f32/f64 wide conversions. Private 16-byte scalar Copy transport preserves both
limbs through generic calls, references and mixed aggregates. Mutable-reference
indexed compounds retain stock RHS/index/current-carrier order, initialization
and every-hop mutable authority, including dead malformed metadata. Physical
scratch planning preserves sealed nominal Copy facts instead of re-proving them
with standalone facts.

All 1,677 hosted tests across 89 binaries, all-target check, strict lint and
formatting pass. Independent stock classification covers 92 exact sources, then
368 native workers1/8 CHECK/BUILD cells with ELF/runtime and artifact gates.
Five unchanged historical integer-type/operator/cast boundaries are promoted
only after executable stock/native parity. Three canonical graph tests, native
arithmetic and roughly 25,000 IEEE conversion oracles complement source coverage.
Retained receipts are data/runs/xps14/2026-10-17-integer128.jsonl. No platform ABI,
profile-exact panic/unwind, complete numeric libraries, full Rust or speed claim
is implied.

### Bounded first-order generic inference

Frontend-only inference now resolves fully omitted call/constructor arguments
from hard expected types and bounded known evidence, with schema-only slots and
rigid caller types even during same-binder recursion. Partial product evidence
retains opaque/numeric leaves privately, defers fallback until all constraints
and never lets unknown evidence force a default. Fallible no-fallback peer hints
are not committed solutions. The ordinary checker still elaborates each child
exactly once in source order before unchanged universal/concrete trust gates.
All 1,270 hosted tests, strict all-target lint and formatting pass. The exact28
frozen source corpus and four graph gates pass, and the real eight-worker native
example executes successfully. Actual reviews closed schema scratch ordering
and constructor lookup-work gaps. No generic references, partial type holes,
global future-use inference, unresolved nested omission or full Rust claim is
introduced. See GENERIC-INFERENCE.md and its retained validation profiles.
Earlier boundary statements describe that historical slice; the newest contracts
and the remaining implementation checklist determine current support.

Integer casts under `INTEGER-CASTS.md` cover all 100 primitive pairs without
overflow traps. Soft source expectations preserve concrete types and checked
literal diagnostics. All 1,067 hosted tests, strict all-target lint, formatting,
exact32 stock/native and four graph tests pass. The unchanged historical cast
fixture and real native example execute. Admitted120cast chains pass native
lowering/execution on the normal stack, with over-cap and cyclic private IR
rejected by bounded lowering. All816tracked22corpus outcomes pass, with23raw
corpus/example receipts retained and the96worker example executed.

Integer foundation through 64 bits is implemented under `INTEGER-TYPES.md`.
All ten integer types preserve exact identity, canonical typed constants and
patterns, contextual radix literals and signed minima. The width-driven native
kernel checks overflow, division and remainder while preserving the private
eight-byte scalar ABI. All 1,021 hosted tests, strict all-target lint and formatting
pass. Exact 32 stock/native fixtures and four graph tests pass, including nine
explicit runtime-101 cases. All 784 tracked twenty-one-corpus native outcomes
pass, with 22 raw corpus/example receipts retained. Casts and the other numeric
gaps remain open.

Mixed-capability reference-result inputs are implemented under
`MIXED-REFERENCE-RETURNS.md`. Shared outputs derive from shared or mutable input
paths, mutable outputs only from mutable inputs, and unrelated inputs retain
their own kinds and region dependencies. Genuine reborrow/unique-return semantics
and all existing limits remain unchanged. All 975 hosted tests, strict all-target
lint, formatting, exact stock20/native20 and four graph tests pass. Four historical
sources execute unchanged after capability promotion. All 752 tracked twenty-corpus
native cases pass, with raw receipts and the executed 96-worker example retained.
Full Rust remains open.

Builtin reference field access is implemented under `REFERENCE-FIELDS.md`.
Frontend normalization inserts actual Deref for `r.field`/`r.0` across reads,
places and scalar hints, preserving original receiver, authority and privacy.
Typed guards keep dynamic reference indexing and enum downcasts unsupported.
All 942 hosted tests, strict all-target lint, formatting and both independent
actual-code reviews pass. The new 20-case stock/CLI corpus and four graph tests
pass. Three historical implicit getter sources and the historical tuple-reference
fixture are promoted with exact source/output bytes unchanged. All 732 tracked
nineteen-corpus native cases pass, with raw receipts and the executed 96-worker
example retained. Full Rust remains open.

Free-function lifetime elision is implemented under `LIFETIME-ELISION.md`.
Stock E0106 probes corrected the initial distinct-ID proposal before release:
output elision counts input reference lifetime positions, not distinct IDs. Fresh
anonymous inputs remain rigid, explicit outputs cannot infer missing bounds, and
NoReference call contracts retain their erased B2 behavior. All 915 hosted tests,
strict all-target lint, formatting and both independent actual-code reviews pass.
The corrected 20-case stock/CLI corpus and four new graph tests pass. Four
historical CLI sources and one graph source are promoted without source edits.
Tracked run `compiler-lifetime-elision-20261016-0915` at `0fe4b3356` passed all
712 cases across eighteen corpora: 425 accepted, 213 invalid rejected, nine
declared runtime-101 traps and 65 legal-unsupported boundaries. All raw receipts
and the executed 96-worker example are retained. Full Rust remains open.

Projected input reference returns are implemented under
`PROJECTED-REFERENCE-RETURNS.md`: complete declaration-derived typed static paths,
source-specific candidate/argument correlation, private-field privacy separation
and bounded occurrence enumeration. All 888 hosted tests, the frozen 30-case stock
corpus and four workers1/8 graph acceptance tests pass. Both independent actual-code
soundness/resource reviews, strict all-target lint and formatting pass. Tracked
M8a run `compiler-projected-reference-returns-20261016-0859` at `9048f4a33` passed
all 692 cases across seventeen corpora: 411 accepted, 205 invalid rejected, nine
declared runtime-101 traps and 67 legal-unsupported boundaries. Raw receipts and
the executed private-field 96-worker example are retained. Full Rust is open.

Mutable whole-parameter reference returns are implemented under
`MUTABLE-REFERENCE-RETURNS.md`, with exact kind contracts, one unique returned
handle, candidate-specific authority and transitive lifetime dependencies. Normal
returns prove universal provenance before consuming a mutable carrier. All 844
hosted tests, strict all-target lint, formatting and both independent actual-code
reviews pass. The exact 30-case stock corpus and four graph tests pass, including
parent freeze/resume and failed unused-callee publication. Tracked M8a run
`compiler-mutable-reference-returns-20261016-0843` at `49a1ae343` passed all 662
cases across sixteen corpora: 394 accepted, 196 invalid rejected, nine declared
runtime-101 traps and 63 legal-unsupported boundaries. All raw receipts and the
96-worker example are retained. Later delivered milestones above add static
projected/mixed-kind results. General lifetime inference and full Rust remain open.

Noescape reference parameters and expected-site reborrows are implemented under
`REFERENCE-CALLS.md`. All 748 hosted tests, strict all-target lint, formatting and
independent resource/soundness review pass. The unchanged 30-case stock corpus
separates 18 successful, eight invalid and four legal-unsupported programs.
Formal referent roots outlive carrier slots, and mutable calls invalidate all
candidate referent proofs without inventing initialization. Tracked fourteen-
corpus native verification passed all 602 cases: 361 successful parity cases,
178 invalid rejections, nine declared runtime-101 traps and 54 explicit legal-
unsupported boundaries. Raw receipts and the executed 96-worker example are
retained under `experiments/results/20261016-m8a-reference-calls-*`.

Shared whole-parameter reference returns are implemented under
`REFERENCE-RETURNS.md`. The frozen 30-case corpus and four retained graph tests
pass hosted acceptance after fixing named-lifetime scanning in filesystem module
loading. The final frozen hosted suite passes 798 tests, strict all-target lint,
formatting and bounded resource/provenance review. Tracked fifteen-corpus native
verification passed all 632 cases at bb3bc5613: 378 successful parity cases, 186
invalid rejections, nine declared runtime-101 traps and 59 legal-unsupported
boundaries. Raw receipts and the executed 96-worker example are retained under
`experiments/results/20261016-m8a-reference-returns-*`. Mutable and projected
results are delivered by later milestones above; general regions and full Rust
remain open.

Genuine nonescaping local references are implemented under `BORROWING.md`, with
canonical issuance-time loan origins, explicit reborrows, backward carrier
liveness and native thin-pointer addressing. The frozen 35-case external oracle
contains 17 accepted programs, 14 invalid programs and four later-stage boundaries.
All 701 hosted tests, strict all-target lint, formatting and final peak-memory
accounting review pass. Tracked M8a validation passed 572 cases across thirteen
corpora: 343 accepted parity, 170 invalid rejections, nine declared runtime-101
cases and 50 legal-unsupported boundaries. Raw receipts and the successful
96-worker native borrowing example are retained. That B1 checkpoint left
reference parameters and implicit expected-site reborrows for the B2 slice above.
General escaping-reference behavior, general regions and reference-containing
aggregates remain open. Later milestones above add bounded input-path reference
results. This does not close the ownership or full-Rust checklist.

Genuine unsigned 64-bit `usize` and runtime array indexing now pass 647 hosted
tests and strict all-target lint. `USIZE.md` records ordered index/readiness
operations, owning RHS snapshots, full-width arithmetic/bounds, direct ignored
non-Copy indexing and conservative selection-proof invalidation. The dedicated
32-case corpus distinguishes 15 successful programs, six invalid programs, nine
explicit exit-101 runtime cases and two legal-unsupported boundaries. The exact
prior array runtime-index boundary is promoted unchanged to accepted. Backward
numeric inference, chained dynamic places, slices, broader primitives and
general references/borrowing remain open. Tracked M8a validation passed 537 cases across
twelve corpora: 326 accepted parity, 156 invalid rejections, nine declared runtime
exit-101 cases and 46 explicit legal-unsupported boundaries. The full-width native
example executed at 96 workers with complete capture; raw receipts are retained.

Fixed arrays and decimal literal indexing now pass 591 hosted tests and strict
all-target lint under `ARRAYS.md`. The bound is 4096 elements, with independent
expanded-path, work and frame limits. Repeats evaluate once even at length zero,
indexed wildcards inspect initialized array storage, and non-Copy indexed moves
reject. The independent 30-case corpus separates 12 supported, 13 invalid and
five legal-unsupported programs. Dynamic `usize` indices, slices and array
patterns remain later work. The tracked M8a run passed505 cases across eleven
corpora (310accepted,150invalid,45legalunsupported), with a successful96-worker
native example and all raw receipts retained. Full Rust completion remains open.

Payload enums now pass 533 hosted tests plus strict all-target lint.
`PAYLOAD-ENUMS.md` defines tuple/braced/unit construction, selected payload
patterns, typed downcasts and separate must-selected-variant proofs. The exact
historical payload fixture is promoted unchanged to accepted. The independent
44-case corpus separates 23 accepted programs, 15 invalid programs and six
legal-unsupported boundaries. The tracked M8a run passed 475 fixtures across
ten corpora: 298 accepted parity cases, 137 invalid rejections and 40 explicit
legal-unsupported boundaries, plus a successful 96-worker native example.
Full Rust remains open, including arrays/numerics,
general patterns, references/borrows/destruction, abstractions and project support.

Typed deferred initialization now passes 465 hosted tests, including branch/match
first writes, loop assignment history, fresh-shadow reads and graph failure
analysis retention. `DEFERRED.md` records the contract. The exact historical
wildcard/uninitialized match fixture is now accepted rather than unsupported.
The dedicated differential corpus contains 10 accepted cases, seven invalid
programs and three explicit inference/pattern boundaries. Payload enums are
the subsequent implemented slice above. The M8a run
passed 431 fixtures across nine corpora: 274 accepted parity cases, 122 invalid
rejections and 35 explicitly unsupported legal programs, with raw receipts retained.

Fieldless enums and scalar/enum match now pass 434 hosted tests. `ENUMS.md`
records nonconsuming variant inspection, selected binding moves, finite coverage
and static ownership checks for unreachable arms. Payload variants, sum-aware
move paths, guards, or/range/nested refutable patterns and broader enum features
remain open. The 59-case independent corpus separates 26 accepted cases,
20 invalid programs and 13 legal-unsupported boundaries. The full M8a regression
run passed 411 fixtures: 263 accepted parity cases, 115 invalid rejections and
33 legal-unsupported boundaries, with raw receipts retained.

Named nonempty structs now extend the cached initialization foundation with
nominal identity, field privacy, whole/partial moves and native aggregate calls.
The following slices remain open: general patterns and destructuring assignment,
references, loans, lifetimes, destructors, enums, broader types and categories above.
This milestone passed 278 hosted tests and all 295 M8a source fixtures, including
19 accepted nominal execution cases, 20 invalid nominal programs and two explicit
legal-unsupported nominal boundaries.

Nested irrefutable let patterns now pass 356 hosted tests, including independent
source acceptance and rejected-move graph retention, plus 352 M8a differential
fixtures: 237 accepted parity cases, 95 invalid rejections and 20 explicitly
legal-unsupported boundaries. Pattern matching/exhaustiveness,
remaining types/control flow and references/loans/destruction remain following
slices. `PATTERNS.md` records the exact implemented contract and explicit gaps.

The internal initialization/move-analysis foundation passed 209 hosted tests and
all 254 existing M8a source fixtures. Both check and emission require the same
cached analysis fact, with resource exhaustion failing closed. This is not a
borrow checker or full Rust support.

Explicit CFG lowering passed 178 hosted tests and 254 M8a differential fixtures:
186 accepted parity cases, 55 invalid rejections and 13 legal-unsupported boundaries.
The old structured native emitter is removed. Both roots share the exact validated
CFG, retained for inspection and explicit trace output.

Copy tuple values passed 169 hosted tests, 38 M8a tuple differential cases and
the existing 28-case Never corpus. These are completed prerequisite slices, not
completion of general types, ownership, source organization or full Rust.

Loop control flow passed 148 hosted tests and the charged M8a differential corpus:
64 generated valid loop programs plus 28 Never-typing cases. Three stock-legal
pure-Never comparison cases remain explicit unsupported inference boundaries.
Labels, for loops and general while expressions remain following work.

External source loading and scalar typed IR are completed prerequisite slices,
not completion of source organization, general types, ownership or full Rust.

No checkbox for an entire category is closed by implementing one slice. Unsupported
legal programs remain tracked as gaps, never counted as conformance successes.

### Eager bitwise checkpoint

`BITWISE.md` is implemented for all ten integer primitives and Bool. Exact types,
eager left-to-right snapshots, canonical normalized results and short-circuit
And/Or separation are validated. All1100 hosted tests and strict all-target lint
pass, including exact20 stock/native cases and four graph tests. Independent
23corpus native follow-through passes all836cases, with24raw receipts and
actual96worker example execution retained. Shifts and compound assignments
were separate work at that checkpoint, and full Rust remains open.

### Checked heterogeneous shifts checkpoint

`SHIFTS.md` is implemented: independently typed integer counts, full-width guard
before hardware masking, truncating left shifts and signed/unsigned right shifts.
The private bounded typed count diagnostic preserves runtime effects, distinguishes
semantic Never from typed noncompletion and shares memo/work/depth limits.
All1145 hosted tests, exact32 stock/native cases, four graph tests and strict
all-target lint pass. Earlier CHECK diagnostics than stock metadata are explicit
for two static invalid fixtures. Independent24corpus evidence passes868cases,
with796normal worker comparisons,54declared trap runs,128generated exit checks
and actual96worker example execution retained in25raw receipts.
Compound assignments, general consteval and full Rust remain open.

### Builtin compound assignment checkpoint

`COMPOUND-ASSIGNMENTS.md` is implemented for the ten primitive operators in
statement contexts. RHS snapshots precede current place/carrier/index selection,
old initialized scalar reads and same-address writes. Dynamic IndexAssignOp
preserves one checked address across division/multiply/shift kernels. Static
updates reuse existing Binary read/write phases, and compound counts share the
same per-function bounded diagnostic evaluator as ordinary shifts. All1191
hosted tests, strict all-target lint, exact32 stock/native plus four graph gates
and the actual real native example pass. Two unchanged historical boundaries
were promoted after native execution. Independent25corpus receipts pass900cases,
including832normal worker comparisons,70declared trap runs,128generated exit
sets and actual96worker example execution, retained in26raw receipts. Overloaded
assignments, full Rust and performance acceptance stay open.

### Shared byte slice (`&[u8]`) native lowering checkpoint

`ArrayToSlice`, `SliceLen` and `SliceIndex` lower through CFG validation,
ownership/borrow analysis and the native backend for nongeneric shared `&[u8]`
parameters and locals only. The slice handle inherits the source reference's
origins, and indexing reads the referent. The native carrier is two words
(data, len), passed as two argument words, and index >= len traps before the load.
The bounded slice-contract runtime runner passes with stock and native output
matching (positive plus three bounds-trap fixtures, workers 1 and 8). Adapted Adler2
acceptance (`--skip ff_44417`) passes 15 of 16 vectors natively at workers 1 and 8,
exactly matching the upstream crate and zlib checksums. These include the 5551..5553 and
22207..22209 NMAX boundaries and all three split vectors. That needed the literal array limit
raised from 4096 to 65535 and saturating recipe footprints, so oversized shapes become
inventoried intrinsic limits instead of hard errors. The 44417-byte vector is
blocked by an ownership analysis resource limit (per-element move paths for
large scalar arrays). Slice returns, `&mut [T]`, non-u8 elements,
range indexing, `for` loops and slice patterns remain unsupported. This is not
unchanged Adler2 acceptance or full Rust support.

### Configured source preparation checkpoint

`SOURCE-PREPARATION.md` records the delivered bounded source envelope: shared
text/file SOURCE facts, cfg-before-child-IO, explicit attribute policy,
original syntax locations and resource/ordinary-stack guards. Hosted all-target
check, 1731 tests in 90 binaries and strict lint pass, alongside 86 stock
fixtures and 344 native worker/mode cells. Full syntax/macros, semantic spans,
library roots, core/alloc/std and full Rust acceptance remain open.

### Source-locked identity runtime checkpoint

`CORE-IDENTITY.md` delivers one unchanged attributed function plus authored
clients, strict full-file/range provenance and scoped inert metadata. All 1758
hosted tests in 93 binaries, strict lint, 72 native identity cells and 344 source
configuration cells pass. Exact-source stock internal-attribute rejections are
recorded separately from installed-core behavioral references. The first native
64/72 and first full 1755/1758 failures remain retained: root-borrow replay was
repaired without demoting valid clients or existing nested-reference fixtures.
Constness does not imply CTFE. Full library crates, macro expansion and full
Rust remain open.

### Next source-expansion contract, not implemented support

`MACRO-EXPANSION.md` decomposes the next G1/G5 work into an immutable frontend
cursor migration, session-qualified tokens/trees/provenance/hygiene, bounded pure
macro matching/transcription and graph-visible ordered discovery. The design
preserves configured-source/no-child-IO and semantic publication boundaries.
Its proposed APIs, caps and stock matrix are a work contract, not macro, core,
CTFE or full Rust acceptance. Freeze each implementation lane before editing
shared files and close the actual native gates, not only token fixture tests.

### Immutable production parser cursor checkpoint

`CURSOR-MIGRATION.md` delivers T0a over the existing lexer and semantic pipeline.
Type-only splits and array-count restoration no longer mutate the token buffer.
The complete hosted suite passes 1766 tests in 93 binaries, with strict lint,
formatting and independent bounded review. Standalone token/session/tree/budget
implementation is a subsequent lane. No lexer replacement, hygienic identifier
resolution or native macro expansion is claimed by the cursor migration.

### Checked structural token utility checkpoint

`TOKEN-PRIMITIVES.md` delivers immutable, owner-qualified transport and flat
grouping behind an explicit public utility facade. Shared expansion leases reuse
extracted reservation telemetry without adopting generic planning caps. Public
source imports enforce the existing file/original-byte ceilings before scans.
The complete hosted suite passes 1828 tests in 94 binaries, with public utility,
foreign-handle, resource, original planning and strict-lint gates. Native lexical
tokens, prepared-source projection, hygienic AST identifiers, matching and graph
expansion remain open. Structural tests are not language-support acceptance.

## Native lexical transport checkpoint (2026-10-17)

The production parser now consumes the native lexer through a budgeted immutable
adapter. Original files are lexically validated with that same scanner before
configuration and child filesystem discovery. Exact literal spellings, suffixes,
punctuation adjacency, doc audit records and original/synthetic EOF provenance
are retained. Source/session/budget owner checks and whole logical-token mapping
reject invalid transport independently of source syntax. Unicode XID uses the
pinned minimal Unicode-17 utility and retained notices, not compiler internals.

Fifty-four stock witnesses run under the actual hosted stock compiler at both
editions 2024 and 2021, with lexical validity separated from expression suffix
semantics. Native facade checks, original-location/CRLF controls, late lexical
errors before projection failures, and workers1/8 CHECK/BUILD graph tests pass.
The unchanged existing source86/native344 and identity18/native72/protocol64
gates remain passing. Initial quadratic interning and logical-view storage
regressions were repaired without increasing caps or reducing source fixtures.

The final hosted all-target check, all 1887 tests across 96 binaries, strict
all-target lint and formatting pass. Twelve successful/failed redacted receipts
are retained in `data/runs/xps14/2026-10-17-native-lexer.jsonl`; the complete
test receipt is `20261017-064506676-590807`. See NATIVE-LEXER.md for phase-specific
corrections and resource evidence. Hygienic semantic identifiers, macro matching,
transcription, expansion/discovery closure, full literal semantics, core/alloc/std
and full Rust compatibility remain open. No performance speedup is claimed.
