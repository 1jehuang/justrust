# Licensing

justrust is open core. Each part of the project has one license, decided by
its directory. The reasoning is in
[docs/remote-strategy.md](docs/remote-strategy.md#open-source-and-licensing).

| Path | License | Covers |
|---|---|---|
| Everything not listed below | [MIT](LICENSE) | The `justrust` CLI, local build tooling, shared dependency cache, remote machine management, docs |
| `protocol/` (future) | MIT | Wire protocol and cache key format, so other tools can implement them |
| `server/` (future) | FSL-1.1-ALv2 (`server/LICENSE`) | Self-hostable single-tenant remote executor and artifact cache |

The hosted multi-tenant service (global shared cache, prebuild fleet, tenant
isolation, billing) is not in this repository and is not open source.

## What FSL allows

The Functional Source License (FSL-1.1-ALv2) applies only to `server/`. You
may read, modify, and run it for any purpose, including running it for your
own company, except offering it to others as a product or service that
competes with justrust's hosted service. Each release converts to Apache 2.0
two years after it is published. Full text: <https://fsl.software>.

When `server/` is created, add `server/LICENSE` with the FSL-1.1-ALv2 text
and set `license-file = "LICENSE"` in its `Cargo.toml`.

## Contributing

Sign off each commit to certify the
[Developer Certificate of Origin](https://developercertificate.org):

```text
git commit -s
```

Contributions are licensed under the license of the directory they change.
No CLA is required.
