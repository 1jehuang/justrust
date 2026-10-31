# Native cache-miss attribution (v43)

Native diagnostic implementation based on accepted HEAD `679d34ffa0f5c11e81ca1f6d1fe35c703c62ef36`. This is not a cache-policy or root-eligibility change. The reported baseline of 10,833 compiled / 44,995 reused functions, versus recovery 178 compiled / 55,650 reused, motivates measurement but does not establish why any entry failed.

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

## v42 root result and v43 repair

Root reported that actual v42 check and build passed, but the native codegen suite passed only 50 of 53 tests. Its preserved log is `ROOT/candidate-unit-v42/codegen-native.log`. Three added fixtures expected one observation but observed zero: root native-import rejection, unavailable dependency traversal, and settled-negative dependency traversal. These results invalidate the v42 fixture gate, not establish a miss distribution or performance result.

The semantic cause is the real store publication contract. In `ra-jit-store/src/lib.rs`, `Store::insert` appends to an open pack, whereas `Store::get` reads sealed packs. The existing native test `sealed_pack_is_named_by_its_bytes_and_read_back` explicitly asserts absence after insert and presence after publication. The v42 fixture helper inserted encoded parents/imported entries without publishing them. Consequently `usable` failed at the root get instead of reaching the import predicate or dependency traversal, explaining all three zero counters without changing their expectations.

The v43 repair adds the actual native `Store::seal()` boundary to the fixture helper and asserts exact byte readback before exercising get/usable. Unlike flush with a size threshold, seal publishes however small the fixture pack. No native counter expectations are weakened, no production instrumentation or decisions change, and `mod.rs` stays byte-identical to frozen v42. Added store reads/publication are confined to `#[cfg(test)]` fixture setup, not disabled production code. Root gated this new identity as recorded below. Worker performed no native runs for the repair. The v43 archive retains exact failed v42 originals and the root failure log, plus repaired candidates, repair/full patches, hashes, and explicit readiness.

## Actual v43 root validation

After the fixture publication repair, root reported actual check/build PASS, full native codegen **53/53 PASS** in run `20261030-201905159-1445554`, and full resident **360/360 PASS** in run `20261030-201926060-1448168`. The 3,757-file source/lock identity remained unchanged across the native gates. Rust sources stayed frozen throughout acceptance.

Root ran the actual strict seven-state sequence in both `ROOT/cache-miss-attribution-v43-on` (task `8665036tdb`) and `ROOT/cache-miss-attribution-v43-off` (task `920222sard`). Both sequences passed: cold preparation, repeated noop, manifest feature addition, expected type error, type recovery, expected assertion failure, and assertion recovery. Expected error phases retained exit 101 rather than being mistaken for successful commands. Root verified the 63-file source identity, tools/toolchain identity, per-row input identity, and final restoration unchanged. `ROOT/validate-cache-miss-v43.py` binds the actual native run summaries, seven accepted state rows per mode, and fixed 19-field native reports. Its result is `ROOT/cache-miss-v43-validation.json`.

Selected enabled observations from that validation result:

| Native observation | Manifest feature addition | Type recovery |
| --- | ---: | ---: |
| `direct_key_missing` | 45,508 | 34,863 |
| `arrived` | 45,207 | 55,754 |
| `native_imports` | 9 | 9 |
| `dependency_unavailable` | 25 | 26 |
| `dependency_settled_negative` | 1 | 3 |
| `transitive_failure` | 26 | 29 |

Enabled reports recorded no decode/soundness, debug-type, path-remapping, or banned failures in this sequence. These are actual overlapping branch observations, not unique lost functions or proof that one cause explains the compiled-function gap. Root and dependency populations, repeated gets, prefetch reads, and settled results differ and must not be summed into a loss total. The last type-recovery snapshot repeats unchanged in later assertion/patch/noop cumulative logs. Its presence there is **not a fresh assertion-phase observation**. The binding script labels report counts as cumulative-log counts and does not establish new report emission per state.

All disabled-mode logs contained zero report lines. These gates validate native behavior and the enabled/disabled reporting scope on this workload. They establish neither a speedup nor causal proof of the baseline/recovery compilation difference. No timing comparison or savings claim is made.

## Native fixture sources and root acceptance

The added tests reside alongside the existing native `Entry` fixtures in `cached.rs`. They exercise the actual `Store::open_by_content`, insertion, native key computation, encoding/decoding, soundness, debug-type lookup, remapping, ban checks, get memoization, import predicate, and positive cyclic/transitive-negative usable traversal. There is no replacement decoder, mocked store, or copied SCC algorithm. Fixture counters are explicitly enabled on their own entries to avoid process-environment mutation. The initial source-only fixture release was not evidence that tests passed. The actual root-run native results above now supply the executed fixture gate.

Worker did not run native builds, tests, workloads, or commits. Root owns compilation, native fixture execution, disabled-output and exact-gate checks, actual strict-state workloads, and all acceptance decisions after frozen-source release. Even after these gates, no speedup or causal explanation of unique compiled-function loss is claimed. Frozen v42 originals, candidates, complete patch, and SHA-256 manifests remain preserved under `ROOT/cache-miss-attribution-v42`. The distinct v43 repair identity is under `ROOT/cache-miss-attribution-v43`.
