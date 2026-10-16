# Generic carriers: requirement-to-observation acceptance map

## Scope and completion status

The bounded contract in `GENERIC-CARRIERS.md` is implemented at
`2a3b022ff24b8f3246af96381dce99058a93e768`. Independent evidence is committed at
`ce8cd3b7d5042dd459b530564c1eeae1312b7a9b`.
**Full Rust is NOT complete or accepted.** This compiler remains a Linux x86-64
Rust subset. Passing this bounded slice is not evidence that arbitrary Cargo
projects, all Rust language features, other platforms or a replacement installed
toolchain work. No such installation or general-project compatibility is claimed.
There is no external outage hiding that gap: the unsupported domains are explicit
implementation boundaries in contract section 8 and `COMPILER.md`.

## Real public acceptance path, not just internal tests

The independent host built the actual `jr-rust` executable from the pushed source,
then invoked its public file-based CLI in CHECK and BUILD modes and executed its
real ELF outputs. The companion runner only supplies inputs and checks process
results; it does not replace the parser, checker, solver, code generator, linker
or executable. Raw process results, stream hashes, source hashes and graph
observations are in `experiments/results/20261016-m8a-generic-carriers-*.jsonl`.
The exact run is `compiler-generic-carriers-20261016-2313`.

A further direct public CLI exercise on 2026-10-16 at 23:20 UTC used the real
checked-in `examples/compiler/generic_carriers.rs`, not a copied source:

```text
target/debug/jr-rust examples/compiler/generic_carriers.rs --workers 1 --check
target/debug/jr-rust examples/compiler/generic_carriers.rs --workers 1 -o NEW_EXECUTABLE
NEW_EXECUTABLE
```

The same sequence with 8 workers also passed. CHECK returned 0, reported zero
executable bytes and 66 nodes/139 edges. BUILD returned 0, produced an 8,212-byte
ELF and reported 72 nodes/162 edges. Both executables returned 0 with exactly
`generic carriers ok\n` and empty stderr. Both worker settings produced the same
ELF SHA256 `21721c45361bc2155b153db5515e3ac54b72a951d1132274ef367c532f93d729`.
The local compiler hash was
`6f4883e31238983e74df0bb5a9b2d92f626215c0cbe938e8b66ef0e9ceb6ca76`.
No process timed out or truncated output.

Packaging/error behavior was exercised, not inferred: using `--check -o PATH`
returned 1 with `--check does not accept -o` and created no artifact. This first
invalid invocation was preserved as a real CLI contract observation, not treated
as a compiler semantic failure. Rebuilding to an existing executable returned 1
with `output already exists`, leaving the original executable hash unchanged.
The installed stable compiler was not changed.

The example actually checks shared replacement and reload through `replace<T>`,
a single affine indirect store through `put<T>`, mutation through the new current
carrier, and the final scalar result 72. It therefore crosses source typing,
universal verification, concrete substitution, effect lookup, borrow checking,
native code generation and real process execution.

## Each frozen external requirement has an observed CLI result

Evidence for every row below is the identically named case record in
`experiments/results/20261016-m8a-generic-carriers-generic-carriers.jsonl`.
Each row contains the full source and hash, stock evidence and both native worker
1/8 process results. `PASS` means CHECK=0, BUILD=0, executable present and actual
runtime=0 with exact `generic carriers oracle ok\n` and empty stderr.
`INVALID` means both native phases return 1 without an executable, with semantic
rejection rather than Unsupported/parser/panic substitution. `BOUNDARY` means
legal stock input, but explicit native Unsupported in both phases with no output.
These are per-case observations, not conclusions drawn from aggregate counts.

