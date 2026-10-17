# Generic tuple structs: existing nominal and universal authority

Status: accepted bounded native implementation on October 17, 2026. Full Rust compatibility remains open.

## Frozen slice

Admit original nonempty `StructShape::Tuple` declarations with the existing supported type parameters, bounds and actual types. Retain real contiguous `FieldName::Tuple(index)` source identities, declaration visibility and every constructor-field restriction. Generic unit and zero-field tuple structs, first-class constructors, numeric brace construction, const/lifetime parameter features and reference carrier forms outside the existing contract remain closed.

No new source AST, checked IR, constructor callable identity, ABI or general inference rule is introduced. Explicit applications, expected-type evidence and bounded first-order constructor inference must pass the same nominal application and universal checking machinery as existing generic records. Nested constructor evidence may compose existing nominal terms and field constraints without resolving numeric literal classes early. Nested generic function-return inference remains outside this slice.

## Exact nominal application

The checked original source DeclId and parameter environment authorize each application. Concrete StructTy retains exact NominalAppId, source declaration and type arguments. Field types are substituted from that exact source schema, not selected by spelling, runtime field count or a favorable concrete instantiation.

Resolver constructor and pattern rewriting must retain original-use GenericPath certificates and actual explicit argument provenance. Import aliases may select the real constructor subject to visibility. Type aliases do not become callable constructors. Constructor expressions never acquire a source callable or OriginalBodyId.

Generic function bodies are checked universally with rigid parameters before concrete use. An unused or otherwise favorable-instantiation-only invalid generic body remains invalid. This milestone must not bypass obligations, rigid type equality, nominal bounds or resource admission.

## Patterns and moves

Let, match, if-let, while-let, let-else and positional-rest plans consume the exact instantiated source field types. Generic tuple-struct plans use ordinary Field projections, never enum Downcast fiction. Genuine source binders and inventory occurrence order remain unchanged. Existing selected-only moves, omitted-field nonconsumption, constructor privacy and non-Copy indexed extraction fences remain intact.

Nested constructor evidence retains exact declaration-qualified nominal terms, rigid parameters and unresolved integer/float classes until the outer solver applies its normal contextual constraints and final fallback. Missing evidence remains unknown. Evidence collection is pure, depth/work/storage bounded and cannot evaluate a source expression, emit locals or move values. No printed-name fallback, inferred nominal shape, ghost binder or erased application argument is permitted.

## Observed acceptance

- Exact stock corpus: 32 sources, 22 valid and 10 invalid. Native partition: 18 Runtime, 10 Invalid and four valid Unsupported boundaries.
- Workers 1/8 CHECK/BUILD: 128 cells and 36 executed ELF images per bootstrap profile, repeated in debug and release. The retained 36-source tuple corpus preserves all source bytes/order/stdout and promotes only its generic constructor witness.
- Public application/schema authority: 23 tests. Structural IR permits constructor field payloads that are independently rejected at the existing CFG lowering or typed aggregate validation boundary. No broader structural-validator claim is made.
- Final targeted debug/release gates: 32 tests each. Library: 1,022 tests. Complete hosted regression: 2,586 tests across 128 binaries, with zero failed, ignored or filtered tests. Strict all-target lint, formatting and diff checks pass.
- Frozen native, application and universal-body witnesses pass without stock compiler fallback. Pure nested constructor evidence is checked for deferred numeric classes, later sibling constraints, repeated parameters, slot exclusion and no expression/local emission.
- Retained 22 actual failed and passing run summaries: `data/runs/xps14/2026-10-17-generic-tuple-structs.jsonl`. Final check `20261017-125300466-2260242`, debug `20261017-125301186-2260289`, release `20261017-125305524-2260699`, strict lint `20261017-125309130-2261124`, full `20261017-125309922-2260155`.

## Acceptance gates

Require stock-first exact source witnesses for explicit/inferred/expected constructors, generic wrapper functions, mixed type parameters, instantiated patterns and rest, imported constructors, macro hygiene, field privacy and partial moves. Include wrong arity/arguments/bounds, same-shaped different applications, unused universally invalid bodies and type-alias constructor negatives.

Public controls must use real generic declarations, parameter IDs, canonical application facts and checked signatures. Foreign source handles, malformed numeric schema, wrong application or field type and local/projection authority must reject at the actual appropriate IR/CFG/ownership boundary, without fabricated nominal IDs.

Native worker1/8 CHECK/BUILD cells, executable ELF output with exact exit/stdout, bootstrap debug and release profiles, strict all-target lint, formatting, unchanged full regressions and retained passing/failing run receipts are mandatory. Preserve historical source bytes and promote only exact newly admitted witnesses. No speedup, public ABI or full Rust claim follows from this slice.
