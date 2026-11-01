# Controlled non-speculative three-stack comparison

These 42 path-redacted shipping-client summaries are the six repetitions of seven phases in the corrected November 1, 2026 direct Cargo/justrust/Blaze comparison. Exactly seven recordings were present in each private justrust home, with no speculative prebuilds or build-daemon directories.

The harness explicitly disables hosted execution, owned-toolchain substitution, prebuilt/remote/peer downloads, telemetry and speculative build/prebuild. Cargo and justrust use the same pinned stock Cargo/rustc, while Blaze uses its native compiler. These are not default-hosted results or a comprehensive workload score.

Actual correctness passed for all 126 stack calls and 54 fresh execution nonces. Full methodology, per-phase medians/ranges, limitations, and the separately preserved initial confounded run are in `docs/three-stack-benchmarks.md`. Do not pool these summaries with `../three-stack-20261101/`.
