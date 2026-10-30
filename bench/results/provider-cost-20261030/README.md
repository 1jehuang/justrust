# Real provider elapsed evidence, not a reuse ceiling

The three successful compiler checks use pinned `f97fcc2`, frozen driver
`8cfb723e9ffb482bdd9518251f281c5169aae7211804e9d12ff5bbb29f321569`,
private source/target, fixed `default,function-args` profiling flags and incremental
hash verification. All optimization experiments remain off. Prepare, forced
unchanged and actual one-line body edit compiler exits are 0. Source was restored
to its pinned SHA-256 `8272de3756d2323b05777c851b19c2cc6aec6bdf073af79056da29370daf6bde`
after measurement. This is a diagnostic capture, not a matched speed comparison.

## Strict decoding limitation

The original strict reuse inventory exits 2 for **each** capture. Direct decoder
replay identifies `ValueError: unmapped virtual string ID`. Receipts preserve
`status: 0` separately from `decoder_status: 2`, and original strict-failure logs
are retained. This failure is not presented as successful strict decoding.
The supported explicit `--allow-unmapped-event-labels` mode retains those events
with null unresolved labels. It does not suppress events, invent names or weaken
format validation. Cost reports must record this opt-in and unknown-label counts.
In the unchanged/edit captures all Query provider intervals have resolved labels,
but this does not imply that all event families or compiler work are covered.
Raw profiles stay at the exact private receipt paths, with hashes retained here.
The opt-in cost CLI reports retain 73,091 unresolved event-label records for the
unchanged capture and 11,633 for the edit. Query interval unknown-name counts
are 0 in both. Independently unioning all Query intervals on thread 3 yields
666,646,752 ns (33,322 records) and 981,657,444 ns (58,014 records) respectively.
These all-query unions must not be added to the per-kind unions below, nor
interpreted as removable time or a measured difference caused by caching.

## Hypothesis check

The prior provider-only edit inventory found 1,475 same-result
`expn_that_defined` classifications. This high count alone suggested neither
cost nor prospective avoidability. In the independent full-event edit capture,
all 1,515 `expn_that_defined` Query intervals on profiler thread 3 occupy an
elapsed union of **1,337,773 ns**. They are not joined to individual result
classifications, so this is not the cost of those 1,475 same-result records.
The local provider itself is a lookup in
`tcx.resolutions(()).expn_that_defined`, defaulting to the root expansion.

For comparison, thread-3 query interval unions in that same instrumented edit
capture are `resolver_for_lowering_raw` 361,959,701 ns, `lower_to_hir`
190,255,239 ns and `analysis` 599,954,636 ns. These scopes nest and **must not be
added**. They include child queries and may include pauses. They are not CPU,
exclusive provider work, removable work, critical path, or predicted savings.
The broad frontend work is a more substantial observed elapsed scope than the
high-count expansion lookup, but sound reuse still needs explicit inputs,
identity remapping, side-effect handling and correctness testing.

Other edit thread-3 unions: `local_trait_impls` 149,248 ns, `constness`
3,000,052 ns, `coroutine_kind` 1,704,460 ns, `hir_owner_parent_q` 724,495 ns,
and `associated_items` 4,458,665 ns. Again these describe all observed provider
intervals of those kinds in this capture, not same-result-only costs or an
optimization effect. No time-saved or global maximum follows from this report.

The comprehensive performance gate remains blocked by retained pinned workload
failures. These partial observations do not replace that gate and do not promote
any experimental optimization to a default.
