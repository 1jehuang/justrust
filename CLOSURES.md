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
source function is unused. There is no body substitution, shared result slot,
new graph node kind or relaxed graph-integrity ratchet.

The environment of this increment is empty. Stored closure values, captures,
parameter/result inference, non-scalar returns, generic enclosing bodies and
closures originating in macro expansions remain unsupported. Unqualified calls
inside a closure body are deliberately unsupported until capture-aware callable
resolution exists. Use an explicit `crate::function` path. Unqualified free
value references reject before constructor lookup, so a captured local cannot
silently become a same-spelled module constructor.

`tests/closures.rs` compares whole programs against stock Rust for native calls,
multiple anonymous identities, nesting, early returns, argument order, mutable
reference parameters and non-Copy arguments. Negative controls cover invalid
unused bodies, move reuse and loan conflicts. These reference parameters are
not closure captures. Additional tests inspect distinct per-callable graph
keys/source anchors and bound closure nesting to 16 live frames, independently
of the number of sibling closures. The ordinary parser depth gate is unchanged.

Remaining next increment: a typed environment whose by-ref/mutable/move fields
are created once at closure construction, genuine closure-local callable
identity and per-call local storage, unused body ownership verification, and
captured-loan last-use propagation. Generic `Fn`/`FnMut`/`FnOnce` bounds and the
full closures/iterators program remain open. This milestone makes no speed
claim and does not promote the frozen full-program closure fixture.
