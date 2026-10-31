# Changelog

All notable changes. Versions follow [semver](https://semver.org); while in
0.x, output formats and the `~/.justrust` layout may change in any release.

## 0.1.0-alpha.1

First public release. Linux only.

- **Agent interface.** `justrust check|test|build|clippy|run` take cargo's
  arguments, print errors and failing tests in full plus the first few
  warnings (`JUSTRUST_MAX_WARNINGS`), drop progress noise, and end with a
  verdict and a timing report with detected waste. Exit code is cargo's.
- **Build recorder.** Per-unit rustc timings (frontend, codegen, link,
  incremental cache), rebuild reasons, lock waits, test time, CPU, memory,
  and pressure stalls. `justrust runs`, `show`, `log`, `status`.
- **`cargo` proxy.** `justrust install` records every cargo build without
  changing its output or invalidating caches. Fails open.
- **Per-agent build slots and a shared dependency cache** so parallel agents
  stop contending for the target dir lock (`justrust slots`, `justrust cache`).
- **`justrust split`.** Suggests which code to move into its own crate to stop
  rebuild cascades, from recorded edits and the module graph.
- **`justrust history`.** Mines Jcode session history for cargo calls.
- **Experimental remote builds.** `justrust remote` routes `check`, `clippy`,
  and `test` to an AWS machine or ssh host when measured faster. Off unless a
  machine is configured (`JUSTRUST_REMOTE=off` to force local).
- **Other platforms.** On macOS and Windows, `justrust check|test|build|clippy|run`
  run plain cargo without recording. Other commands are unavailable.
