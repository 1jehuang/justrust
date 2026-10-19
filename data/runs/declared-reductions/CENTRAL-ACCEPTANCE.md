# Central immutable declared-reduction acceptance

Frozen `b6bb5e9218c7c621e838110ba24c7ac651f09153` composes the bounded
construction-final reduction freeze with published `ecc52509d`.

The sole reductions producer now builds a task-local paid sink, then moves or
appends it through the existing declaration index before world publication.
Published declared contexts contain immutable rows and receipts, not the former
reductions mutex. World/stage/declaration, membership, binder and normalized RHS
replay remain required. Exactly 10 reachable debt entries were removed (72 to 62)
and one mutable carrier disappeared (7 to 6). No other ceiling, budget, node kind,
cache, index or whole-program join was added or loosened. Required1208/1209 and
all 10 integrity gates remain unchanged except the exact shrink-only inventories.

Six central owned redacted records are in `central.jsonl` (the export's machine
label is inherited from the preceding export directory, not a test identity).
All IDs below have prefix `20261018-`:

| ID suffix | Check and observed result |
| --- | --- |
| 233339124-3047765 | Forced-hosted `justrust check`, exit 0 |
| 233345521-3048070 | Forced-hosted `justrust test --lib declared_reduction`, 8 passed |
| 233403412-3059219 | Forced-hosted graph_integrity plus parameter/associated equality and associated-type authority, 61 passed across 4 binaries |
| 233423918-3066680 | Forced-hosted strict all-target clippy, clean |
| 233455921-3068122 | `JUSTRUST_TEST_JOBS=4 justrust test --local --no-fail-fast`, 3513 passed, 0 failed/ignored/filtered, 196 binaries |
| 233914728-3252798 | Final forced-hosted strict all-target clippy, exit 0 (fresh artifacts) |

Formatting and diff checks passed. The local full suite exercises pinned stock
subprocess paths. It used fresh test artifacts and was contended (50.442 CPU-stall
seconds); it is correctness evidence, not a speed measurement. The source remained
frozen until full and lint completion.

Original isolated expected-red and green records remain in `scoped.jsonl`.
Actual-capacity rejection is fail-closed; old and failed retained admission is
conservative, not refund-neutral or a minimal-peak guarantee. See
`DECLARED-REDUCTIONS.md` for trusted producer and remaining nominal authority
boundaries. PlanCtx, nominal interner, schema adapter and other mutable graph debt
remain explicitly unfinished. No full Rust, corpus promotion or speedup claim.
