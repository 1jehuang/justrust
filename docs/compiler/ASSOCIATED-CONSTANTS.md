# Associated constants

Status: implementation in progress. This does not claim complete Rust constant evaluation or trait support.

## Frozen external contract

Trait declarations may require `const NAME: Ty;` or provide a default initializer. Trait implementations may assign a matching member, and inherent implementations may declare constants. Members retain their original source identifiers, declaring owner, and ordered generic binders. Duplicate value members, unknown members, missing required members, and incompatible member types are rejected even in unused implementations. Default selection never overrides an explicit implementation assignment.

`T::NAME`, `Self::NAME`, and `<T as Trait<Args>>::NAME` resolve using canonical declaration identity and the caller's actual trait obligations. A type projection in the member's type is not a value projection. Generic references remain typed symbolic references until the actual implementation and substitutions are known. A successful supported program must match stock rustc's output.

Initializers are constants, not runtime getter bodies. Executable uses consume immutable typed constant-value facts through engine handles. Evaluation failures and cycles cannot publish successful executables. Unsupported initializer forms receive an explicit diagnostic rather than an approximate runtime translation. Existing CTFE limitations remain explicit and are not silently expanded by parsing new syntax.

## Graph and IR shape

Source-owned constant declarations and references preserve original name/type/expression handles. A resolved reference contains the declaring member identity, canonical trait application or inherent owner, and ordered substitutions. Universal checking validates member membership and type obligations without inventing a concrete value. Closed evaluation is keyed by the selected constant declaration and canonical actual arguments. Dependent initializers request their independent fact prerequisites before awaiting them. No whole-program node, mutable fact payload, external result slot, or runtime replacement is permitted.

## Acceptance gates

- Unchanged time-core 0.1.2 `convert.rs`, including `Per<T>::VALUE: Self::Output` and `Self::VALUE` inside generic `const fn per<T>`.
- Stock-differential CHECK/BUILD/run at workers 1 and 8 for required members, defaults, overrides, inherent members, qualified paths, generic callers and dependent values.
- Stock controls for unused type mismatch, missing/extra/duplicate members, invalid defaults, ambiguous references and evaluated constant cycles.
- Forged foreign member/type/world references fail source and semantic authority checks.
- CHECK demands no code generation; graph-integrity ratchets pass without increasing baselines.

## Current implementation and observed acceptance

The scalar draft passes 23 stock-differential cases in `tests/associated_constants.rs`: eleven declaration/default/generic cases, four inherent-owner cases, and eight initializer-dependency cases. Its local driver additionally executes native CHECK and BUILD/run at both workers 1 and 8. CHECK runs without `-o` and verifies that its isolated directory gains no artifacts. This is not merely the shared oracle helper's default-worker BUILD/run coverage.

Operations 134, 135 and 136 publish immutable header, original-use selection and scalar value facts. Selection publishes its source authorization early, then separately publishes qualification of every original initializer member reference before value evaluation. This includes references skipped by boolean short circuiting: stock rejects `false && Self::CYCLE` when that reference forms a cycle. Independent dependency requests precede awaits. Cycle identity uses the selected declaration and ordered actual types, while dependency keys retain the original caller occurrence and initializer coordinates. An overriding implementation never demands an unrelated cyclic default.

Universal bodies retain typed symbolic operands without evaluating values. Closed bodies read exact graph value facts and replay source/type authorization before replacing operands. Caller identity is the canonical graph key: declaration plus ordered structural type arguments, not transient nominal realization IDs. Reconstructed equivalent callers share a computation, while reordered arguments, another declaration, a foreign snapshot, or another original occurrence remain distinct. Initializer inputs are typed original-node inputs, not fabricated declarations or runtime getter calls. Supported evaluated values are the existing fixed-width integer/bool scalar CTFE forms. General const-fn execution, arbitrary non-scalar values and free-constant initializers containing associated-member references remain outside this implementation. Implicit member lookup requires retained caller obligations; general lexical trait-scope lookup is not claimed.

The canonical-caller regression passes the existing 11-case declaration/value matrix, including two nominal instantiations of `Self::VALUE`, with CHECK/BUILD/run at workers 1 and 8 against the recorded rustc 1.101.0-nightly oracle. The nominal argument-order/declaration/foreign-snapshot regression, original-site authority controls, strict all-target lint and all 14 graph-integrity gates also pass. Exported runs are in `data/runs/rust-support-followup-20261020/associated-caller.jsonl`. This is not a full associated-constant or Rust compatibility claim.

The unchanged time-core gate is still open. The registry owner's latest unchanged-source probe stops earlier at `lib.rs:6:4` with unsupported non-string `doc` metadata. No cached dependency source was rewritten, and generic `const fn per<T>` execution is not claimed from the focused `Self::VALUE` cases. Graph-integrity and scalar-input tests pass without stack or ratchet changes. This milestone remains in progress until its remaining external acceptance boundaries are implemented.

The source dependency may reveal additional compiler boundaries. Reaching one is reported as blocked, not as successful associated-constant support.
