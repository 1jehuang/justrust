# Read-only optimizer accounting audit

Reviewed published `eb755d7f703243db724ea0cd0ec21b80d83a2c38`, optimizer introduction `1f6924cf6`, on 2026-10-19. Line numbers below refer to that published tree. Inspection only, no execution or speed claims. Existing conventions do not excuse new unbounded loops. No gate, cap or debt increase proposed.

## Priority 1: graph producer retained accounting

- `src/compiler/mod.rs:196-200`: `SsaFact` owns `Arc<Lowered>` with no immutable retained receipt. `SSA:963-988` charges blocks+locals work only. `lower` allocates nested SSA/callee/signature/string storage, boxes it, and publication creates another Arc allocation without byte admission.
- `LOCAL_OPT:991-1004`, helper `1194-1207`: optimization clones body and callee metadata without retained storage admission. `IPO_OPT:1081-1088` retains merged body and optimized result simultaneously, then allocates an Arc, without accounting peak or output.
- `SUMMARY:1006-1010`: `Summary::of` scans and hashes without work admission. Summary is Copy and does not own the body. Charge its owned fact/envelope once according to engine publication accounting, not once per consumer Arc clone. The engine envelope's precise accounting ownership must be resolved with its owner rather than double-counted.
- `OBJECT:947-954`: verification scratch is uncharged. Machine output is charged only as `object.code.len()` AFTER allocation. This excludes code capacity, calls/string tables and payloads, and transient peak. No immutable receipt accompanies newly retained optimizer output.

Repair: producer-owned fallible builders, checked cardinalities, admission before allocation, reconcile actual capacity before success, preserve old+new overlap during growth, and seal immutable `RetainedStorage` in the published owner. Never publish PlanBudget/PlanReservation or another mutable backreference. Shared input Arc clones do not duplicate underlying body storage. A post-hoc length tally is not a substitute for admission.

## Priority 2: node-local frontiers and lowering scratch

- `mod.rs:1029`: slots `with_capacity`.
- `1034-1040`: summary_handles collect.
- `1041`: summaries with_capacity.
- `1063-1069`: body_handles collect.
- `1070`: bodies with_capacity.
- `1075-1079`: pairs collect.
- `ssa/inline.rs:71-78`: chosen push.

All are uncharged allocations. Only `mod.rs:1031` charges one work unit per callee, not all scans/copies. Use held scratch leases, checked count times element size, fallible reservation and actual-capacity reconciliation. `scratch_vec:319-329` is an existing pattern but charges requested size only, so do not copy its missing actual-capacity reconciliation as a new exemption. Keep leases alive across awaits while buffers remain allocated.

`ssa/lower.rs`:
- `108,121,127,137`: types/params/blocks/environment allocation.
- `132`: clone all local types per CFG block; Builder::block creates local parameter and value tables per block. This scales blocks times locals, not blocks plus locals.
- `138-139`: another type clone and parameter position search per local, O(locals times params).
- `315`: environment vector copy per edge.
- `344,351`: call signature and argument tables.
- `361`: CallTarget deep clone.
- `364`: linear structural callee deduplication.
- `384`: panic string clones.
- `159`: verify scratch and traversal.

Repair: thread task-local budget through lower and Builder, preflight checked block/local/edge dimensions, budget each statement/operand/copy/lookup, use a local-index parameter map and metered canonical callee lookup. Preserve explicit unsupported outcome, distinct from resource/infrastructure failure.

## Priority 3: inlining work and growth

`ssa/inline.rs` has no budget parameter:
- `84`: clone caller; `85`: extra clone of panic table while original remains live.
- `117`: split_off continuation allocation; `124-128`: continuation params/block growth.
- `131`: rename map sized by callee value_bound.
- `142`: panic string clones; `143-185`: instruction/terminator/parameter clones and new blocks.
- `197,205`: used/remap arrays; `210`: deep callee clone.
- `93-108`: repeated instruction searches, each with body searches.
- `132-185`: per-callsite value/block/edge copying.
- `221`: final verifier allocates scratch and scans output.

`INLINE_LIMIT=24` constrains instruction count only. It does not bound blocks, params, value_bound, panic bytes, or cumulative caller expansion across sites. Preflight checked exact expansion dimensions and value/block ID arithmetic before allocation. Meter per-site and per-element work, hold scratch and retained-output peak, seal only final output receipt. Do not raise threshold/caps or add baseline exemptions.

## Priority 4: local pass fixed points and transitive helpers

