# Generic tuple structs: existing nominal and universal authority

Status: pending implementation acceptance. Full Rust compatibility remains open.

## Frozen slice

Admit original nonempty `StructShape::Tuple` declarations with the existing supported type parameters, bounds and actual types. Retain real contiguous `FieldName::Tuple(index)` source identities, declaration visibility and every constructor-field restriction. Generic unit and zero-field tuple structs, first-class constructors, numeric brace construction, const/lifetime parameter features and reference carrier forms outside the existing contract remain closed.

No new source AST, checked IR, constructor callable identity, ABI or general inference rule is introduced. Explicit applications, expected-type evidence and already-supported bounded first-order constructor inference must pass the same nominal application and universal checking machinery as existing generic records.

## Exact nominal application

The checked original source DeclId and parameter environment authorize each application. Concrete StructTy retains exact NominalAppId, source declaration and type arguments. Field types are substituted from that exact source schema, not selected by spelling, runtime field count or a favorable concrete instantiation.

Resolver constructor and pattern rewriting must retain original-use GenericPath certificates and actual explicit argument provenance. Import aliases may select the real constructor subject to visibility. Type aliases do not become callable constructors. Constructor expressions never acquire a source callable or OriginalBodyId.

Generic function bodies are checked universally with rigid parameters before concrete use. An unused or otherwise favorable-instantiation-only invalid generic body remains invalid. This milestone must not bypass obligations, rigid type equality, nominal bounds or resource admission.

## Patterns and moves

Let, match, if-let, while-let, let-else and positional-rest plans consume the exact instantiated source field types. Generic tuple-struct plans use ordinary Field projections, never enum Downcast fiction. Genuine source binders and inventory occurrence order remain unchanged. Existing selected-only moves, omitted-field nonconsumption, constructor privacy and non-Copy indexed extraction fences remain intact.

Nested or unconstrained constructor evidence is admitted only where existing inference already supports it. No printed-name fallback, inferred nominal shape, ghost binder or erased application argument is permitted.

## Acceptance gates

Require stock-first exact source witnesses for explicit/inferred/expected constructors, generic wrapper functions, mixed type parameters, instantiated patterns and rest, imported constructors, macro hygiene, field privacy and partial moves. Include wrong arity/arguments/bounds, same-shaped different applications, unused universally invalid bodies and type-alias constructor negatives.

Public controls must use real generic declarations, parameter IDs, canonical application facts and checked signatures. Foreign source handles, malformed numeric schema, wrong application or field type and local/projection authority must reject at the actual appropriate IR/CFG/ownership boundary, without fabricated nominal IDs.

Native worker1/8 CHECK/BUILD cells, executable ELF output with exact exit/stdout, bootstrap debug and release profiles, strict all-target lint, formatting, unchanged full regressions and retained passing/failing run receipts are mandatory. Preserve historical source bytes and promote only exact newly admitted witnesses. No speedup, public ABI or full Rust claim follows from this slice.
