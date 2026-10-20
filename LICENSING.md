# Licensing

The justrust compiler on `main` is source-available under the Functional
Source License, FSL-1.1-ALv2 ([LICENSE](LICENSE)). Each release converts to the
Apache License 2.0 two years after it is published. Contributions come in
under Apache 2.0 (see [Contributing](#contributing)): permissive inbound, FSL
outbound.

| Path | License | Covers |
|---|---|---|
| Everything not listed below | [FSL-1.1-ALv2](LICENSE) | The compiler, engine, prelude library, tests, benchmarks, tooling, and docs |
| `fixtures/adler2_adapted/upstream/` | 0BSD ([upstream/LICENSE-0BSD](fixtures/adler2_adapted/upstream/LICENSE-0BSD)), original copyright retained | Unmodified Adler2 sources used only as a stock-rustc oracle |
| `libraries/probes/*/` | MIT OR Apache-2.0, Rust Project Contributors (`LICENSE-MIT`, `LICENSE-APACHE`, `COPYRIGHT` in each directory) | Source-locked original `core` items, pinned in `libraries/source-lock.json` |
| Third-party crates (`unicode-ident`) | Their own licenses, see [THIRD-PARTY-LICENSES.md](THIRD-PARTY-LICENSES.md) | Utility dependencies |

FSL does not relicense third-party code. Preserve the original `LICENSE*`,
`COPYRIGHT`, attribution, and notice files when changing or distributing those
paths. Files adapted from third-party sources keep their attribution header
and remain available under the original terms for the adapted portions.

Earlier history was published under MIT. Copies obtained under MIT before this
change keep their MIT grant. The `stable` branch (the shipping justrust client)
is licensed separately; see `LICENSING.md` on that branch.

## What FSL allows

You may read, modify, and run the software for any purpose, including inside
your own company, internal builds and CI, research, and non-commercial
education. You may not offer it to others as a product or service that
competes with justrust, such as a hosted or remote Rust compile service built
on this code. Each release becomes Apache 2.0 two years after it is published.
Full text: <https://fsl.software>. [LICENSE](LICENSE) holds the official
FSL-1.1-ALv2 text, unmodified.

## Contributing

Contributing is a normal pull request. There is no CLA, bot, or required
sign-off.

| Contribution to | Licensed to the project under |
|---|---|
| Project code and docs | Apache License 2.0 |
| Third-party paths listed above | Their applicable original license terms |

Contributions come in under Apache 2.0 even though the project goes out under
FSL. Apache 2.0 lets the maintainer distribute contributions under FSL, under
the future Apache 2.0 conversion, and in the hosted service, and it carries a
patent grant. Contributors keep their copyright. The terms are in
[CONTRIBUTING.md](CONTRIBUTING.md) and repeated in the pull request template,
so they appear in every pull request.

If the project becomes commercially important, or legal review finds the
notice insufficient, switch to a click-through CLA. That change applies only
to new contributions.

## Attribution

[NOTICE](NOTICE) lists contributors and third-party code. Add each new
contributor when their first pull request is merged, and ship the file with
every copy, including the hosted service.

## Legal review

Open questions for a lawyer are tracked in
[docs/legal-review.md](docs/legal-review.md).
