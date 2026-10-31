# Contributing to justrust

Thanks for helping make Rust builds faster. Contributing works like any other
GitHub project: fork, change, open a pull request. There is no CLA and no bot.

## Licensing of contributions

Which license applies depends on the directory your pull request changes. See
[LICENSING.md](LICENSING.md).

- **Everything outside `server/`:** your contribution is licensed under MIT,
  the same license the project uses.
- **`server/`:** the project distributes `server/` under FSL-1.1-ALv2, but
  your contribution to it is licensed to the project under the
  [Apache License 2.0](https://www.apache.org/licenses/LICENSE-2.0). This lets
  the maintainer relicense server code later, including for the hosted
  justrust service, and includes Apache 2.0's patent grant. You keep your
  copyright.

By submitting a pull request, you agree to these terms and confirm you have
the right to submit the contribution. The pull request template repeats this.

## Before opening a pull request

- Use `justrust check`, `justrust test`, and `justrust clippy`, not cargo
  directly.
- Add a test for every behavior change.
- Keep commits focused, and describe what changed and why.
