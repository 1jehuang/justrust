# Native cache-miss attribution (v42 candidate)

Source-only diagnostic proposal against accepted HEAD `679d34ffa0f5c11e81ca1f6d1fe35c703c62ef36`. This is not a cache-policy or root-eligibility change. The reported baseline of 10,833 compiled / 44,995 reused functions, versus recovery 178 compiled / 55,650 reused, motivates measurement but does not establish why any entry failed.

## Activation and output

Set exactly `RA_JIT_CACHE_MISS_STATS=1` before starting the native process. The first native `Entries` construction latches the process gate in a `OnceLock`; subsequent environment changes are ignored. Missing, empty, `0`, `true`, and whitespace variants remain off. Each `Entries` owns optional native relaxed atomic aggregate counters. Disabled mode allocates no counter array, performs no counter atomic operations, adds no store reads or clocks, and emits nothing. Existing key computation, store reads, decode/soundness checks, short-circuit ordering, banned lookup, path remapping, import predicates, memoization, and SCC settlement decisions remain intact.

At the successful native compilation boundary, immediately before releasing the shared entries map, `mod.rs` requests one bounded snapshot. It contains only the fixed prefix `ra-jit cache-miss-stats v1`, fixed counter labels, and unsigned counts. No function names, paths, cache keys, environment values, or imported symbols are printed. Stderr write errors are ignored. Counters span native retry rounds sharing that entries map. Aborted compilations do not promise a snapshot. A new compilation has a new entries population. This is not a process-lifetime counter report.

## Populations, not exclusive explanations

| Counter | Actual observation |
| --- | --- |
| `key_unavailable` | Existing `Entry::key` returned `None` in get initialization or prefetch key construction. Not called a missing stored key. |
| `direct_key_missing` | Existing direct `Store::get` or keyed `get_many` result was `None`. Store-layer absence/read failure is not distinguished. Counts reads of dependency entries too, not only compilation roots. |
| `banned` | Existing ban check rejected a fresh get initialization or a prefetch candidate. |
| `decode_or_sound` | Existing `Entry::decode` returned `None`, including malformed encoding or failed internal reference soundness. |
| `debug_types` | Existing stored-debug-type availability predicate rejected the decoded entry. It retains the exact `has_used || types` short circuit. |
| `path_remap` | Existing `Entry::arrived` returned `None`. Currently this means cargo-placeholder files without a local cargo directory. |
| `arrived` | Decode, debug-type predicate, and path remapping accepted the entry. This does not imply usable transitive dependencies. |
| `get_positive`, `get_negative` | Every `Entries::get` return, including repeated positive or negative OnceLock memo hits. Prefetch is not a get call. |
| `settled_positive`, `settled_negative` | Root `usable` returned immediately from an existing settled result. A negative SCC memo is expressly not counted as a fresh missing key. |
| `direct_unavailable` | Fresh root usable evaluation failed its existing get-and-links filter. Inclusive of get failures and native-import failures. Read-reason counters provide context, not a new guessed cause. |
| `native_imports` | Existing root `links(defines)` predicate failed. |
| `dependency_settled_positive`, `dependency_settled_negative` | Traversal encountered an already settled dependency. Negative settlement is not classified as absence and does not trigger another get. |
| `dependency_unavailable` | Existing get returned None for an unsettled dependency. It can be a negative get memo, not necessarily a fresh missing key. |
| `dependency_native_imports` | Existing links predicate failed on an unsettled dependency entry. |
| `transitive_failure` | Existing traversal rejection branch propagated a dependency failure to the active path, including settled-negative dependency failures. One count per traversal rejection, not one per ancestor. |
| `usable_positive` | A fresh root traversal completed successfully. Includes SCC reuse without changing the native algorithm. |

These populations overlap across stages and repeated requests. For example one absent dependency can contribute a missing-store result, a negative get return, an unavailable dependency, a transitive rejection, and later settled-negative observations. Prefetch can race with get or another prefetch, as before; observations of attempted work are included even when `cell.set` loses a race. Arrived acceptance is not a count of functions actually inserted into the final object. Counts must not be summed as unique failed functions or interpreted as time savings. No clocks, estimated durations, or hypothetical recovered-function counts are introduced.

## Native fixture sources and root acceptance

The added tests reside alongside the existing native `Entry` fixtures in `cached.rs`. They exercise the actual `Store::open_by_content`, insertion, native key computation, encoding/decoding, soundness, debug-type lookup, remapping, ban checks, get memoization, import predicate, and positive cyclic/transitive-negative usable traversal. There is no replacement decoder, mocked store, or copied SCC algorithm. Fixture counters are explicitly enabled on their own entries to avoid process-environment mutation. Source-only fixtures are not evidence that tests passed.

Worker did not run native builds, tests, workloads, or commits. Root owns compilation, native fixture execution, disabled-output and exact-gate checks, actual strict-state workloads, and all acceptance decisions after frozen-source release. No speedup or root cause is claimed before those gates. Frozen originals, candidates, complete patch, and SHA-256 manifests are delivered separately under `ROOT/cache-miss-attribution-v42`.
