# Contributing

The long version, with the rules for code and how to change the vendored rust-analyzer, is
[docs/contributing.md](docs/contributing.md). The short version:

## Build

```
git clone https://github.com/indexable-inc/blaze
cd ra-jit
nix develop            # rustup at the nightly in harness/toolchain, Python, and the LLVM 23 of `--features llvm`
rustup component add rust-src
cargo build -p ra-jit --release
```

## Test

```
cargo fmt --all -- --check
cargo test --release -p ra-jit
cd vendor/rust-analyzer && cargo test --release -p hir-ty -p hir-def -p parser -p syntax
```

The rust-analyzer fork's crates are tested in `vendor/rust-analyzer`'s own workspace: the root
workspace excludes it so that `cargo install --git` can build ra-jit (#293), and cargo only
tests a member of the workspace it runs in. `python3 harness/unittests.py` runs both, as CI does.

The real gate compares ra-jit's output with rustc's on real programs: see
[docs/harness.md](docs/harness.md). Run what fits your change and say in the pull request
what you ran and on which platform.

## Changes

- Add a reduced case that fails before your change and passes after.
- Add a line under `## [Unreleased]` in [CHANGELOG.md](CHANGELOG.md) for anything a user
  would notice.
- Credit anyone whose idea, diagnosis or patch the change carries with a `Co-authored-by:`
  trailer: [docs/contributing.md](docs/contributing.md#credit).
- Contributions are accepted under MIT OR Apache-2.0, the project's licence.

Security problems go through [SECURITY.md](SECURITY.md), not public issues.
