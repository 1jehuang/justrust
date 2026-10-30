# ra-jit-live

Edit Rust and the running program has the new code about a millisecond later. Its state stays.

The runtime half of [ra-jit](https://github.com/indexable-inc/blaze): link it, poll once per
frame, and run your program under the ra-jit daemon.

```toml
[dependencies]
ra-jit-live = "0.1"
```

```rust
let mut live = ra_jit_live::Live::start().ok();
loop {
    if let Some(outcome) = live.as_mut().and_then(|it| it.poll()) {
        eprintln!("ra-jit: {outcome}");
    }
    // ...your frame
}
```

While the program runs, the daemon also samples where its threads spend their time and
compiles the functions they are found in again with LLVM, swapping them in as patches like any
edit. `ra_jit_live::tiers()` says which functions are hot and at which level, for the program to
show. `BLAZE_TIER=off` in the program's environment turns it off.

macOS on Apple silicon and Linux x86-64; elsewhere `Live::start` says so and you still get
ra-jit's fast builds. MIT or Apache-2.0.
