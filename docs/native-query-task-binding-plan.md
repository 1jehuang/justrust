# Next causal mapping slice: explicit point-to-task binding

Status: design only, 2026-10-14. This capability is not implemented or claimed
by the current query schema 3 and pool schema 1 receipts.

## Prerequisites found while testing the next hypothesis

The pool's `CURRENT` logical-task slot is separate from rustc's implicit query
TLV. A helping worker can execute unrelated scope/spawn work while a task is
installed. Same observer identity alone does not establish ownership of that
executing job. Actual pool regression run `20261014-114100301-42923` failed all
three added controls: opaque work inherited a root or child, and a nested join
incorrectly forked the helping root and replaced it with a continuation. The
follow-through dispatch isolation fix must pass rebuilt native acceptance
before new query bindings can rely on this slot.

Standalone query capture identity is currently chosen at final publication,
whereas the pool chooses its identity at observer creation. A `-pool` filename
is not an authority proof. Keep distinct receipt capture IDs and explicitly
bind them in both directions before reconciling their namespaces.

## Proposed contract, not an implemented schema

- Query outer schema 4 gains a typed `task_bindings` schema 1 object. Existing
  query admission, wait and read event contracts remain individually typed.
- Its header identifies the exact expected pool receipt by capture ID and
  observer ID, or declares absence with an explicit reason.
- Pool schema 2 names the corresponding query capture ID. Transport-assigned
  query identity remains unchanged. Standalone identity must be created before
  pool attachment rather than only at root Drop.
- Bind points, not an entire query-job lifetime: admitted `(context, job)`,
  request start, dep-read, produced result, registered wait and resumed wait.
  A job/request can cross forks and distinct continuations.
- Each binding has a fully qualified source identity and a nullable target task,
  with an explicit reason for unbound points. A bound point carries time from
  the pool's own epoch, permitting comparison with its task start/finish.
  Never compare unrelated query/pool clock epochs.
- Read every point directly. Do not inherit a read binding from an earlier
  request or assume job ID equals task ID, context ID equals observer ID, or
  OS/TLS/pointer identity is a recorded causal endpoint.

## Independent inventory and publication

Expected rows come from independent allocator counters, not the recorded rows:

```
expected admitted jobs
+ (request_high_water - 1)
+ (read_high_water - 1)
+ (result_high_water - 1)
+ 2 * (wait_high_water - 1)
```

Use checked arithmetic and explicit independent loss accounting. Require exact
source identity ranges and one row per point, including explicitly unbound
rows. Distinguish `records_complete`, `all_targets_bound` and reconciled graph
binding validity. A complete row inventory containing unbound points is not a
fully mapped pipeline. Bounded overflows stay incomplete.

Capture access must remain inside existing observer-enabled branches. Avoid
new query TLS/clock/allocation work when capture is disabled. A pool API can
cheaply borrow the current slot, copy a task ID, and verify observer ownership
without cloning several Arcs or locking the receipt snapshot. Wait registration
must remain lock-free with respect to observer buffers while native latch locks
are held. Flush rows only after leaving native locks.

## Required gates before any claim

1. Real worker dispatch isolation: unrelated helper work has no inherited
   ownership, including nested help, panic, CountLatch, FIFO, broadcast and
   cross-pool paths. Instrumented stolen/inline join branches retain their own
   explicit identity and the suspended caller's exact slot is restored.
2. Strict typed client tests: duplicate/unknown fields, missing entire rows,
   counter mismatch/overflow, foreign observer/capture/task IDs, absent pool
   receipts, nonreciprocal headers, points outside task lifetime, explicit
   unbound points, loss and false full/CPU/span claims.
3. Rebuilt compiler fixtures which actually exercise query/read/wait points in
   nested joins, including a distinct resumed continuation, plus opaque scope
   work that remains unbound. Standalone observer tests do not replace this.
4. Genuine Cargo/compiler authority reconciliation, capture-off/on artifact
   and proc-macro-visible argv/output neutrality, preserved errors and fail-open
   recording sinks.
5. Matched measurement of overhead. No general speedup/default promotion claim
   without the comprehensive pinned suite comparison and relevant regressions.

Even successful point binding leaves scope-task scheduling, resource/token
causality, nonquery/frontend/codegen/link/subprocess work, previous-session/feed
provenance and CPU-work/span accounting incomplete. This design is a next
capability, not a claim of the full map or a parallelism ceiling.
