# Contributing to justrust

Thanks for helping make Rust builds faster. Contributing works like any other
GitHub project: fork, change, open a pull request. There is no CLA and no bot.

## Licensing of contributions

The project is distributed under FSL-1.1-ALv2. See
[LICENSING.md](LICENSING.md).

- **Project code and docs:** your contribution is licensed to the project
  under the [Apache License 2.0](https://www.apache.org/licenses/LICENSE-2.0).
  This lets the maintainer distribute it under FSL, under the future Apache 2.0
  conversion, and in the hosted justrust service, and includes Apache 2.0's
  patent grant. You keep your copyright.
- **Third-party paths** (`libraries/probes/`, `fixtures/adler2_adapted/upstream/`):
  preserve the applicable original license terms and notices. FSL does not
  replace them.

By submitting a pull request, you agree to these terms and confirm you have
the right to submit the contribution. The pull request template repeats this.

## Before opening a pull request

- Use `justrust check`, `justrust test`, and `justrust clippy`, not cargo
  directly.
- Add a test for every behavior change.
- Keep commits focused, and describe what changed and why.
