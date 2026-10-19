# Target-keyed instance graph decomposition

Status: design only, not an implemented or measured improvement. Owner: bat. Baseline inspected: `3537c8fef`. Coordinate nominal/PlanCtx authority with llama and PARSE/RESOLVE with zebra before any source edits. Node numbers are intentionally unallocated.

## Actual remaining barriers

- `Facts::instance(index)` derives closed roots from SIGNATURES, but reads all of MONO_PLAN for every additional instance.
- MONO_PLAN reads every TEMPLATE_BODY, clones PlanCtx and runs `instances::build` over the entire reachable frontier. Its entry order supplies the generic BODY/LOWER/ANALYZE/OBJECT keys.
- `instances::build` combines target admission, source lookup, immediate successor discovery, recursion depth, canonical collision detection, sorting and link inventory in one mutable local computation.
- Both ANALYZE and OBJECT call `Plan::validate_cleanup_dependencies` through MONO_PLAN when the CFG has Drop. Thus early closed-root scheduling does not remove the plan dependency for Drop owners.
- OBJECT already derives relocation symbols with `symbol_of(target)` independently of inventory. Keep that property. BUILD must still prove that every relocation has a corresponding checked object before publishing an executable.

## First bounded immutable output

`InstanceSuccessors` is an immutable output for exactly one admitted closed target. It contains typed edges with reasons: explicit direct call, exact unsizing/vtable slot, owning Drop cleanup, and vtable Drop demand. It reads only its own admitted target, its own canonical checked body, the declaration/signature facts required for substitution, and relevant trait/normal-Drop facts. It must never await successors' `InstanceSuccessors`, concrete proofs, objects, or a completed link inventory.

Admission remains distinct: `InstanceAdmission` proves the target belongs to this snapshot, all type arguments are closed and well-formed, the declaration is callable, entry predicates hold, and canonical identity agrees with that exact target. A memo hit is not admission authority. Foreign declaration/type-owner handles fail before cache lookup. These checks currently appear in Queue::add and must not disappear when the queue is split.

The first independently useful slice is owner-local cleanup evidence: replace the two `has_drop -> facts.plan()` reads with a typed per-owner cleanup-demand fact. Validate every lowered cleanup descriptor against this owner's closed canonical Drop demand. Final BUILD still demands and checks the resulting cleanup objects. Do not simply delete the membership check or accept any declaration with matching spelling. This slice would remove a genuine Drop-owner barrier without pretending that generic instance indexing is already decomposed.

Preserve both anti-bypass controls in `instances.rs`: `normal_drop_demand_is_sealed_even_when_destructor_is_already_a_source_root` proves that a symbol in inventory alone is insufficient and that a missing executable target also fails; `generic_normal_drop_is_relowered_and_demands_its_destructor` proves instantiated cleanup cannot be removed or justified by stale template lowering authority. When splitting local evidence from terminal membership, move each assertion to its new owner rather than deleting it.

## Identity is the blocking design contract

Current engine keys contain `Vec<u64>`. Current target identity encoding is collision-free structural bytes, but it is not a typed decoder or authorization token. A hash-only key, process-global target registry, cloned mutable PlanCtx, or opaque numeric MONO_PLAN slot is not a solution.

Before activating target-keyed BODY/LOWER/ANALYZE/OBJECT, agree one authority-preserving path:

1. Engine-owned immutable typed key/input interning, with equality on the complete target representation and explicit reads of the producing fact, or
2. Complete structural target encoding plus a checked decoder that resolves declaration and nominal application components only through immutable graph facts.

The selected mechanism must preserve snapshot ownership, avoid using a symbol string as semantic authority, deduplicate equal targets produced by different callers, and charge key storage/decoding to the relevant task. Merely keying a node by `(caller, successor ordinal)` preserves provenance but duplicates recursive target computations and is not the final canonical instance identity. Llama's nominal-handle API must settle this contract before choosing either option.

## Cycle handling and completeness

Exact self recursion and mutual recursion are finite graph cycles in successor *data*, not await cycles. Publish immediate edges before following them. A terminal CHECK/BUILD root may walk a deduplicated frontier of graph handles; no compiler-wide registry may coordinate intermediate instances outside the graph. Independent nodes must not await this terminal traversal.

Changing-type recursion must retain an explicit well-founded admission policy. Existing depth129/limit128 behavior is a compatibility control, not permission to put path depth into canonical instance identity. Equal targets reached along different paths must not get different bodies or arbitrary first-writer budget authority. Decide and test path-depth bookkeeping in the terminal traversal separately from target-local validation.

CHECK additionally preserves universal validation of unused user bodies and applicable prelude/template obligations. Reachability of executable instances is not permission to skip an invalid unused generic body. BUILD waits on CHECK, gathers all required concrete objects, rejects missing relocation and cleanup targets, checks canonical symbol uniqueness, and only then publishes ELF bytes. Deterministic sorting belongs to terminal emission, never to admission/body availability.

## Acceptance map before implementation

| Requirement | Required observation |
|---|---|
| Owner-local cleanup replaces global plan read | Public check/build of a Drop owner with an unrelated expensive generic body; trace shows cleanup fact read and no MONO_PLAN dependency in owner's ANALYZE/OBJECT transitive closure. Existing invalid-cleanup controls still fail. |
| Equal targets share work | Two distinct callers reach one generic target; one admission/body/concrete-proof/object evaluation, workers1/8, exact stock output. |
| No await cycle | Self recursion and mutually recursive generic instances compile and execute with workers1/8; trace complete, no scheduler deadlock. |
| Changing-type recursion remains bounded | Original growing-type CLI profile still fails at requested129/limit128, no executable and useful failure attribution. |
| Hidden successors retained | Generic Drop, nested owning aggregates, dyn unsizing/vtable slot and vtable Drop execute with exact stock outputs. Every relocation resolves and every cleanup object is checked. |
| Unused validation retained | Existing parameter/associated equality, bounds and invalid-unused-body suites preserve rejection categories and no executable publication. |
| Authority retained | Forged/foreign declaration and nominal handles rejected, admission cannot be bypassed by a cached target, exact source publication checks preserved. |
| End-user interfaces unchanged | Real CLI check/build/output collision/profile/timeout tests plus full original-program corpus; supported outputs exact, unsupported inputs explicit rejection. |
| Graph rules retained | Both static and reachable-fact mutation ratchets, dependency/leaf growth, unchanged size and resource controls. No new whole-program kind and no ceiling increase. |
| Speed improvement demonstrated | Matched isolated pre/post native CLI artifacts and identical source/configuration, repeated paired timings with correctness controls. Report per-case regressions and scope; no general Cargo speed claim. |

All rows are future gates for this split, not results. The published baseline's correctness evidence is in `data/runs/graph-final-acceptance/TRACEABILITY.md`.
