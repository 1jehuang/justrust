# Compiler-private storage stride and Box source integration

Status: focused standalone acceptance passed on the captured `0fd5a8ed` parent.

## Frozen contract

`mem::storage_stride<T>` is private to the embedded bootstrap library. It
computes the existing compiler-owned value-carrier extent for an exact closed
canonical `T`. This is not Rust's public `size_of`, native pointer arithmetic,
a public layout/niche guarantee, allocator authority, or an external ABI layout.

The original embedded `mem::storage_stride` declaration alone issues the query.
Its immutable witness binds query kind, declaration/snapshot, ordered rigid
parameter and exact substituted type. Foreign source owners, public access,
changed kind/type and foreign snapshot/owner must reject. A genuinely symbolic
query remains symbolic until ordinary instance substitution, never a guessed
integer. Closed lowering uses the existing backend carrier-layout calculation.

Ordinary source `Box<T>` uses this private extent for its existing `raw_alloc`
and `raw_free` calls. Allocation failure checks, mmap/munmap source, typed
initialization, moves and normal Drop remain unchanged. No new memory-effect
producer or unconditional entry gate is introduced. Public `size_of` and
native-memory compatibility debt remain separate and open.

## Graph / IR shape

The existing typed TypeAlignment query gains an authenticated private-stride
kind. Source replay compares the kind as well as original canonical owner and
queried type. The query travels through existing source/check/instance/lowering
facts. CHECK remains codegen-free. No new graph kind, global join, mutable fact,
cache, budget or ceiling is added. Node137 and opaque/closure/entry consumers
remain byte-identical to the parent.

## Acceptance gates

- Focused stock CHECK/BUILD/run at workers1/8: u64 Box initialization, mutation,
  normal drop, owned Box move with non-Copy Pair, narrow/wide integer pointees.
- Source authority: foreign intrinsic owners and public private-helper access.
- Sealed-query unit: query-kind swap, foreign owner and substituted-type tamper.
- Graph-shape control: CHECK emits no executable/codegen or memory-origin137.
- Mandatory graph_integrity unchanged ceilings.

All gates above passed for frozen tree `c736bc04` on parent `0fd5a8ed`.
Matching nightly `1d81eb4ad` LOCAL check--tests passed in task `947306y4dw`.
Task `582233krek` passed the three focused integration tests, sealed-query unit,
and all 14 graph-integrity tests with unchanged ceilings (graph receipt
`20261019-195406439-1685308`). Stock and native Box behavior matched at workers
1 and 8. Scalar values are bound before formatting because temporary-rooted
format arguments remain a separate unsupported boundary.
Earlier larger private-memory experiments are not acceptance evidence here.
Full Box/Vec/String, public layout/niches and general pointer compatibility are
not completed by this slice.
