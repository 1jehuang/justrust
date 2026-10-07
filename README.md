# justrust

An attempt at a faster all-in-one Rust compiling solution for coding agents.

Coding agents run `cargo check` and `cargo test` hundreds of times a day, and
spend most of that time waiting on the compiler rather than running tests.
justrust aims to make the agent edit, check, and test loop as fast as possible.
Every optimization has to be justified by measurements from real agent sessions.

## Status

Early. The first tool is `justrust history`, which measures where build and test
time actually goes in recorded [Jcode](https://github.com/1jehuang/jcode)
sessions. See [FINDINGS.md](FINDINGS.md) for the first results.

```sh
cargo run --release -- history                    # all sessions in ~/.jcode/sessions
cargo run --release -- history --repo jcode-desktop
cargo run --release -- history --dump calls.jsonl # raw per-call records
```

## Direction

Based on the data so far, the plan is roughly:

1. **Measure.** Turn session history and live builds into a benchmark of real
   agent edit loops (`history`, then a replay harness).
2. **Check before test.** About a fifth of agent build time ends in compile
   errors. Report errors from a fast `check` instead of waiting on test-binary
   codegen and linking.
3. **Run only the affected tests.** Agents usually filter to a handful of
   tests, but still pay the cost of rebuilding and linking one huge test binary.
4. **Keep the compiler warm.** Use a persistent build daemon so incremental
   state, metadata, and file watching do not restart from zero on every call.
5. **Share artifacts.** Use a content-addressed cache of dependency and
   workspace artifacts across checkouts, worktrees, agents, and machines.

## License

MIT
