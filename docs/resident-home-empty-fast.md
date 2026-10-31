# Empty-argument home specialization v68, October 31, 2026

**Rejected for performance, source restored, no promotion.** The actual native
correctness gates and all 28 strict runtime rows passed. The enabled branch was
observed, but two same-binary OFF/ON pairs did not establish a repeatable useful
scoped latency improvement. Noise is unknown. No speedup or causal regression
claim is made. The rejected implementation is preserved as evidence, not shipped.

## Hypothesis and exact candidate

For empty `StoredGenericArgs` and `InstanceDef::Fn`, the original `Mentions::of`
constructs a singleton crate hash set and a one-element bodies vector. V68
attempted to avoid these allocations. Its process-latched
`RA_JIT_HOME_EMPTY_FAST=1` flag was default OFF and accepted exactly `1`.
The original dynamic `RA_JIT_NO_HOME` presence and nonempty caller-bounds guards
remained first. Only that empty-argument function case specialized.

The branch got the same `func.krate(db)`, populated the identical per-`Homes`
`sees` entry with `loaded_crates(db, krate).iter().copied().collect()`, and checked
`sees.contains(&krate)`. It did not assume loaded crates contains self. Success
used the identical `ParamEnvAndCrate { krate, ..asked.as_ref(db) }.store()` and
failure cloned `asked`. No new query or query cache was introduced. Removing
only the new helpers, branch, and test module reconstructs the original source
byte-for-byte. Generic, closure, coroutine, opaque, and shim paths were unchanged.

The fixed ignored-write-error stderr marker
`ra-jit: home-empty-fast-v68 branch` appeared only on actual enabled branch entry,
once per process. Its thread-local initializer avoided marker-related atomic
checks after that thread's first branch entry. The process-latched flag still
used a per-call `OnceLock` check. The measured candidate included these costs.
The marker establishes entry, not avoided work per phase or latency attribution.

## Actual correctness gates

Root ran release locked check/build, then **56 codegen tests passed, 1 ignored**,
and **364 resident tests passed**. The entire 3,757-file native source inventory
and integrated lock were checked unchanged across the gates. The only lock
integration was two already-resolved hir-ty-codegen devdependency edges,
`ide-db` and `test-fixture`, with no new package resolution. Root owned that lock.

The three real `RootDatabase` fixtures compared the entire stored parameter
environment and crate, unpacked environment, and complete cache contents against
an independent frozen original algorithm with a separate `sees` cache. They
covered same/repeated/cross-crate requests, cfg-selected functions, signature
references to a dependency with live source revisions and fresh/retained caches,
nonempty generic arguments, an inferred closure, opaque return-type arguments,
shim fallback, real nonempty caller bounds, and an explicit equal-cache seed
without self to test the false membership outcome. The cfg variants used
separate fixture databases, not in-place crate-graph cfg mutation.

Root additionally ran four **actual fresh-process fixture controls**: flag unset,
`0`, and `1` each passed all three ordinary fixtures, with the NO_HOME fixture
ignored. The enabled control emitted one branch marker, the other two emitted
none. A separate process with both NO_HOME and the candidate flag set passed the
one ignored-by-default guard fixture, with no branch marker. These controls
validate this branch's observed correctness, not useful workload acceleration.
The control environment is documented by root's explicitly post-hoc witness of
its actual tool call, not a contemporaneous worker-generated launch receipt.
The original console logs and complete journals independently prove execution,
names, counts, ignored status, and observed marker output. No rerun was used.

## Actual scoped runtime result

One frozen candidate binary ran OFF, ON, ON-repeat, OFF-repeat, each through seven
ordered phases with fresh nonces. All **28 strict original-oracle outcomes**
passed: exact four named tests and counts, filtered count 144, invocation witness,
compile-error/no-runtime separation, fresh type diagnostic, fresh assertion
failure, recovery, process status, and unchanged source inputs. Each OFF worker
snapshot contained zero branch markers. Each ON snapshot contained the same
cumulative once-per-process marker, not a new invocation count for every phase.

Percent ON versus paired OFF, descriptive only:

| Phase | Pair 1 | Counterordered pair 2 |
|---|---:|---:|
| Cold prepare | +3.55% | -1.88% |
| Repeated no-op | +11.91% | +16.74% |
| Manifest feature add | -2.10% | +0.82% |
| Type error | +2.45% | +2.16% |
| Type recovery | +0.89% | -4.92% |
| Assertion failure | -1.11% | +6.30% |
| Assertion recovery | +5.47% | -11.84% |

Root rejected the candidate because it showed no repeatable useful scoped
latency improvement. These are bounded, instrumented transitions, not the
comprehensive session-weighted synthetic workload. OS cache/background CPU
noise was uncontrolled and remains unknown. Positive deltas are not proof of a
causal regression, and negative deltas are not a speedup claim.

## Restoration and portable evidence

Before restoration, all three live owned files matched their frozen candidate
copies exactly and were separately preserved. Only `home.rs` and its Cargo.toml
were restored by writing their original backups. Both exact backup comparisons
and scoped HEAD diffs passed at 12:11 UTC. Native source ownership was promptly
released. Root separately restored its lock. The remaining change is this
rejection record plus `bench/results/resident-home-empty-fast-20261031/`.
Restoration receipts are point-in-time observations, not claims about later
work by other owners.

The bundle preserves full original/candidate/live Rust and manifest snapshots,
proposal-document and patch derivatives, SHA/byte provenance, source63,
toolchain4020 and native-source3757 inventories, complete named native results,
four fixture-control journals, all runtime semantic receipts, original strict
oracle sources, root arithmetic and rejection, lock-edge patch, and restoration.
Private path-bearing documents/scripts are explicitly sanitized derivatives.
No raw private state, ambient environment, home/cache paths, executables,
credentials, build targets, or unrelated artifacts are published.

Portable replay uses only Python's standard library and packaged files:

```sh
python3 -B bench/results/resident-home-empty-fast-20261031/validate.py --manifest
python3 -B bench/results/resident-home-empty-fast-20261031/test_offline.py
```

Replay re-extracts only hash-bound pure original parser/contract/oracle functions
without importing their execution drivers. Raw text is freshly parsed back into
the original tuple representation. JSON list serialization does not weaken or
replace the original equality oracle. An independent exact named/count/status
oracle also checks every row. Eight observed offline tests passed, including
42 semantic negative mutations, coherent wrong raw/parsed names, exact patch
context rejection, rehashed failed-native-result and coherent timing/arithmetic
rewrite controls, privacy selection, and isolated unrelated-directory relocation
with empty HOME/PATH and Python `-I -B`. These are evidence checks only. No new
native job, benchmark, install, or toolchain probe was run to assemble this record.
