# Legal review brief

Questions for a lawyer before the hosted justrust service launches or
`server/` accepts its first outside contribution. Nothing here has been
reviewed yet. Update the status column as items are answered.

## Setup to review

| Part | License | Files |
|---|---|---|
| Client, local tooling, protocol, docs | MIT | `LICENSE` |
| `server/` (self-hostable single-tenant server) | FSL-1.1-ALv2, unmodified official text | `server/LICENSE` |
| Hosted multi-tenant service | Proprietary, separate private repository | none here |
| Inbound contributions | MIT outside `server/`, Apache 2.0 for `server/`, stated without a CLA | `CONTRIBUTING.md`, `.github/pull_request_template.md` |
| Attribution | Contributor and third-party notices | `NOTICE` |

## Questions

| # | Question | Status |
|---|---|---|
| 1 | Is the inbound Apache 2.0 notice in `CONTRIBUTING.md` plus the pull request template enough to bind contributors, given GitHub's Terms of Service default that contributions are licensed under the repository license? Is a click-through CLA needed, and at what point? | Open |
| 2 | Under that notice, may the maintainer use `server/` contributions in the closed hosted service and relicense them? | Open |
| 3 | Does FSL's Competing Use definition clearly block a third party from offering hosted remote Rust builds built on `server/`? Does it also restrict our own MIT client in any way? | Open |
| 4 | Is mixing MIT and FSL directories in one repository sound, and does `exclude = ["server/"]` in the root `Cargo.toml` keep FSL code out of the MIT crate published to crates.io? | Open |
| 5 | Does the `NOTICE` file satisfy MIT and Apache 2.0 attribution for contributor code used in the hosted service? | Open |
| 6 | Should the licensor be an entity rather than an individual before launch, and how are the copyright notices and existing licenses transferred? | Open |
| 7 | Is a trademark registration for "justrust" advisable before launch? | Open |
| 8 | Hosted service terms: data handling for uploaded source code, liability for build output, and terms of service. | Open |

## Decisions already made

See [remote-strategy.md](remote-strategy.md#open-source-and-licensing) for
the reasoning: open core, FSL for the server, inbound Apache 2.0 with no CLA
to keep contributing frictionless, and a switch to a CLA if review requires.
