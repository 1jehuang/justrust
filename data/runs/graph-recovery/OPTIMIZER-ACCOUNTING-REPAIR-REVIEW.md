# Optimizer accounting repair checkpoint: A-H review

Read-only source review on 2026-10-19 of private checkpoint `67e1906364533fbd5c5c8f2968d5ade3dcded359` (baseline `368101dd18d73208a7cc4da4a8faaccd6f07fe03`; preceding `11fc496eae1c6db66da95606d5981934a6df76f5` concerns optimizer graph tests/kind names). All line references below are to `67e190636`. Commit identities were resolved with git rev-parse. No tests executed for this review. This documents blockers, not fixes or accounting acceptance. Broader SSA algorithm review is separately owned by nautilus.

## A. Nested callee capacities measured as lengths

**Site:** `src/compiler/ssa/store.rs:121-133`, specifically `125`.

`lowered_capacity_bytes` calls `callee_bytes(&lowered.callees, false)`. That measures nested symbol and signature-params allocations using length, despite this function promising actual capacity. Adding spare outer callee slots at `126-131` does not repair nested surplus capacity.

**Bounded fix:** use capacity mode for the nested callee measurement, retaining separate outer spare-slot calculation. Audit all callers against the corrected total, without a budget increase.

**Regression:** create a callee with large spare String capacity and signature Vec capacity but small lengths. Compare measured bytes with independent checked sum of capacities. Do not compare two outputs from the same faulty sizing function as the only oracle.

## B. Machine code and trap buffers can grow after admission

**Sites:** `ssa/machine.rs:79-80`, `103`, `689-694`, `913-919`, `995-996`, `1014-1026`.

`Asm::bytes` uses unrestricted `extend_from_slice`; trap insertion uses raw `push`. The final `code.len() > code.capacity()` check is always false for a valid Vec and cannot detect reallocation. Panic expansion grows code in place, admitting only needed-minus-current-capacity, not a full replacement while the original allocation remains live. The allocator's returned capacity is not immediately reconciled. Panic insertion can consume remaining normal-emission headroom, after which later ordinary instructions/trap append can auto-grow without admission. `reserve` returns actual allocator capacity without checking it against paid request, and `bounded_push` bounds against actual capacity rather than an independently authenticated admitted limit.

**Bounded fix:** fallible bounded code/trap append, with an admitted capacity independent of unchecked Vec growth. For panic expansion, prepay a full independent replacement while old storage remains paid, validate actual capacity before moving/copying, and preserve remaining normal-emission allowance. Reconcile every returned capacity before use or fail closed and discard the temporary.

**Regressions:** explicit panic block followed by substantial reachable code/trap emission; injected/constructed overcapacity buffers; exact-small byte budget; error must precede successful output publication. Assert no unrestricted append can cross its admitted capacity.

## C. Final Owned reconciliation cannot prove construction peak

**Sites:** `ssa/store.rs:148-158`, `177-184`, machine paths in B.

`Owned::build` and `Owned::adopt` reconcile after construction. They cannot retroactively authorize an allocation or reallocation that exceeded its admitted preflight bound while being built. B gives a concrete path, not just a hypothetical generic closure.

**Bounded fix:** require every builder passed to these wrappers to use admitted fallible bounded allocation/growth internally. Keep final reconciliation as a validation step, not permission to grow freely. Distinguish conservative requested-capacity preflight followed by fail-closed allocator-capacity reconciliation from a claim that exact physical peak was prepaid.

**Regressions:** nested growth/relocation with simultaneous old and new buffers, allocation failure before commit, and pointer/capacity/receipt preservation on failure.

## D. Lease authority laundering

**Sites:** `ssa/meter.rs:142-178`, including no-op branches `155-156`, `164-165`, `173-174`.

`regrow`, `resize`, `admit` and `absorb` accept a Meter without proving it has the same PlanAuthority as the existing reservation or absorbed lease. A lease originally paid by A can be resized under B, refunding A and eventually sealing a B receipt. Equal policy is not equal authority. No-op branches also skip authority validation.

**Bounded fix:** authenticate self, absorbed lease and meter under exact authority before mutation, refund, merge or no-op success. Keep immutable authority evidence in the lease as needed, never a new published mutable ledger. Do not reinterpret bytes paid in another task/authority as valid credit.

**Regressions:** equal-policy distinct-budget resize/admit/absorb including zero and equal-size cases. Rejection must not mutate original lease byte count/authority or silently refund/transfer its charge.

## E. Sticky failure is not enforced by Meter/seal

**Sites:** `ssa/meter.rs:51-65`, `154-186`; `ir.rs:1937-1963`, `1966-1976` at this checkpoint.

Meter work/lease call PlanBudget directly without checking recorded resource failure. The underlying direct ledger charge records rejection but can accept a later smaller or zero charge. Lease seal calls retain without a recorded-failure guard. Thus a caught huge failed request can be followed by a successful small lease/work/seal despite the documented sticky contract. Zero/no-op lease methods bypass checks as well.

