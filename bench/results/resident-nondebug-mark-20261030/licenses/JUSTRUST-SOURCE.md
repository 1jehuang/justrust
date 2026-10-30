# Experimental resident engine source

This is an explicitly **Blaze-derived** resident frontend experiment, not an independently implemented compiler or a demonstrated improvement over Blaze.

## Import provenance

- Origin: `https://github.com/indexable-inc/blaze.git`
- Release tag: `v0.20261027.0`
- Exact commit: `5d4f78f7577cf7170993b74f735099dd03f5c6ca`
- Imported using `git archive`, without a nested Git repository or submodule.
- 6,210 imported files were checked byte-for-byte against the pinned checkout before any local changes.
- The only four excluded tracked files were `AGENTS.md`, `CLAUDE.md`, `vendor/rust-analyzer/AGENTS.md`, and `vendor/rust-analyzer/CLAUDE.md`. Repository-root agent policy remains authoritative. Technical documentation, fixtures, attribution and license files are retained.
- Root `LICENSE-MIT` SHA256: `6ee1369d459d09f1c8a552c6faa7aa29119df6c240014594c4fe28d8e6af72ab`.
- Root `LICENSE-APACHE` SHA256: `62c7a1e35f56406896d7aa7ca52d0cc0d272ac022b5d2796e7d6905db8a3636a`.

The original package names, upstream authorship, vendored dependency notices, and MIT/Apache licensing are preserved. Local development belongs in this ordinary owned source tree. Upstream internal compatibility and rebasing are not architectural requirements.

Import-time local additions are this provenance document and three exact-byte exceptions appended to `vendor/rust-analyzer/.gitattributes`. They prevent its inherited `text=auto eol=lf` rule from normalizing existing CRLF in three upstream files. All other retained upstream blobs and file modes match the pinned commit.

## Status and boundaries

At import this source has **not been built or correctness-validated here**. It is not selected by the shipping justrust client, not installed over any tool, and not enabled by default. Source import alone is not an optimization.

The experiment will need its own explicit executable identity and private cache namespace, native process/worker dispatch, exact matching toolchain, and differential execution/error/recovery checks before prospective timing. Existing upstream worker protocols must not be passed to stock tools. Stock fallback must remain separate and must not hide unsupported behavior.

The benchmarked reference Blaze has known fresh-store failures on the pinned workload. Successful seeded warm-store results do not establish cold support or full Rust correctness. Any derived engine inherits open correctness questions until independently tested. No code from this source belongs in the separate clean-room compiler checkout.

Build provenance must explicitly identify the fork. The imported build script can otherwise discover the enclosing justrust Git HEAD and report it as an upstream commit. Do not use such an ambiguous build as evidence.