| Contract requirement / case identity | Observed workers 1 and 8 |
|---|---|
| P01 explicit_shared_root_actual_identity | PASS |
| P02 rigid_parameter_forwarding_preserves_hidden_origins | PASS |
| P03 shared_root_copy_without_referent_copy | PASS |
| P04 single_affine_mutable_root_identity_transfer | PASS |
| P05 internal_tuple_components_keep_distinct_occurrence_origins | PASS |
| P06 internal_array_constant_index_component_transport | PASS |
| P07 internal_nominal_variant_component_correspondence | PASS |
| P08 partial_component_move_preserves_sibling_carriers | PASS |
| P09 shared_carrier_put_replacement_preserves_incoming_origin | PASS |
| P10 single_affine_put_preserves_checked_child_dependencies | PASS |
| P11 current_cell_reload_and_return_follow_written_poststate | PASS |
| P12 derived_aliases_observe_one_current_canonical_cell | PASS |
| P13 separate_call_occurrences_keep_fresh_lifetime_bindings | PASS |
| P14 shared_join_and_rebind_union_component_origins | PASS |
| P15 firm_explicit_mutable_expectation_retains_checked_reborrow | PASS |
| P16 recursive_shared_forwarding_uses_declared_envelope | PASS |
| N17 local_referent_escapes_through_opaque_return | INVALID |
| N18 short_incoming_reference_escapes_external_put_cell | INVALID |
| N19 mutable_hidden_component_region_invariance_violation | INVALID |
| N20 same_erased_type_reuses_unrelated_call_lifetime | INVALID |
| N21 distinct_parameter_occurrences_cannot_exchange_return_authority | INVALID |
| N22 affine_root_actual_is_used_after_explicit_move | INVALID |
| N23 unbounded_opaque_parameter_is_duplicated_without_copy | INVALID |
| N24 indirect_put_through_shared_handle_is_invalid | INVALID |
| N25 live_derived_shared_alias_conflicts_with_mutable_cell_access | INVALID |
| N26 stored_reborrow_child_does_not_reactivate_parent | INVALID |
| N27 returned_written_cell_cannot_hide_short_incoming_origin | INVALID |
| N28 unused_generic_body_has_invalid_carrier_escape | INVALID |
| X29 newly_inferred_structural_reference_actual | BOUNDARY |
| X30 multi_expanded_affine_result_components | BOUNDARY |
| X31 mixed_affine_result_and_written_cell_flows | BOUNDARY |
| X32 derived_internal_nominal_reference_actual_call | BOUNDARY |

The contract's prefreeze X32 correction and original immutable stock diagnostics,
including null-code lifetime errors, are not changed by this acceptance map.

## Integration, regression and resource requirements

| Requirement | Concrete check and observed result |
|---|---|
| CHECK does not demand code generation | Public CHECK example reports zero executable bytes; `tests/generic_carriers_graph.rs` and complete suite pass with semantic/unsupported failure graph gates. |
| Universal proof precedes concrete effects/native emission | Graph tests exercise root 115 publication and root 114 native barrier, exact keys and no object on failures; all pass in full hosted run `20261016-160812184-1354167`. |
| Existing reference-free generics remain accepted | Existing generics, inference, associated items, traits, generic references and native ABI cases pass in their separate historical corpus receipts, not just the new32. |
| Wider workflow regression | 33 independently executed corpora cover modules, tuples, arrays, patterns, matches, loops, integer operations, reference returns and native runtime. All 1,148 source cases match declared outcomes. |
| Resource exhaustion remains Resource | Historical `expanding_recursive_instances` remains the one resource rejection, both native phases reject without artifacts. Stock BUILD recursion-limit failure versus metadata CHECK success is retained, not hidden as native parity. |
| No accidental new acceptance | All 86 declared Unsupported boundaries remain explicit. New32 X29-X32 prove four stated exclusions through the actual CLI. |
| Useful semantic errors | Every new32 N17-N28 rejects in CHECK and BUILD without an executable; separate post-fetch audit rejects Unsupported/parser/panic markers. |
| Structural/provenance/resource invariants | Exact world/phase/ledger/snapshot replay, mutable parent, local byte-limit and allocation-peak regression groups pass in the 429-test library suite. These are internal correctness gates, not substitutes for public CLI acceptance. |
| Formatting, compilation, lint and test integration | Hosted all-target check `20261016-160807352-1352794`, full 1,471-test run `20261016-160812184-1354167` and strict lint `20261016-160815408-1354710` pass. Exported profiles are committed in `data/runs/prototype-bootstrap/2026-10-generic-carriers.jsonl`. |
| Independent artifact provenance | All 33 corpus receipts use release compiler SHA256 `661081771eb14e72e4d32ea1e6e42e030d3ac540203f2c44dc53ea636188b3eb`; coordinator checked 14,106 raw stream hashes and all case source hashes. |
| Real independent example graph | `20261016-m8a-generic-carriers-example.jsonl` records exact stdout/exit, 72 once-only nodes, 162 edges, complete capture, no errors/dropped events and trace/source/compiler hashes. |
| Improvement over prior bounded domain | Six previously explicit Unsupported source identities now pass genuine CHECK/BUILD/runtime unchanged. Hash-pinned native promotion maps retain original frozen dictionaries and stock classifications; individual new records identify each changed public outcome. This is expanded supported semantics, not a measured speedup. |

## Remaining gap, explicitly not closed

The older goals named `Full Rust` and `full-rust` are open-ended aspirations, not
completed deliverables. Their acceptance workflow would require arbitrary real
project manifests, dependencies, build scripts, proc macros, platform/native ABIs
and unsupported language features. This stage does not exercise or satisfy that
workflow. In particular, X29-X32 are direct evidence of incompleteness, not merely
missing test coverage. No aggregate test count, design review or synthetic corpus
can close that broader goal. This report closes only the bounded generic-carrier
requirements and records where broader generics remain unsupported.
