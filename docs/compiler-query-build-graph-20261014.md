# Optimized BUILD compiler graph follow-through, 2026-10-14

## What is now measured

The initial [query mapping](compiler-query-graph-20261014.md) was a hosted
**CHECK**, not a build. We now have a genuine optimized hosted **BUILD**, its
successfully downloaded library artifact, and a same-invocation compiler graph
joined to query-provider and blocked-event timings. This improves the mapping
coverage, not the compiler's performance. No speedup or hardware ceiling is
claimed.

Run `20261013-232242825-2161355` used the supported billed route:

```text
justrust build --remote --release -p jcode-desktop-ui --lib
```

A standard Cargo `build.rustc-wrapper` configuration added existing profiling
flags to **only the UI crate**. The wrapper forwarded all other crates unchanged.
The UI used four frontend threads, `-Zquery-dep-graph`, light self-profile events
(`generic-activity,query-provider,query-blocked,incr-cache-load`), and private,
isolated profile/incremental directories. There were no rustc implementation
edits, compiler/default promotions, billing changes or security exceptions.

## Real BUILD durations and cache epoch

| Observation | Measured result |
|---|---:|
| Recorded hosted command wall time | 77.276860237 seconds |
| Full client wall time | 89.474384324 seconds |
| UI rustc unit wall time | 15.908533573 seconds |
| UI reported process CPU | 129.697791 aggregate CPU seconds |
| Whole build reported CPU | 880.737922 aggregate CPU seconds |
| Build CPU utilization | 11.397 average / 23.941 peak cores, out of 24 |
| UI frontend threads | 4 |
| Compiled / Fresh units | 887 / 0 |
| Dependency-cache hits / misses | 726 / 107 |
| Local-cache hits / misses | 0 / 53 |

This was an isolated fresh workspace and target with a **warm shared dependency
cache and 107 misses**, not a cold-registry build or a warm one-line edit. It
built a selected library and its dependency closure, not every workspace binary
or test. One 22.9 MB artifact was downloaded successfully. Executable launch
acceptance is not claimed for this `--lib` build.

One foreign rustc was observed, with average other CPU usage of 0.752 cores.
These timings are not an uncontended baseline or a causal comparison. The UI
CPU/wall ratio includes LLVM and other process activity, not just frontend
queries. It is not evidence that eight cores are a ceiling or that 96 cores
would be cost-effective. `jcode_base` separately took 24.987 seconds; the UI
unit is not the whole build's critical path.

## Actual dependency and elapsed mapping

| Mapped output | Actual result |
|---|---:|
| Graph nodes | 2,809,610 |
| Read-dependency edges | 20,496,477 |
| Sparse index holes | 320 |
| Provider intervals joined to saved nodes | 2,695,071 |
| Provider intervals without a saved node | 0 |
| Nodes without observed provider timing | 114,539 |
| Raw profile records validated | 6,851,280 |
| Unresolved event labels | 0 |
| QueryBlocked records | 177 |
| Aggregate known blocked elapsed | 228.328355 ms |
| Blocked records lacking a saved caller-to-producer read edge | 68 |
| Blocked records without an observed enclosing caller | 2 |
| Structural longest read-dependency chain | 146 nodes |
| CPU-work / execution-span model | **Unknown / unknown** |

The mapped exclusive provider elapsed sum is 21.568632374 seconds across
threads. Nested query intervals and known blocked intervals are subtracted,
but unreported locks and OS descheduling remain. It is **not CPU work W**.
The 146-node chain and structural frontiers have no CPU weights and do not
establish span S or a runnable-thread count. Untimed nodes remain unknown,
not zero-cost work.

The largest observed blocked interval is 61.475192 ms. QueryBlocked records
identify the **producer**, and the mapping separately finds the observed
containing query as the caller. The timer includes latch waiting **and post-wake
cache lookup**. It is not a pure idle timer or a serial fraction. Missing read
edges and caller contexts are retained in the public output.

### Concrete provider observations

Names use the exact pinned 337-kind table, not the fork's shifted enum.
The committed mapping includes numeric node IDs and individual intervals.

| Query kind and node | Exclusive elapsed ms | Inclusive elapsed ms |
|---|---:|---:|
| `resolver_for_lowering_raw`, 34492 | 393.651140 | 408.352972 |
| `collect_and_partition_mono_items`, 2530820 | 315.752178 | 4,362.513204 |
| `symbol_name`, 2445501 | 137.258856 | 137.258856 |
| `analysis`, 1063506 | 102.182103 | 3,012.611809 |
| `typeck_root`, 433285 | 84.176400 | 95.795019 |
| `typeck_root`, 506105 | 81.704923 | 91.354008 |
| `check_type_wf`, 430900 | 59.947870 | 553.582858 |

The report deliberately ranks elapsed observations rather than promising that
these operations can safely run in parallel or be removed.

## What remains unknown

