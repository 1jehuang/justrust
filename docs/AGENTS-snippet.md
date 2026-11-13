## Rust builds and tests: use justrust

For Rust projects, use `justrust` instead of calling cargo directly:
`justrust check`, `justrust test`, `justrust build`, `justrust clippy`, and
`justrust run` take exactly cargo's arguments (`-p`, `--lib`, test filters,
`-- --nocapture`, `--release`, ...) and return cargo's exit code.

- Output: errors and failing tests in full, the first few warnings, no
  progress noise, then a `justrust:` report: verdict, where the time went,
  why each crate rebuilt, and `waste` lines with fixable inefficiencies.
  A `habit` line names a cost this project keeps paying (a slow test you
  keep running, a crate every edit rebuilds) and how to stop paying it.
  Do not pipe it through `grep`, `tail`, or `head`: that hides errors.
- More: `justrust log --grep <text>` or `justrust log --tail 200` prints the
  full saved output of the last run. `justrust show` explains the time,
  from routing and source sync to compiling and tests.
  `justrust profile --here --days 7` shows where this project's build wait
  goes across runs.
- Prefer `justrust check` to find compile errors before `justrust test`.
- Rerunning an identical passing `justrust test` with no source, env or
  command change reuses its result instantly (it says so). Add `--rerun` when
  a test reads files outside the repo or the network and must run again.
- Other cargo commands (`cargo fmt`, `cargo add`, `cargo tree`) stay cargo.

Where builds run: when signed in to hosted builds, builds run on hosted
machines and this machine does not compile. A hosted failure is not replaced
by a local compile. When one happens:

1. Find the cause. `justrust remote status`, `justrust log --tail 200` and a
   rerun tell a problem on this side (network, sign-in, a path outside the
   synced roots, a full disk) from a hosted one. Fix what you can and rerun.
2. If the hosted service itself is at fault, report it:
   `justrust feedback --kind infra-urgent "<command, error line, what you
   tried>"` when it blocks your work (this emails the maintainer), or
   `--kind infra` when it does not.
3. Only when it cannot be fixed from here, rerun with `--local="<why>"`.

Do not start with `--local`, and do not run `justrust local on` yourself:
that setting is your user's. Other feedback is welcome too:
`justrust feedback --kind bug|idea|question "<message>"`. Never put secrets
or private source in a message.

Not signed in yet? Run `justrust login --no-wait`, give your user the link
it prints, and keep working: builds stay local until it is approved. In CI,
set `JCODE_API_KEY` (create one at https://jcode.sh/account). If a build
says a card is needed, the trial or builds are used up, or a limit was reached, relay that line to
your user. Billing is always the user's decision.
