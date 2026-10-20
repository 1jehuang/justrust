# Legal review brief

Questions for a lawyer before the hosted justrust service launches or the
compiler accepts its first outside contribution. Nothing here has been
reviewed yet. Update the status column as items are answered.

## Setup to review

| Part | License | Files |
|---|---|---|
| Compiler on `main` (engine, prelude, tests, tooling, docs) | FSL-1.1-ALv2, unmodified official text (MIT before 2026-10-19) | `LICENSE` |
| Client on `stable` | Per that branch's `LICENSING.md` | `stable:LICENSE` |
| Third-party paths on `main` (`libraries/probes/`, Adler2 fixtures, `unicode-ident`) | Original licenses retained | `NOTICE`, `THIRD-PARTY-LICENSES.md` |
| Hosted multi-tenant service | Proprietary, separate private repository | none here |
| Inbound contributions | Apache 2.0 for all project code, stated without a CLA | `CONTRIBUTING.md`, `.github/pull_request_template.md` |
| Attribution | Contributor and third-party notices | `NOTICE` |

## Questions

| # | Question | Status |
|---|---|---|
| 1 | Is the inbound Apache 2.0 notice in `CONTRIBUTING.md` plus the pull request template enough to bind contributors, given GitHub's Terms of Service default that contributions are licensed under the repository license? Is a click-through CLA needed, and at what point? | Open |
| 2 | Under that notice, may the maintainer distribute contributions under FSL, use them in the closed hosted service, and relicense them? | Open |
| 3 | Does FSL's Competing Use definition clearly block a third party from offering hosted remote Rust builds built on the compiler? Does it unduly deter internal company and CI use? | Open |
| 4 | Is relicensing `main` from MIT to FSL sound given that earlier MIT copies keep their grant, and are the third-party carve-outs (`libraries/probes/`, Adler2 fixtures) correctly scoped? | Open |
| 5 | Does the `NOTICE` file satisfy Apache 2.0 and third-party attribution for contributor code used in the hosted service? | Open |
| 6 | Should the licensor be an entity rather than an individual before launch, and how are the copyright notices and existing licenses transferred? | Open |
| 7 | Is a trademark registration for "justrust" advisable before launch? | Open |
| 8 | Hosted service terms: data handling for uploaded source code, liability for build output, and terms of service. | Open |

## Decisions already made

See [remote-strategy.md](remote-strategy.md#open-source-and-licensing) for
the reasoning: FSL outbound for the whole compiler, inbound Apache 2.0 with no CLA
to keep contributing frictionless, and a switch to a CLA if review requires.
