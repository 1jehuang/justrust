# justrust server

The self-hostable, single-tenant justrust remote server: a remote build
executor plus an artifact cache, for teams that want to run justrust's remote
builds on their own machines. No code has been written yet. The design is in
[../docs/remote-strategy.md](../docs/remote-strategy.md) and
[../docs/remote.md](../docs/remote.md).

## License

Everything in this directory is licensed under the Functional Source License,
Version 1.1, ALv2 Future License ([LICENSE](LICENSE)). You may use, modify,
and self-host it for any purpose except offering it to others as a competing
commercial product or service. Each release becomes Apache 2.0 two years after
it is published.

The rest of the repository is MIT. See [../LICENSING.md](../LICENSING.md).

Contributions to this directory are licensed to the project under the Apache
License 2.0. See [../CONTRIBUTING.md](../CONTRIBUTING.md).

## When adding the crate

- Set `license-file = "LICENSE"` in `server/Cargo.toml`, not `license = "MIT"`.
- Keep this directory out of the MIT `justrust` package. The root
  `Cargo.toml` already excludes `server/`.
- The global shared cache, prebuild fleet, tenant isolation, and billing
  belong in the private hosted-service repository, not here.
