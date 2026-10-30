# Correction: allocation equality is not provider re-execution

The original six `jr_incremental_reexecuted_*` and
`jr_incremental_eval_always_*` markers in driver
`47caa85292e729553328c669d88631e0b90550a173609941aebcd14c0a3cf10c`
classify the shared `hash_result_and_alloc_node` path. That path receives
**fed query results as well as executed task results**. Original reports remain
unchanged as historical artifacts, but any scope text claiming these records
prove a provider executed is superseded by this correction. Counts and hashes
are valid allocation-path observations, not provider execution counts.

The private pinned `f97fcc2` real workload exposed the distinction. A forced
unchanged `justrust check --lib` observed 17,019 ordinary same-result records,
including 8,936 `def_kind` and 6,315 `visibility`. The former explicitly has no
local provider in `compiler/rustc_middle/src/queries.rs`: its local values are
fed during definition creation. `DepGraph::with_feed_task` calls the shared hash
helper. Thus simply retaining more `def_kind` values is not established as a
provider-work optimization by these counts.

A real one-line body edit inserted `std::hint::black_box(123u64);` into `ago` in
`src/runs.rs`. Same source/cache/flags produced 19,051 ordinary same-result,
34 ordinary different-result, and 30 ordinary no-hash records. Eval-always
records were 12,592 same, 16 different, and 2 no-hash. These are **mixed-path
records**, not avoidable calls or a reuse percentage. Unchanged, edit and
restored-source checks succeeded with incremental hash verification. The source
was restored to the pinned bytes. All experiments remained default-off.

`real-mixed/` retains exact commands/policy, client/compiler/driver/Cargo hashes,
source hashes, logs, raw profile hashes and strict replay inventories. The
runner uses a single fixed profile directory and RUSTFLAGS across all epochs,
so changing the capture output does not invalidate Cargo inputs. Private target
and source directories were used, with local execution required for the private
frozen native compiler. No hosted workload was started during maintenance.
Instrumented timings are not a performance comparison.

The corrective native protocol will use distinct `jr_incremental_provider_*`
labels emitted only at the query provider execution boundary. Historical mixed
labels must never silently acquire the stronger provider-only interpretation.
New provider-only captures and correctness gates are required before interpreting
that frontier. This finding narrows what has been established, not evidence of
having reached a maximum.