**Bounded fix:** explicit healthy-ledger checks at entry and seal, including no-op paths. Preserve allocation/infrastructure failure classification rather than manufacturing a resource failure using fake charges. Mutable checks stay task-local and never enter retained receipts.

**Regressions:** reject huge work/bytes, then try zero work, small work, small lease, equal-size resize, zero admit, zero absorb and seal. Preserve the first failure and reject all successful publication paths.

## F. Unsupported and summary publication envelopes unpaid

**Sites:** `compiler/mod.rs:255-262`, `1081`, `1091`, `1107-1108`, `1113-1117`; `engine.rs:841-842`.

Unsupported SsaFact says the engine pays its envelope, but Context::publish_fact simply allocates Arc::new and publishes without byte admission. Supported SsaFact publish/share do explicitly pay their envelopes, so this convention is inconsistent. SUMMARY retains only sizeof<Option<Summary>>, omitting its Arc header and immutable receipt ownership.

**Bounded fix:** settle engine-versus-producer envelope ownership explicitly and charge exactly once. Unsupported facts and None summaries still need admitted publication storage. Summary consumers cloning an Arc must not duplicate the underlying summary charge.

**Regressions:** zero/tiny-byte unsupported SSA, LOCAL_OPT and IPO_OPT paths, plus Some/None SUMMARY publication. Verify exact envelope count and immutable receipt without charging shared input body again.

## G. Published Object and Receipt wrappers omitted

**Sites:** `compiler/mod.rs:1054-1056`; `ssa/machine.rs:1043-1056`.

Object sizing counts code/call/string table capacities and call names but omits Object value plus engine Arc envelope. `Arc<Receipt>` is allocated after object sealing without admission for its header/value. Receipt metadata is itself retained storage and must be covered. Direct Receipt contains immutable RetainedStorage only, no mutable backreference was seen.

**Bounded fix:** prepay complete owned wrappers before sealing, including receipt allocation and object publication envelope, with exact authority. Do not double-charge bodies shared by Arc or fields already included inline in Object.

**Regressions:** independent complete capacity/envelope sum, final receipt authority, zero-byte wrapper failure, and output remaining valid after the original task-local ledger is destroyed.

**Additional allocation-before-admission site:** `machine.rs:879-881` allocates a symbol using try_string before admitting its capacity. Prepay requested string bytes, allocate fallibly, reconcile returned capacity, then copy/fill. Map this regression alongside G and B.

## H. Panic callback scratch and payload provenance

**Sites:** `backend.rs:1885-1886`, `1907-1922`, `1940-1946`, `1979-1992`; `ssa/machine.rs:903-922`, `1040-1056`.

PanicAsm code grows using unrestricted extend; panic_object returns Object with receipt None. The known production panic producer admits diagnostic payload capacity through retain_bytes, and relocation table growth through a delta-only path, but code buffer allocation and relocation overlap are not fully admitted. Machine excludes moved diagnostic payloads from its final receipt because it trusts the callback paid them. Callback type Fn -> Object carries no exact authority or payload accounting evidence. Production callback identity is known, so this is not a claim that its diagnostic bytes are wholly unpaid. The public helper interface nevertheless permits foreign/unpaid callback payloads, and callback scratch code remains a real gap.

**Bounded fix:** scoped paid panic adapter returning Owned storage and immutable payload receipt under the exact task meter. Move diagnostic payload plus evidence into final Object without charging moved bytes twice. Admit code copy separately and refund callback-only scratch. Keep legacy stack emitter accounting separate rather than implying the new receipt covers it.

**Regressions:** foreign/unpaid callback object rejection; exact callback authority; moved payload evidence survives callback drop; copied code overlap accounted; callback failure refunds temporary storage; source panic behavior unchanged.

## Conservative bookkeeping caveat, not permission to raise limits

`Lease::regrow:142-158` and `absorb:172-178` reserve a new combined ledger total before dropping previous charges even for pure bookkeeping merges or shrinkage with no new physical allocation. That creates conservative phantom peaks. It must not be described as exact physical overlap. Prefer authority-preserving chunk/lease transfer where possible, or document the conservative admission bound. Never compensate with a cap or graph-ratchet increase.

## Review tracking and completion criteria

1. Fix A, D, E first: sizing oracle, authority and sticky contract are shared foundations.
2. Fix producer envelope ownership F/G and preallocation ordering.
3. Fix B/C machine growth with real bounded helpers and temporary replacement.
4. Fix H paid callback transfer, retaining exact moved-payload provenance.
5. Run scoped regressions mapped above, independent accounting oracles, unchanged graph gates and strict lint under coordinator's build window. Broad SSA algorithm review remains separate.

This audit changed only this private Markdown receipt. No source edits, builds, cleanup, push, installation, gate/budget/debt changes or agent spawning were performed.
