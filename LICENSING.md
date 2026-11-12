# Licensing

The justrust client is open source. Licenses are scoped by directory; imported components
also retain their file-level licenses and third-party notices. The reasoning is in
[docs/remote-strategy.md](docs/remote-strategy.md#open-source-and-licensing).

| Path | License | Covers |
|---|---|---|
| Everything not listed below | [MIT](LICENSE) | The `justrust` CLI, local build tooling, shared dependency cache, remote machine management, docs |
| `protocol/` (future) | MIT | Wire protocol and cache key format, so other tools can implement them |
| `toolchain/` imported sources | Component and file-level licenses retained in each tree, including MIT/Apache-2.0 and third-party notices | Owned compiler, Rust libraries, Cargo, and included tools; see `toolchain/sources.json` for import provenance |

Vendoring does not relicense imported code. Preserve the component `LICENSE*`,
`COPYRIGHT`, attribution, and third-party notices when changing or distributing
these trees. The root MIT license is not a replacement for those notices.
The client crate excludes `toolchain/` from its package.

The hosted multi-tenant service (global shared cache, prebuild fleet, tenant
isolation, billing) is not in this repository and is not open source. There is
no self-hostable server: the planned `server/` directory was removed on
2026-11-12 before any code was written.

## Contributing

Contributing is a normal pull request. There is no CLA, bot, or required
sign-off.

| Contribution to | Licensed to the project under |
|---|---|
| Project code outside imported `toolchain/` sources | MIT |
| Imported component sources under `toolchain/` | The applicable component/file license terms |

Contributors keep their copyright. The terms are in
[CONTRIBUTING.md](CONTRIBUTING.md) and repeated in the pull request template,
so they appear in every pull request.

## Attribution

[NOTICE](NOTICE) lists contributors and any third-party code. Add each new
contributor when their first pull request is merged, and ship the file with
every copy, including the hosted service.

## Legal review

Open questions for a lawyer are tracked in
[docs/legal-review.md](docs/legal-review.md).
