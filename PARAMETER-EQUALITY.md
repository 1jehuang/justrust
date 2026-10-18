# Parameter-valued associated equality

Status: implemented and validated for the bounded scope below. Builds on accepted `bc428d8aa` and ASSOCIATED-EQUALITY.md. Full Rust remains open.

## Bounded vertical

Add a bare, authoritative caller type parameter as equality RHS: `T: Source<Out = U>`. Keep existing closed RHS forms. Preserve the exact `ParamId` including owner and index through source lowering, staged/final predicates, normalization and substitution. Do not infer U from a selected implementation or equate unrelated parameters. Parameter-containing RHS tuples/arrays/nominals and projected RHSs remain separate work; an actual substituted U may be an already-admitted concrete aggregate.

The target includes universally checked `get<T,U>(x:T)->U where T:Source<Out=U>`, not merely an explicit U-input passthrough. Ordinary bodies may assemble returned U into existing tuples/arrays/nominals. Non-Copy U moves once; duplication requires actual U:Copy. No scalar whitelist or invented Copy permission.

## Raw validation and contextual proof

Split raw equality metadata/domain-shape validation from contextual Sized proof. Raw validation visits every projection/RHS handle, member owner and binder before capability restrictions, and accepts only existing closed shapes or a bare member of the exact env. It must not recursively invoke the solver while declared environments are being constructed.

After actual inherited/raw rows, implicit Sized and membership closure exist, validate bare U's Sized requirement against that exact private declared context. Final queries and substitutions use their real ProofContext and active budget. Never prove symbolic U:Sized using empty Closed facts. Trait Self does not acquire implicit Sized merely by being a trait-owned parameter.

Closed left-hand application plus rigid RHS remains explicitly Unsupported in this bounded stage, after metadata checks. No normalization of a closed assignment may solve a caller parameter. Fully closed equality continues to compare the exact selected assignment. Rigid LHS plus declared U equality uses the exact caller row and its membership evidence.

## Explicit restricted-result domain authority

Keeping a U ParamTail row with no sources is not enough. Existing carrier derivation matches ParamTail sources only by identical ParamId; `T -> U` cannot invent a T-to-U family mapping. Existing call flow unions only listed origins, so an unexplained source-free U result would otherwise pass vacuously.

Introduce private, sealed restricted-result-domain evidence. Its meaning is narrow: every executable substitution of this exact raw equality-dependent result is checked to have a recursively reference-free associated normal form before execution. It is NOT a global proof that U has no carriers, a Rust lifetime relation, or a substitute for Copy/Sized evidence. Retain the U ParamTail in universal IR, signatures, origin mappings and carrier views.

Bind each certificate to the real world, declaration/call identity, exact env and phase, stage/final epoch where relevant, full ProjectionEq row, real RHS ParamId, normalized signature, exact result carrier location and normalization dependencies. A required trait-call view uses the required declaration's original source regions, never the selected implementation's topology. A direct generic-call view must obtain the corresponding certificate from its immutable original equality requirements as well: normalization may have erased every projection from the signature. Body-only and unused equalities remain obligations.

Before accepting a source-free symbolic result row, call-flow consumption must explicitly validate this certificate through the already checked call/view authority. Matching an output ParamId by spelling, finding any equality elsewhere, or merely finding no source row is insufficient. Ordinary non-equality paths retain their existing behavior. Forwarding retains original equality discharge or genuine value-origin transfer; no silent dropping of constraints at direct-call, instance or body-local boundaries.

Every closed substitution revalidates exact equality, Sized and recursive reference-free normal form, including nominal fields and zero-length metadata children. Reference-bearing U and reference-carrying nominal U remain Unsupported even if Copy. Unrelated reference actuals must not be globally prohibited. Public IR/CFG/native entry points cannot bypass these guards. A retained domain certificate must fail replay if its row, ParamId, result location, full trait args, source metadata, world/env/phase or dependencies change.

## Source and resource invariants

Existing parser binding vectors already retain U as a real source type occurrence. Preserve all source/callback provenance, unused-row validation, duplicate handling and exact source replay. Conflicting U/V equalities remain explicit Unsupported rather than selecting a convenient RHS. Raw signature callbacks stay immutable; no world normalization under the PlanState mutex. Original sticky resource ledgers and bounded work/depth/capacity accounting apply to certificate creation, comparisons and replay. Keep construction/final memo namespaces separate.

## Evidence and release gates

Stock-only receipt: `parameter-rhs-bc428d8aa-oracle/oracle.json`, SHA256 `f4da57c52ed0930976109226f90a8043cf1583168489ed50daada7d2830659a2`, 14 original sources, eight runtime successes and six compile failures. Manifest SHA256 `4feac27d1e01d7acedff75fd39a836b5f81eac42f50c5ae830ea2341bf4d07c8`. The original positives contain panic branches unsupported by the current parser; preserve them and measure separate positive-only equivalents before native expectations. Composite RHS supplement is evidence only, not admitted by this contract.

## Validation evidence

The stock receipts retain the original 30 sources, including their active `allow` attribute boundary. Fifteen separately measured sources remove only that outer attribute, without changing their bodies or payload checks. `tests/parameter_equality.rs` preserves all 45 sources and checks workers 1/8 in CHECK/BUILD: 180 CLI cells and 180 all-body graph cells, including 12 real native payload executions per build profile. The partition is six Runtime, six ordinary Invalid, 21 stock-valid Unsupported and 12 stock-invalid Unsupported. Earlier parser/attribute boundaries are not claimed as equality-semantic coverage.

Sixteen public authority tests and twelve private parameter-equality tests cover real sealed facts, missing or altered certificates, exact source/binder replay, empty and nonempty result-source unions, body/direct/required subjects, non-Copy versus Copy, nested and zero-length reference metadata, fixed/origin/region transport, sticky failures and allocation accounting. The historical 46-source associated-equality corpus keeps every source byte unchanged and promotes only its now-supported parameter-RHS case 37 to Runtime.

The complete hosted suite passes 2,969 tests across 144 binaries, with all-target check, strict Clippy, formatting and diff checks. Run `20261017-173121336-3832748` records that full suite. The unchanged growing-generic profile also reaches its original depth boundary after removing a redundant metadata scan following complete predicate validation, not by raising a budget. Debug/release and failed-then-repaired runs are retained in `data/runs/xps14/2026-10-18-parameter-equality.jsonl`.

This is bounded parameter equality, not general equality solving, reference-bearing associated values, associated bounds/defaults, parameter-containing RHS syntax or complete Rust support. No compile-speed claim follows from these correctness gates.
