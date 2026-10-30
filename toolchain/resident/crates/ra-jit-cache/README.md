# ra-jit-cache

`#[ra_jit::cache]` keeps the results of a pure function across runs of a program built by
[ra-jit](https://github.com/indexable-inc/blaze), and drops them when the code the function reaches
changes.

```toml
[dependencies]
ra-jit-cache = { git = "https://github.com/indexable-inc/blaze" }
```

```rust
#[ra_jit::cache]
fn fib(n: u64) -> u64 {
    if n < 2 { n } else { fib(n - 1) + fib(n - 2) }
}
```

Arguments and the result go through serde. A comment, a moved line or an edit to code the function
does not reach keeps what was kept; an edit to the function or anything it calls computes it again.
A function that reads a static, does I/O, calls foreign code or takes something that can change
behind a shared reference is refused, with the place that made it so, and runs as written.

Under plain `cargo build` the attribute does nothing. See
[docs/fn-cache.md](https://github.com/indexable-inc/blaze/blob/main/docs/fn-cache.md).
