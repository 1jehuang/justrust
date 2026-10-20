# Explicit loan renewal

Status: implemented and stock-differentially validated for the contract below.
This does not complete Rust lifetime or arbitrary loop-carried reference support.

## Contract

A static explicit `Borrow` loan identity may execute again across an SCC edge
only when every reachable path to its next issuance first reaches a solved point
where the previous dynamic loan is dead. Liveness includes transitive reborrow
dependencies. A missing point or still-live rebirth fails closed. Returned-carrier
origins retain the existing explicit Unsupported boundary.

The proof walks existing per-function CFG edges. It does not remove loans,
carrier origins, targets or authority, and does not exempt all cyclic loans.
Visited blocks represent the single proposition that the old issuance has not
yet been proved dead. A cycle with no further issuance cannot create a second
loan. Work and temporary storage use the existing per-task budget. No new graph
root, mutable fact, shared budget or cache is introduced.

## Acceptance

`tests/loan_renewal.rs` compares native CHECK/BUILD/run with stock Rust for
borrowed guard retry and nested continue/break paths, with an escaped shared-loan
invalid control. The byte-identical borrowed guard fixture is also exercised
at workers 1 and 8 in `tests/guarded_or_patterns.rs`. Existing graph integrity and
all-target compilation remain required. Conditional origin correlation is a
separate change governed by `LOAN-CORRELATION.md`.