The incremental read graph is not a complete execution DAG. It omits ignored
and eval-always reads, ordered parent pre/post-child continuations, jobserver
handoffs, scheduler delays, unreported locks and subprocess causality. Existing
LLVM/codegen generic activities broaden elapsed coverage, but their labels do
not supply complete task creation, token identity or fork/join edges.

Profiler thread IDs are Rust thread IDs, not proven Linux OS TIDs. Existing
measureme counters provide wall time and supported instruction-count variants,
not per-thread CPU nanoseconds. Instruction counts are not CPU time. Therefore
both `cpu_work_ns` and `execution_span_ns` remain null. The usual model
`T(P) >= max(W/P, S)` cannot yield a hardware recommendation without valid W/S.

No rustc implementation change is proposed or made by this deliverable. The
fork's repository policy prohibits this session from implementing
soundness-sensitive query/codegen changes. A named reviewer is not an exemption.
Existing-source analysis and existing profiling outputs are the scope here.

## Bounded large-capture interface

The real release capture has a 167,442,506-byte graph and a 263,258,619-byte
profile. Both remain below the unchanged 256 MiB per-input limit, but exceed
ordinary node/event limits. An explicit `--large-capture` flag raises only the
selected count limits:

| Limit | Default | Explicit large capture |
|---|---:|---:|
| Graph nodes | 2,000,000 | 4,000,000 |
| Graph edges | 20,000,000 | 32,000,000 |
| Graph index space | 4,000,000 | 4,000,000 |
| Profile records | 4,000,000 | 8,000,000 |
| String entries / mappings | 2,000,000 | 4,000,000 |
| Input bytes, each | 256 MiB | 256 MiB |

```bash
python3 bench/query_graph.py CAPTURE-dep-graph.bin --large-capture
python3 bench/query_profile.py --input CAPTURE.mm_profdata --output NEW-summary.json --summary-only --large-capture
python3 bench/query_mapping.py --graph CAPTURE-dep-graph.bin --profile CAPTURE.mm_profdata --receipt PRIVATE-receipt.json --output NEW-mapping.json --large-capture
```

Receipt, version, sparse-index, cycle, nesting, string expansion, symlink and
exclusive-output validation remain in force. Both real large inputs still
reject through the default CLIs with exit 2. Mapping now streams validated raw
records instead of retaining all GenericActivity dictionaries, but still retains
the graph and query intervals. **Memory usage is not constant.**

## Acceptance and requirement traceability

| Requirement / changed output | Check and observed outcome |
|---|---|
| Actual optimized BUILD, not CHECK | Recorded `subcommand=build`, `--release`, hosted routing, exit 0, UI opt-level 3 and artifact download observed. |
| Same-session graph/profile identity | Raw SHA256, metadata PID, filename PID, final incremental path, crate and unit interval match the private receipt. All 2,695,071 provider records join saved nodes. |
| Real public large mapping | Public mapping CLI exit 0; committed compact output preserves unknown CPU/span and missing coverage. |
| Public graph and profile interfaces | Real large structural graph CLI and streamed profile-summary CLI exit 0. Repeated profile summary exactly matches the original. |
| No silent default resource expansion | Real default graph/profile CLIs reject exit 2, without an output file; opt-in count-limit and non-boolean API regressions pass. |
| No filtering or nested-time invention | All 6,851,280 records counted; iterator/materialized fixtures agree. Existing nesting, producer/caller and crossing-interval cases pass. |
| Whole package and boundaries | 169 offline bench tests pass, including 61 query tool tests, malformed formats, count limits, receipts, symlinks and output preservation. |
| Independent whole-result review | Reviewer verifies both original raw hashes, seven receipt identity fields, provider/node/frontier/raw-count invariants and honest null CPU/span. Small real default/large CLI outputs are byte-identical. This is not an independent second large parse. |
| Exact exported records and privacy | Two cited run records exported from repository root into an owned namespace. Full commands, raw profiles and private receipts remain unpublished. Published profile summary replaces the command with its hash. |
| Infrastructure and lifetime boundaries | Build ended 06:24:11 UTC; bounded read-only retrieval ended and host was explicitly released at 06:41:46 UTC. No subsequent account calls, installs or default changes. |
| No performance or core-ceiling claim | Foreign rustc, cache epoch and profiling overhead remain attached. No before/after suite comparison exists for this mapping deliverable. |

Evidence is under `bench/results/compiler-query-build-graph-20261014/`.
Tools are committed as `8652f305786c4d5f0d0d880f9f7a97db4767269d`.
The compact mapping retains input hashes and a private-command hash. It is a
trusted owner-generated consistency receipt, not cryptographic attestation.

The failed harness attempt `20261013-231920710-2139396` is also retained. A
`--target-dir` override selected the documented local-only route; remote paths
then failed before any compiler unit ran. The initial environment variable was
misnamed. The successful retry used `JUSTRUST_REMOTE_REQUIRED=1` and a new pinned
workspace without overriding target-dir. This was a harness correction, not a
routing defect, an AWS failure, or permission to bypass billing.
