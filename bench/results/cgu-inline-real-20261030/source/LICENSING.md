# Licensing

justrust is open core. Each part of the project has one license, decided by
its directory. The reasoning is in
[docs/remote-strategy.md](docs/remote-strategy.md#open-source-and-licensing).

| Path | License | Covers |
|---|---|---|
| Everything not listed below | [MIT](LICENSE) | The `justrust` CLI, local build tooling, shared dependency cache, remote machine management, docs |
| `protocol/` (future) | MIT | Wire protocol and cache key format, so other tools can implement them |
| `server/` | FSL-1.1-ALv2 ([server/LICENSE](server/LICENSE)) | Self-hostable single-tenant remote executor and artifact cache |

The hosted multi-tenant service (global shared cache, prebuild fleet, tenant
isolation, billing) is not in this repository and is not open source.

## What FSL allows

The Functional Source License (FSL-1.1-ALv2) applies only to `server/`. You
may read, modify, and run it for any purpose, including running it for your
own company, except offering it to others as a product or service that
competes with justrust's hosted service. Each release converts to Apache 2.0
two years after it is published. Full text: <https://fsl.software>.

`server/` holds the official FSL-1.1-ALv2 text, unmodified. The root
`Cargo.toml` excludes `server/` so FSL code never ships in the MIT crate. When
the server crate is added, set `license-file = "LICENSE"` in its
`Cargo.toml`.

## Contributing

Contributing is a normal pull request. There is no CLA, bot, or required
sign-off.

| Contribution to | Licensed to the project under |
|---|---|
| Anything outside `server/` | MIT |
| `server/` | Apache License 2.0 |

`server/` contributions come in under Apache 2.0 even though `server/` goes
out under FSL. Apache 2.0 lets the maintainer relicense that code, including
for the hosted service, and carries a patent grant. Contributors keep their
copyright. The terms are in [CONTRIBUTING.md](CONTRIBUTING.md) and repeated
in the pull request template, so they appear in every pull request.

If the server becomes commercially important, or legal review finds the
notice insufficient, switch `server/` to a click-through CLA. That change
applies only to new contributions.

## Attribution

[NOTICE](NOTICE) lists contributors and any third-party code. Add each new
contributor when their first pull request is merged, and ship the file with
every copy, including the hosted service.

## Legal review

Open questions for a lawyer are tracked in
[docs/legal-review.md](docs/legal-review.md).
