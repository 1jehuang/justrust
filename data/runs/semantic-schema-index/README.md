# Immutable schema lookup: exact eb755 refresh

Published base: `eb755d7f7`. Source-only replay: `a890aa0a3`, sole parent that
published base, changing only `src/compiler/traits/associated_types.rs` lookup
and its three tests. It replays `6c3ad1593` without old cache-branch ancestry or
any previous gate/evidence commit. No main worktree, stable client or remote
branch was changed. The source and this measured gate/evidence are separate
commits for coordinator review and integration.

The index construction audit remains unchanged: fields and mutation sites are
private; `stage_associated_types` enumerates the exact normalized vector then
sorts complete declaration identities. Header duplicates are rejected before
staging and publication. The three tests verify strict key ordering/uniqueness,
exact in-range positions, schema/environment/predicate identity, logarithmic paid
comparisons, foreign/missing declaration rejection, authoritative required-item
fallback and sticky poisoned-ledger rejection. No public forged position can
enter the index. No shared proof-cache migration or nominal-key activation is
part of this patch.

## Exact current-base hosted evidence

`eb755.jsonl` contains only these six exact exported records. Run-ID dates use
the local machine timezone. All commands used the installed stable justrust
client with explicit `--remote`, and ran on hosted.

| Run ID | Result |
|---|---|
| `20261018-221603678-1861584` | Source-only replay check passed. |
| `20261018-221609930-1861583` | Three focused normalized-schema tests passed. |
| `20261018-221646656-1889357` | Unchanged graph gate: 6 passed; only failure was positive ratchet assertion that 1166 now compiles. |
| `20261018-221709600-1887597` | Fresh exact-current adjacent probes: 1208 passed; 1209 failed with the typed failure below. |
| `20261018-221744321-1936151` | All seven graph gates passed after separately approved stronger 1208/1209 requirement. |
| `20261018-221750943-1942044` | Strict all-target clippy passed. |

The 1208 success had aggregate work 66,215,600 across graph tasks. The 1209
failure was `Work`, requested 1, used 32,000,000, limit 32,000,000, no arithmetic
overflow, `src/compiler/ownership.rs:384:12`, graph 2/task 1222, producer
`Key { kind: 109, version: 1, params: [1210] }` (ANALYZE). This adjacent floor was
remeasured on eb755 plus the source patch, not inferred from the earlier 677
composition. The stronger required size is a tightening, not an allowance
increase. Work, 128MiB bytes, depth, ratios, graph kinds and mutable-state debt
ceilings are unchanged. Temporary probe source was removed before final checks.

`cargo fmt --check` and `git diff --check` passed. No full/fleet rebuild was run.
No speed or fully immutable-world claim follows from this scoped evidence.
Wide cache, PlanCtx/SchemaAdapter and cycle-free nominal/target authority
activation remain held. Main integration belongs to the coordinator.
