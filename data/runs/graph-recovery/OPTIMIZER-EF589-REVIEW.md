# Independent optimizer accounting follow-up review

Frozen private checkpoint: `ef5890d918141f4d522d6b744aa15fbe98edead8`, parent `7f9ebcbe13e95faf42b08f1e28b32660934f4032`. This checkpoint is NOT imported by this document. Main remains on the default-off optimizer policy. Findings below are source inspection by Hare, Nautilus and the coordinator, not independently executed tests. Owner-reported hosted tests are not acceptance of these failure paths.

## Confirmed blockers

1. **Payload lifetime on healthy error paths.** `Owned::seal` evaluates a tuple containing the moved payload and consuming `Lease::seal(meter)?`. Authority, sticky-state or work failure inside the latter drops/refunds the lease before the payload temporary is destroyed. `Owned::combine` has the same problem when consuming `Lease::absorb` fails while its incoming payload is live. At the end of machine emission, raw code/calls/strings vectors are extracted before fallible absorbing of their leases, creating equivalent failure-order exposure. Repairs must keep each payload coupled to its admission through every fallible operation, return unchanged leases on precommit failure, and destroy payloads before refunding. Drop-observing tests must cover foreign authority, sticky state, tight work, incoming-combine failure and machine merge failure.
2. **Type-comparison setup work.** The iterative CallTarget comparison allocates and pushes all N argument pairs before comparing, but charges popped visits rather than all setup pushes. An early mismatch after that setup costs O(N) initialization without corresponding work. Charge actual setup and child pushes as well as visits, with shared-prefix candidate scaling tests.
3. **Kept-callee copy work.** Inline construction counts retained symbol and parameter storage in byte preflight but copies non-inlined callee symbol bytes and parameter entries without corresponding work admission. Add copy-time work and long-symbol scaling tests.
4. **Liveness literal visits.** The flat liveness matrices and fixed-point dimensions are paid, but some literal clearing/comparison/copy visits remain outside the work count. This is a constant-factor accounting omission, not evidence of a missing fixed-point dimension or a miscompile.

## Improvements accepted by inspection

Buf now checks the paying authority and sticky state before fast-path mutation, and grows through an independent replacement committed after admission. Chunk totals are cached, shrink removes exhausted tail chunks, and chunk movement/retention work is explicit. The prior independent 4096 type-comparison depth cap is gone in favor of caller depth. These observations do not waive the remaining lifetime and work blockers.

## Important correction: failed PlanReservation retention

At this exact checkpoint, `PlanReservation::retain` can fail only when locking the non-metered usage ledger returns poison. Its destructor retries that same poisoned mutex and refunds only on successful locking. There is no poison-clearing path in the inspected repository. Thus the currently failing chunk and the remaining chunks on that ledger stay conservatively charged. The metered retain path is infallible. This differs from ByteLease retention, whose error path can refund a healthy ledger.

A mid-retain error is therefore NOT demonstrated current-chunk underpayment. No shared IR promotion API expansion is required on that evidence. Preserve this precise invariant and test the observed payload-drop accounting under poisoned retention. The confirmed healthy precheck/work/absorb errors above still require repair.

## Scope and claims

Keep the frozen checkpoint intact and place repairs in a separate private commit. Do not raise limits, introduce independent source ceilings, activate the optimizer by default, or import the accounting stack before independent re-review. Ledger-reserved old/new peak accounting is not proof that allocator-rounded transient physical surplus was prepaid. Bookkeeping bytes must be included separately; loose upper-bound tests are not exact physical-peak measurements. Error-message String allocations remain explicit diagnostic-allocation debt, not paid merely because the same failure convention was already used.
