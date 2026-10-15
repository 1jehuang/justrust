<div align="center">

<picture>
  <source media="(prefers-color-scheme: dark)" srcset="assets/justrust-icon-dark.svg">
  <img src="assets/justrust-icon.svg" alt="justrust: a halftone gear" width="140">
</picture>

# justrust

**Rust compile as a service.**

A faster, vertically integrated Rust build toolchain optimized for agents.

[Website](https://jcode.sh/rust) · [Status](#status) · [Design](#design) · [Goal](#goal) · [Contributing and license](#contributing-and-license)

</div>

---

justrust is being rewritten from scratch as one integrated system for turning
Rust source edits into trustworthy build and test results. The rewrite will
replace the existing internals rather than wrap or extend Cargo and rustc.

Your coding agents should spend their time editing and reasoning, not waiting
for builds. We are designing the whole path around that loop, from reading
source to running tests and explaining failures.

## Status

**This branch is the clean-room rewrite, not a usable replacement yet.**

The rewrite starts on `clean-room-rewrite`. The previous implementation is
preserved at the Git tag `pre-clean-room-rewrite-2026-10-15`. Existing source,
scripts, and documentation are still present during the transition and describe
the previous system unless explicitly updated.

There are no installation or usage instructions for the rewrite yet. The
published installer and hosted service are not releases of this rewrite.
Benchmarks from the previous implementation do not establish the rewrite's
performance. New claims will require measurements of the new system.

## Design

**A fresh implementation, not another layer around the old toolchain.**

The direction is to build our own integrated Rust build system and compiler,
without Cargo, rustc, their vendored forks, or third-party dependencies in the
new implementation. That is the target architecture, not a claim that the
transition is complete or the repository is already dependency-free.

We will design compilation, dependency handling, code generation, linking,
and test execution together. Existing component boundaries are not constraints.
Local and remote execution should serve the same goal: the shortest reliable
feedback loop for the agent.

Independence of implementation does not mean dropping support for Rust projects
that use dependencies. Supported language semantics, project inputs,
build-script and proc-macro behavior, and platform ABIs are external correctness
requirements. Support will be documented and tested as it is implemented,
not assumed from the previous system.

## Goal

**The shortest possible path from a source edit to a trustworthy test result.**

Optimize the entire edit, check, build, and test loop, not just individual
compiler stages. Preserve useful diagnostics and make failures visible.
Measure cold builds and repeated edits on representative workloads, and
compare against both the previous implementation and stock tools.

A clean-room rewrite is an architectural decision, not evidence of a speedup.
Correctness tests and reproducible before-and-after measurements will determine
which designs we keep and what we can claim.

## Contributing and license

The rewrite is at its starting point. Existing contribution instructions and
architecture documents have not yet been updated for it.

The client is MIT. Existing self-hostable server and vendored sources retain
their applicable licenses and notices while present. See [LICENSE](LICENSE),
[LICENSING.md](LICENSING.md), and [NOTICE](NOTICE). The rewrite does not change
the licensing of the preserved implementation.
