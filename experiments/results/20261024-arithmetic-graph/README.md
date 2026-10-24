# Hosted arithmetic graph profile, 2026-10-24

## Result

The current compiler is **slower than stock rustc on these tiny programs**.
More workers do not remove the dominant serial frontend work. This is a
scoped diagnostic, not a representative full-Rust performance comparison.

| Program | Operation | Stock rustc | Native, 1 worker | Native, 8 workers | Native, 96 workers |
|---|---|---:|---:|---:|---:|
| 6 functions | Check | 16.8 ms | 387.9 ms | 394.3 ms | 406.4 ms |
| 6 functions | Build executable | 42.4 ms | 391.2 ms | 389.0 ms | 403.9 ms |
| 258 functions | Check | 29.1 ms | 494.2 ms | 482.5 ms | 478.0 ms |
| 258 functions | Build executable | 64.0 ms | 579.7 ms | 497.3 ms | 507.6 ms |

These are medians of seven fresh-process invocations after one warmup per cell.
Raw minima/maxima are in `summary.json`. The shared host was not reserved,
and the small-program native build samples ranged from 364.1 to 506.1 ms at
one worker. Do not interpret small differences between worker settings as
stable improvements or regressions. The native/stock gap is present throughout
the observed ranges. Stock uses its normal default threading, not an injected
96-thread compiler setting.

## What the graph profile shows

Separate bounded captures, not the timed samples, report these task intervals:

| Native one-worker build | 6 functions | 258 functions |
|---|---:|---:|
| Parse | 67.0 ms | 74.4 ms |
| Resolve | 66.0 ms | 70.6 ms |
| Signatures | 203.1 ms | 230.4 ms |
| Total of those three nodes | 336.1 ms | 375.3 ms |
| Graph elapsed time | 351.4 ms | 545.1 ms |
| Object emission task intervals | 0.18 ms | 5.94 ms |
| Graph node count | 291 | 2,559 |

Parse, resolve, and signatures account for about **96% of the small graph's
elapsed time** and **69% of the larger graph's elapsed time** at one worker.
These are whole-program nodes. In the measured source revision,
`src/compiler/mod.rs` makes resolution await parsing, and signatures await
resolution. This is an actual serial dependency chain, not merely idle cores
in an otherwise wide graph.

At 96 workers, the larger graph still spends roughly 376 ms in those same
three nodes, out of 483 ms graph elapsed. Parallelizing later work cannot
remove that floor. The broader frontend processes the native source prelude;
the graph profile does not separately attribute how much of each frontend
node is prelude work, so that contribution is not quantified here.

The 96-worker larger build's summed task-wall intervals reach 2,203 ms while
its graph elapsed time is 483 ms. **Summed intervals are not CPU time** and
must not be added to elapsed time or interpreted as 96-core utilization.
They include scheduling/transition effects. No CPU samples or allocation
counts were collected. Allocator data is null. Some node kinds are still
labelled `other` by the bounded exporter; their numeric kinds are retained.

All 12 final bounded profiles report complete event capture, zero dropped
events, zero unmatched intervals, and successful compilation. This asserts
event accounting completeness, not complete semantic or CPU profiling.

## Workload and measurement boundaries

- Exact `experiments/compiler.py::source_for` generator: 4 or 256 arithmetic
  functions, plus `main` and a division failure helper. Sources are retained.
- Each arithmetic function computes `if x < 0 { -x } else { x + constant }`.
  Main calls every function with 2, verifies answers, and prints a success line.
- Every produced executable, including warmups and profiled builds, was run
  and checked for exit success, exact stdout, and empty stderr.
- Wall timing includes process startup and stdout collection. It excludes
  executing the generated program and excludes the separate profile runs.
- Warm filesystem caches, no Cargo invocation inside samples, no incremental
  directory, no explicit cache flush, no CPU pinning. Check means stock
  `--emit=metadata` and native `--check`.
- Stock Rust 1.99.0, edition 2024, opt-level 0, overflow checks enabled,
  normal standard runtime and linker. Native uses its current default
  optimizer setting, source prelude, private runtime, and direct ELF emitter.
  These are not identical optimization pipelines or language capabilities.
- AWS hosted M8a.24xlarge, 96 physical cores, AMD EPYC 9R45. Full topology and
  starting load are retained in `environment.json`. SSH/network transfer is
  outside each compilation timer.
- Native revision `acd9c70d4a53a1b987b2ea5d8d3216ae9b0edf0c`, built in a
  separate immutable archive through hosted `justrust build --release --bin
  jr-rust`. Build run `20261023-183001202-2893779`. Binary SHA-256 is retained
  in `environment.json`. Compiler build time is not a benchmark sample.

## Historical comparison and next questions

October 15's prototype was much smaller and did less work. Its sub-millisecond
small-program result cannot be presented as the current compiler's speed.
The current test supports the architectural diagnosis that coarse frontend
nodes remain a bottleneck; it does not disprove fine-grained graph execution
or demonstrate the intended persistent caching benefits.

The next useful measurements are subdivisions of signature construction and
prelude preparation, followed by controlled per-file/per-item graph changes
or immutable prelude reuse. Keep semantic and forged-input gates intact.
Measure those changes against this exact workload and broader real programs,
not just worker utilization or synthetic graph throughput.

## Reproduce

Build the named revision as an isolated release binary using `justrust`, then
run on the target host:

```sh
python3 experiments/arithmetic_graph_bench.py \
  --binary /absolute/path/to/jr-rust \
  --revision acd9c70d4a53a1b987b2ea5d8d3216ae9b0edf0c \
  --output /new/private/result-directory
```

The final retained run used this bounded-only harness and completed end to end.
An earlier diagnostic completed the timing matrix but was interrupted while
serializing an unbounded Debug trace, which exceeded 500 MB. That capture was
stopped; it is not the source of the table above. Full Debug traces are not
necessary to obtain these bounded phase profiles and are not committed.
