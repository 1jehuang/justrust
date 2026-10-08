## Rust builds and tests: use justrust

For Rust projects, use `justrust` instead of calling cargo directly for checks,
tests, builds, lints, and runs:

| Instead of        | Use                |
|-------------------|--------------------|
| `cargo check ...` | `justrust check ...` |
| `cargo test ...`  | `justrust test ...`  |
| `cargo build ...` | `justrust build ...` |
| `cargo clippy ...`| `justrust clippy ...`|
| `cargo run ...`   | `justrust run ...`   |

- Arguments are exactly cargo's (`-p`, `--lib`, test filters, `-- --nocapture`, `--release`, ...).
- Output is already compact: errors and failing tests in full, the first few
  warnings, no progress noise, then a `justrust:` verdict line. Do not pipe it
  through `grep`, `tail`, or `head`. That hides errors and is never needed.
- The exit code is cargo's exit code.
- Need more? `justrust log --grep <text>` or `justrust log --tail 200` prints the
  full saved output of the last run. `justrust show` explains where the time went.
- Prefer `justrust check` to find compile errors before running `justrust test`.
- Other cargo commands (`cargo fmt`, `cargo metadata`, `cargo tree`, `cargo add`)
  stay as plain `cargo`.
