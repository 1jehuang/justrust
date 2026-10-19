# Central standalone layout integration

The input-scaled calculator was integrated on `b1646c60209ccbcd738249839b45f10a896a42d0`, composing source commits `39b1166c5` and `1072336c7` with the owner's five exact validation receipts. Production and test SHA-256 hashes remain exactly those in README.md. Hare independently inspected those complete files and the relevant PlanBudget ownership APIs, with no concrete blocker under the stated standalone bootstrap contract. This was inspection, not independent execution.

Central commands all used the installed stable client with `JUSTRUST_BUILDD_PREBUILD=0` and forced hosted placement:

| Run | Command | Result |
| --- | --- | --- |
| `20261018-235202329-3290980` | `justrust check --remote --test object_layout` | exit 0 |
| `20261018-235202966-3291009` | `justrust test --remote --test object_layout -j 2 -- --test-threads=1 --skip c_scalar_and_array_object_layout_controls` | 24 passed, 1 C control filtered |
| `20261018-235203930-3291049` | `justrust test --remote --test graph_integrity -j 2 -- --test-threads=1` | all 10 passed |
| `20261018-235212468-3291515` | `justrust clippy --remote --all-targets -- -D warnings` | exit 0, fresh artifacts |

`cargo fmt --check` and `git diff --check` also passed. `central.jsonl` contains only these four original path-redacted records, selected by exact ID and coordinator session from the stable export. The C control was not rerun centrally: the unchanged-source owner run `20261018-234402059-3266586` passed 1 control with 24 filtered, and is preserved separately in `2026-10.jsonl`.

The module remains unregistered and is compiled through the integration test's explicit path import. No compiler consumer, source intrinsic, semantic type admission, addressed storage, platform calling convention, nominal layout or Box support was added. All graph ceilings and the 62-entry mutable-debt baseline remain unchanged. Requested storage is prepaid; actual allocator surplus is admitted before initialization or publication, not claimed to have been prepaid before the allocator returns it. Accounting receipts are not source or semantic authority.

A redundant whole-compiler suite was not run for this unregistered building block. The prior compiler milestone remains the 3513-test immutable-reductions acceptance, not a claim that these new layout tests participated in that snapshot. No performance or full-Rust claim follows.
