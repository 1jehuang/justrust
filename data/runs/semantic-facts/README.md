# Trait fact immutability checkpoint receipts

These are exact selected JSONL records from `justrust export`, exported to an
isolated scratch directory and filtered by this lane's session and run IDs.
No unrelated historical records are included. Dates in run IDs use the local
machine zone. This is correctness evidence, not a performance comparison.

- `20261018-200025519-1664987`: full hosted suite on composed checkpoint
  `617254bff`. 3305 passed and 51 failed. Thirteen parameter-equality tests and
  the recursive planning profile exposed repeated obligation-search admission
  after shared memo removal. Later exact-assumption terminal discharge and an
  explicitly borrowed, planner-local `ProofSession` repair those cases without
  increasing the 4096 allowance. The size gate also failed. Other failures
  included hardcoded `/usr/bin/rustc` oracle subprocesses absent on hosted.
  This failed full run is retained, not represented as a green milestone.
- `20261018-201329287-2197770`: forced-hosted trait unit tests, 80 passed,
  including parallel graph readers, exact assumption context, fresh independent
  sessions, cross-world session rejection, negative mutability controls and
  immutable equality reduction replay.
- `20261018-201343178-2204079`: forced-hosted all-target clippy with `-D warnings`,
  passed. Source at local checkpoint `6d35b7ecd` includes the explicit planner
  session and the exact 22-line reachable-mutable-state debt shrink.

The source-program workers 1/8 tests in `tests/trait_fact_scratch.rs` retain the
signature-to-body graph edges and reject invalid unused generic bodies in check
and build modes. Actual source is compiled to an ELF and executed on Linux
x86_64. The recursive planning profile still reaches the original typed depth
limit, rather than an earlier obligation allowance caused by recomputation.

Remaining work is explicit: `PlanCtx.state` is still shared mutable authority,
not fixed by this checkpoint. The budget lane separately owns retained mutable
budget handles. A complete new full-suite and unchanged size-gate pass are
required before this lane pushes a coordinated milestone.
