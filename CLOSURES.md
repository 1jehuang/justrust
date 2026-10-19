# Closure callable foundation

This is not full closure support or full Rust compatibility.

The first genuine executable increment admits immediately invoked,
noncapturing closures with explicit parameter types, an explicit scalar/unit
return type and a braced body, for example:

```rust
fn main() {
    let answer = (|x: i32| -> i32 { x + 1 })(41);
    assert_eq!(answer, 42);
}
```

The parser retains each body as a separate anonymous `Function` declaration.
Its unnameable identity is deterministic in source traversal order and retains
its actual source-site span and module. Anonymous declarations are not inserted
into module or import lookup. Calls use the existing checked-call IR, including
ordered once-only argument evaluation. The normal per-declaration graph nodes
universally check body typing, CFG, moves and loans, even when the enclosing
source function is unused. There is no body substitution, shared result slot
or relaxed graph-integrity ratchet.

The next executable increment admits shared scalar captures of explicitly typed
parameters of the immediate parent free function (including an anonymous
parent's explicit parameters):

```rust
fn run(n: i32) -> i32 {
    (|x: i32| -> i32 { n + x })(2)
}
fn main() { assert_eq!(run(40), 42); }
```

Per-owner `BODY_LEXICAL` (116) and per-source-site `CAPTURE_ENV` (117) graph
facts retain original parameter binding occurrences, declaration/snapshot
identity, first-use field order and scalar layout. They contain immutable data,
not compiler contexts or mutable caches. Their dependencies are acyclic:
capture layout reads the owner's lexical fact and signatures, never its checked
body. Both the parent and anonymous checked bodies consume the layout handle.

The immediate-call ABI flattens the environment into private ordinary shared
reference parameters. Each real loan is created once, before explicit call
arguments, and each capture read dereferences the checked field parameter.
This does not add general references-in-aggregates support. Normal ownership
checking rejects writes and mutable borrows of a captured original parameter
while arguments evaluate, and admits modification after the immediate call.
Repeated uses deduplicate the original binding. Unused captured bodies are
independently checked for type errors and non-Copy argument move reuse.

Stored closure values, local/inferred binding captures, mutable/move captures,
parameter/result inference, non-scalar returns, generic enclosing bodies and
closures originating in macro expansions remain unsupported. Shadowed parent
parameters explicitly reject rather than being rebound by spelling. Unqualified calls
inside a closure body are deliberately unsupported until capture-aware callable
resolution exists. Use an explicit `crate::function` path. Unqualified free
value references reject before constructor lookup, so a captured local cannot
silently become a same-spelled module constructor.

`tests/closures.rs` compares whole programs against stock Rust for native calls,
multiple anonymous identities, nesting, early returns, argument order, mutable
reference parameters and non-Copy arguments. Negative controls cover invalid
unused bodies, move reuse and loan conflicts. These reference parameters are
not closure captures. Shared-capture programs additionally exercise field order,
deduplication, nesting, last use and call-argument loan conflicts. Additional tests inspect distinct per-callable graph
keys/source anchors and bound closure nesting to 16 live frames, independently
of the number of sibling closures. The ordinary parser depth gate is unchanged.

Remaining increments include general typed environments with local, mutable and
move fields, stored closure-local callable identity and per-call local storage,
and returned-reference captured-loan lifetime propagation. Generic `Fn`/`FnMut`/`FnOnce` bounds and the
full closures/iterators program remain open. This milestone makes no speed
claim and does not promote the frozen full-program closure fixture.