`mod.rs:1198` charges inst_count+1 once, while `ssa/opt.rs:36-58` can perform I+B+2 whole-function rounds. Each round includes four canonicalizations, two verifications, folding, merging, parameter elimination and DCE, with final verification afterward.

- `opt.rs:68-70`: full clone and known/substitution arrays.
- `213-227`: edge/substitution/absorbed arrays and full clone.
- `280-328`: def_block, nested incoming Vec<Vec<Vec<Value>>>, uses and nested drop arrays.
- `439-449`: known/live/defining arrays and worklist.
- `72,222,362`: substitution chain loops.
- `290`: dominance walk.
- `333`: incoming argument scans.
- `459`: DCE worklist.

Meter actual rounds, nodes, edges, operands and chain hops with the existing task limit. Hold leases for simultaneously live current/next/canonical/verification buffers and release scratch on drop. Thread accounting through canonicalization, dominators and verify, not only their callers. Resolve signature cloning (`lower.rs:44`, `signature_of`) under the same contract.

Verifier-specific sites in `ssa/verify.rs`: dominators `38,42,51` allocate RPO-index, nested predecessor and idom arrays; `57-64` intersection chain walks and `68-85` fixed-point predecessor scans have no budget parameter. `verify:119-129` allocates entry types and value-bound definitions; `191-214` allocates reachable/seen/unreachable/pending tables; `223` runs pending type propagation to a fixed point; `248` allocates combined order; `294` collects final type table. All require scratch admission and actual traversal/round charges, including callback signature clones. Returning a type table transfers its lease to the consumer for its real lifetime rather than refunding at verifier return.

Crosscut `ssa.rs:258-286` operands/operands_mut and `317-365` targets/operand helpers allocate small Vecs on each traversal, with Call args cloned. Replace with borrowed slices/allocation-free iterators or visitors where practical. This avoids repeated tiny allocations in every analysis/hash/verify loop rather than hiding them behind coarse charges.

## Priority 5: machine allocation and emission

`ssa/machine.rs` has no budget input:
- `255-350`: uses/fused/start/def_pos/end tables; gen_sets/defs/live_in/live_out require at least four block_count times ceil(value_bound/64) word matrices, plus nested Vec headers and other scratch.
- `352-370`: liveness fixed point has no charged work limit and allocates fresh out and inn rows per block per round.
- `404-426`: candidate collection/sort and calls.any per candidate, O(values times calls) comparisons.
- `482-507`: parallel-move pending allocation and repeated searches/retains.
- `535-550`: code/traps/calls/strings, labels, edge and move tables.
- `719`: symbol clone; emission also incorporates panic-object storage from callback.

Repair: budget allocate/Asm/moves/emit, checked matrix dimensions and fallible buffers, reused leased scratch rows, metered fixed-point word/edge visits and comparisons. Capture emitted Object complete capacities and nested strings/call names before publication. Callback ownership must avoid double-charging moved panic buffers while admitting any copied bytes and relocation peaks.

## Bounded staged owner repair plan

1. Freeze output storage accounting contract and ownership. Add immutable receipts for new SSA/local/IPO outputs and complete object admission. Preserve sharing without duplicate body charge. No mutable budget inside facts.
2. Introduce narrow task-local checked/fallible allocation helpers. Fix frontier vectors and lowering/Builder growth first, including actual capacity and overlapping allocation lifetime.
3. Thread budget into Summary/hash/verify/canonical/local/inline with allocation-free operand visitors. Charge real nested traversal and fixed-point work, including poison checks on zero/small follow-up paths.
4. Meter machine tables, liveness, moves, emission, and final object storage. Reuse scratch where safe. Checked arithmetic failures remain explicit and sticky under existing contract.
5. Validate each stage independently at unchanged budgets and ratchets. Do not bundle unrelated activation or claim graph compliance from stock correctness alone.

## Proposed acceptance evidence

- Zero/tiny-byte rejection for SSA, LOCAL_OPT, SUMMARY, IPO_OPT and OBJECT producers before large allocation.
- Many-block/few-instruction, many-local/many-edge, many-inline-site and long-liveness fixtures to exercise non-instruction dimensions.
- Work exhaustion across fixed-point rounds, chain hops and hash/string scans, with sticky follow-up failures.
- Checked multiplication/addition and ID overflow, with no successful publication after resource/infrastructure failure.
- Exact retained authority and actual-capacity receipts, scratch refund on failure/success, immutable output outliving ledger, and reachable-fact tests proving no mutable backreference.
- Existing graph_integrity unchanged, scoped correctness and worker-count acceptance, no raised baseline.

No source edits, builds, cleanup, push, install, main changes, or agent spawning performed for this audit.
