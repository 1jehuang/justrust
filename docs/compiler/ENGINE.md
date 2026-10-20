# Graph engine prototype

This is the new clean-room implementation, not the previous compile-service
client. The contract is [PROTOTYPE.md](PROTOTYPE.md). Real Rust parsing,
semantics, code generation, and linking are not implemented yet.

## Bootstrap

Fresh package `justrust-engine`, executable `jr-prototype`, Rust 2024 edition.
The initial implementation uses only std. This is a starting choice, not a ban
on utility crates. It neither embeds rustc nor delegates compiler operations
to Cargo. Those tools only build this implementation.

Initial local tools: Arch Linux rustc 1.98.1
(`48a229ceaefd4985c50990b14116b6d856af0985`, 2026-09-01), Cargo 1.98.1
(`797e8a9bc`, 2026-08-05), x86_64-unknown-linux-gnu. Record the actual hosted
toolchain separately with each experiment. Do not infer it from this local one.

```sh
cargo fmt --check
justrust check
justrust test
justrust clippy --all-targets -- -D warnings
justrust run --bin jr-prototype -- --shape diamond --size 64 --workers 8
```

Use the installed stable justrust client, not this experimental binary, to run
these commands. Do not `cargo install --path .` over the stable service client.

## Runtime boundaries

- One graph is one immutable snapshot and one execution. A second `run` is
  rejected. There is no cross-build cache validation, eviction or persistence.
- Canonical key: operation kind, schema version and ordered integer parameters.
  Providers define valid keys and their fixed output count. These are synthetic
  operations with u64 results, not a finalized compiler IR or identity schema.
- Nodes own outputs. A logical node may fork multiple independently runnable
  futures. A suspended future yields its worker. A finalized output can wake a
  consumer before other outputs or branches of the producer finish.
- `Demand::result` reads an individual retained output. It does not establish
  whole-root success. Check `Demand::completion` separately, including failures
  after publication. Finalized values are not retracted by later failure.
- Cancellation detaches the individual request. This baseline can still finish
  orphan work rather than prune its transitive demand closure. It must never
  cancel work needed by another request.
- Providers must yield only to engine dependencies or immediate self-wakes.
  External I/O/timer futures need resource admission and external-pending
  accounting before integration. Never label an idle external wait a proven
  semantic cycle. Non-yielding provider code also needs an outer timeout.

The initial runnable queue is a shared mutex queue: a **correctness baseline**,
not a proposed scalability-optimal default. Canonical interning is partitioned.
Execution batching, work stealing, NUMA placement and allocator layouts remain
measurement-driven follow-ups. Logical dependency boundaries must not change
merely to reduce physical scheduling overhead.

## Fixtures and accounting

`--shape` selects wide, chain, diamond, dynamic, fixed-point, multi-output,
or build-check-test.
`--size`, `--work` (CPU rounds), and `--seed` control deterministic inputs.
Each invocation compares results to a sequential reference unless explicitly
passed `--no-reference`. That switch is reported, never hidden.

Build/check/test are synthetic roots sharing source/signature/body computations.
Test harness work remains additional. The fixed-point fixture is a small local
monotone finite-lattice iteration, not general recursive Rust inference.

CLI emits one JSON summary. `--revision FULL_SHA` supplies caller-recorded source
provenance, not an intrinsic proof of the binary's revision. `--trace NEW_FILE`
retains Debug-format engine events, canonical keys, and observed edges. Trace
files are not overwritten. IDs distinguish nodes, requests and continuations.
Accounting is on during normal execution, with bounded event storage and
explicit overflow/incomplete capture reporting.

Queue, poll-execution and suspension intervals are sums across continuations,
not mutually exclusive pieces of wall time. Poll wall time includes provider
work and engine calls, not exact CPU time or pure useful work. Node counts and
retained u64 outputs are not allocator-byte measurements. Sampled stacks,
critical-path reconstruction, complete resource-admission attribution,
per-node allocated bytes and instrumentation-disabled comparisons remain work
to implement. Do not describe these first counters as free full profiling.

## Hosted experiments

Run only in the dedicated experimental checkout and cache on the shared M8a.
Keep stable tools untouched. Invoke the private service's authenticated
`tools/experiment.py` wrapper with guest-side `timeout` for lifecycle tracking.
Use the exact source revision and record the actual build commands/toolchain.

After building a release binary from a clean checkout:

```sh
python3 experiments/matrix.py \
  --binary target/release/jr-prototype \
  --revision "$(git rev-parse HEAD)" \
  --output /path/to/new-matrix.jsonl
```

The default matrix spans 1, 2, 4, 8, 16, 32, 48, 64, and 96 workers, three wide
work costs, and serial/shared/compiler-shaped controls. Every subprocess has a
120-second timeout. Output is flushed after every sample and never overwritten.
The environment record includes lscpu topology, source revision, seed and
instrumentation mode. Measurements include process CPU time and cumulative child
RSS high-water mark (not per-node memory or per-sample RSS). Matrix rows retain
reference-check and capture coverage status.

The machine is shared with stable builds, so samples are **contended/uncontrolled**.
One run is diagnostic, not a stable distribution, a compiler speedup, or a
scheduler default-promotion gate. Keep raw results and limitations with any
interpretation. The historical `bench/` suite targets the deleted legacy client,
not this engine and not a drop-in acceptance harness for the rewrite.
