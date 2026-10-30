# Real pinned provider-only reuse frontier, 2026-10-30

This extends the finite native acceptance with a real `f97fcc2` justrust library
check. It is an instrumented mechanism diagnostic, not a speed benchmark or a
maximum reuse calculation. The corrected provider boundary excludes feed results
and generic non-query tasks, unlike the historical mixed markers.

Frozen driver SHA-256:
`8cfb723e9ffb482bdd9518251f281c5169aae7211804e9d12ff5bbb29f321569`.
Compiler and matching standard library built successfully in 3m11s. Exact client,
Cargo, executable and driver identities and fixed RUSTFLAGS are in
`bench/results/reexecuted-value-20261030/provider-real/*-receipt.json`.
The private runner, full logs, strict inventories and raw capture hashes are
retained there. The source snapshot and compile flags remain identical across
epochs except the single intended body edit. All three experimental optimizations
remain disabled. Self-profile filter is `generic-activity,function-args` and
incremental hash verification is enabled. No hosted jobs were started.

Sequence: prepare fresh private target, touch identical `src/lib.rs` bytes to
force unchanged recompilation, insert `std::hint::black_box(123u64);` in
`src/runs.rs::ago`, then restore the exact pinned source and compile again.
All compiler and strict decoder exits were zero. Source hashes prove unchanged
and restored states match prepare, while the edit differs. Client/compiler/
driver/Cargo hashes and RUSTFLAGS match across all four epochs.

| Capture | Ordinary same / different / no-hash | Eval-always same / different / no-hash |
| --- | --- | --- |
| Fresh prepare | absent / absent / absent | absent / absent / absent |
| Forced unchanged | 5 / absent / absent | 12,608 / absent / 2 |
| One-line body edit | 2,045 / 26 / 28 | 12,592 / 16 / 2 |
| Restored source | 2,045 / 26 / 28 | 12,592 / 16 / 2 |

Absent means unknown-marker-absent, not a general zero-work claim. These are
records for previously existing keys, not unique keys or complete query work.
No legacy mixed marker was emitted. Locally fed `def_kind` and `visibility` are
absent from all provider inventories, as the corrected boundary requires.

The unchanged ordinary same-result providers are `early_lint_checks`,
`registered_attr_tools`, `registered_lint_tools`, `resolutions`, and
`resolve_type_relative_delegations`, once each. With an actual edit, ordinary
same-result records include `expn_that_defined` (1,475), `local_trait_impls` (177),
and smaller type-checking, lint and MIR families. The eval-always same-result
inventory includes `lower_to_hir` (8,930) and `source_span` (3,287), versus 6 and
9 different-result records respectively. Lowering and span queries intentionally
access current-session inputs. Their equality after execution alone is not a
safe cache lookup rule, proof of changed inputs, or evidence that a resident HIR
can be reused without re-resolution and identity remapping.

These observations distinguish three remaining hypotheses: provider work caused
by coarse dependencies, always-run frontend construction, and proven-green query
values reconstructed instead of loaded. The last already has default-off scalar
and associated-item experiments. The first two require more dependency/input
and cost evidence before changing reuse decisions. Marker recording intervals
are not provider costs, so ranking the above counts by time saved would be wrong.
The global maximum and practical time benefit remain unknown.

Correctness gates: corrected five-case native schema2 acceptance passed, the old
mixed compiler fails the new provider marker gate after matching all diagnostics
and runtime outputs, and an independent same-path profiling-off/on gate produced
identical successful binary artifacts across initial/unchanged/edit/revert states
and exact type-error diagnostics. Full Python discovery passed 512 tests. Native
rustfmt and client `cargo fmt --check` passed. These gates validate the observation
protocol, not a default optimization promotion. Comprehensive matched performance
acceptance remains blocked by the previously retained pinned workload failures.
