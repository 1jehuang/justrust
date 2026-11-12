# Contributing to justrust

Thanks for helping make Rust builds faster. Contributing works like any other
GitHub project: fork, change, open a pull request. There is no CLA and no bot.

## Licensing of contributions

Which license applies depends on the directory your pull request changes. See
[LICENSING.md](LICENSING.md).

- **Project code outside imported `toolchain/` sources:** your
  contribution is licensed under MIT, the same license the project uses.
- **Imported sources under `toolchain/`:** preserve the applicable component
  and file-level license terms and third-party notices. Vendoring does not
  replace them with the root MIT license.

By submitting a pull request, you agree to these terms and confirm you have
the right to submit the contribution. The pull request template repeats this.

## Before opening a pull request

- Use `justrust check`, `justrust test`, and `justrust clippy`, not cargo
  directly.
- Add a test for every behavior change.
- Keep commits focused, and describe what changed and why.
