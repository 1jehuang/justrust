# Parallel native declaration query v72/v73 evidence

**Performance acceptance rejected, no promotion. Mixed scoped results, unknown noise, broad/useful-roots improvement unproven.** Cold preparation was higher in both pairs. Repeated noop and assertion-failure elapsed times were lower in both pairs. Other phases were mixed. Do not read this as an every-phase slowdown, causal regression or comprehensive benchmark result. See `docs/resident-parallel-declared-query.md` for exact paired seconds/percentages and the native dependency/attachment design.

## Actual history retained

- V72 check/build exited 0 and codegen passed 53. Its full resident suite passed 365 and failed 4 of 369 with the real attached-database identity panic. `native/failed-v72-full-native.log` is the approved byte-exact complete original failure transcript, independently fixed-digest bound. No private paths or telemetry data occur in that transcript. The failed source is retained, not relabeled passing or timed.
- V73 repaired only the production warmup route: small/empty/single-worker waves read `Body::of` directly on the attached parent, large multicore waves retain original parallel `lower` via safe clone. All parent tracked fragment reads remain mandatory. No unsafe casts, identity bypass or assertion weakening. The five original strict fixtures/full graph oracle are unchanged and a sixth tests actual production small/multi/transition routing and nested mutation.
- V73 passed 53 codegen and 370 resident native tests. Complete selected named native journals are retained, including all six strict fixtures. Actual run IDs: `20261031-063254128-1558563`, `20261031-063254498-1558730`.
- `observations.json` contains four actual seven-phase runtime cases, all 28 strict rows, exact selected raw stdout/diagnostic witnesses, fresh nonces, statuses, per-row inputs, safe controls/environment and cumulative entry-marker snapshots. Original private inputs retain byte-length/SHA provenance but are not published in full.
- Native-source3757, workload-source63 and pinned-toolchain4020 inventories are retained with relative paths. Native source/root lock validation passed without lock edits. Every measured case used the same engine binary and unchanged source/toolchain identities.
- Full Rust snapshots for baseline, failed-v72, candidate and live-before-restoration are byte-exact. Full proposal and repair patches, freeze receipts and source-only static checks are retained as sanitized historical text. All three exact main patches replay offline. Historical uncompiled notes describe the freeze time, not the later actual gates.
- Root's actual successful reanalysis output and historical analysis source are retained. Pair arithmetic is independently recomputed. The process-once marker proves provider entry only, not cache hits, fresh work, thread/body counts or latency attribution.
- Accepted original main was restored at `2026-10-31T13:40:07Z` after exact live-to-frozen comparisons and preservation. Bytewise original comparison and empty scoped HEAD diff passed, then production source was released. The receipt is point-in-time, not a claim about subsequent independent edits.
- All four completed exact private namespaces were stopped post-measurement with exit 0, stores preserved. Only safe cleanup receipt fields are published.

## Portable offline replay

```sh
python3 -B /path/to/bundle/validate.py --manifest
python3 -B /path/to/bundle/test_offline.py
```

Only Python's standard library and packaged files are used. No Rust, Cargo, justrust, Git, engine, daemon, toolchain or network job runs. Original hash-bound pure parser/contract/oracle functions are extracted by AST without importing execution drivers. All 28 rows are reparsed so the original tuple/list equality contract is preserved, not weakened. Independent exact named/status/count checks, source patch context, unchanged OFF/lower/five strict fixtures, bounded repair, native sets, inventories, arithmetic, restoration and nonclaims are also checked.

Eight offline tests include 41 semantic negative mutations, coherent raw-plus-parsed wrong-name rejection, original tuple/list behavior, exact patch-context rejection, rehashed native/failure-log tampering and coherent elapsed-plus-arithmetic tampering. Isolated relocation runs Python `-I -B` from an unrelated directory with empty HOME/PATH and verifies HOME remains unchanged. These validate evidence integrity/replay, not new native acceptance or performance.

`SHA256.json` inventories the package. Independent fixed commitments bind historical observations, selected native journals, full failure transcript, full Rust snapshots and original pure oracle sources. They are not external signatures and do not protect against rewriting the verifier itself.

## Explicit privacy allowlists

`export_private.py` consumes only named historical artifacts. It selects eight fixed non-path environment values and five startup controls, omits twelve path-bearing environment entries per row, permits only exact stdout line grammar and exact diagnostic witness lines, and exports only exact provider-entry marker lines from worker snapshots. Full raw worker/daemon/state logs, ambient environment, private homes/cache roots, credentials, telemetry data, binaries and targets are excluded.

Only explicitly selected source/script/proposal text has path-bearing constants replaced by labeled placeholders. Full approved Rust snapshots retain pre-existing synthetic fixture literals such as `/home/one/.cargo`, not the user's home. The full failure transcript is separately approved and path-free. All consumed private originals are checked unchanged after export. The bundle-local `.gitattributes` exempts historical unified-diff text only from whitespace checks, not Rust snapshots or executable Python replay code.
