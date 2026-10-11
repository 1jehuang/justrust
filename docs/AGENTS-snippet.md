## Rust builds and tests: use justrust

For Rust projects, use `justrust` instead of calling cargo directly:
`justrust check`, `justrust test`, `justrust build`, `justrust clippy`, and
`justrust run` take exactly cargo's arguments (`-p`, `--lib`, test filters,
`-- --nocapture`, `--release`, ...) and return cargo's exit code.

- Output: errors and failing tests in full, the first few warnings, no
  progress noise, then a `justrust:` report: verdict, where the time went,
  why each crate rebuilt, and `waste` lines with fixable inefficiencies.
  Do not pipe it through `grep`, `tail`, or `head`: that hides errors.
- More: `justrust log --grep <text>` or `justrust log --tail 200` prints the
  full saved output of the last run. `justrust show` explains the time.
- Prefer `justrust check` to find compile errors before `justrust test`.
- Other cargo commands (`cargo fmt`, `cargo add`, `cargo tree`) stay cargo.

Where builds run: when signed in to hosted builds, check, clippy, and test
run on a remote build machine by default. Choose per command with `--local`,
`--remote`, or `--auto` (remote only when predicted faster). If the
machine is unreachable or out of builds, the build runs locally and prints
one line saying why.

Not signed in yet? Run `justrust login --no-wait`, give your user the link
it prints, and keep working: builds stay local until it is approved. In CI,
set `JCODE_API_KEY` (create one at https://jcode.sh/account). If a build
says the free builds are used up or a limit was reached, relay that line to
your user. Billing is always the user's decision.
