# Explicit final source release: v54b

Releasing ONLY toolchain/resident/crates/ra-jit/src/main.rs and NEW docs/resident-cold-check-overlap.md to coordinator skunk. The preliminary v54 freeze remains untouched and its complete candidate is also preserved under preliminary-v54 here. original/ contains full pre-experiment main.rs and the documentation absence receipt. candidate/ contains complete final source and doc. candidate.patch is original-to-final.

Final correction: process-local LazyLock<bool> reads exact RA_JIT_COLD_CHECK_OVERLAP=1, forced as the first operation in main. No per-request environment read remains. Exact final whole-source plan equality, scoped joins before include_sources, include invalidation and original error/refusal priority remain unchanged.

rustfmt --check and git diff --check are the only executable source validation. No builds/tests/native workloads/commits. Full native correctness, error/include/macro/fallback and varied timing gates belong to root, which prepared the 30 real original/OFF/ON fixture runner. This release is permission for those gates, not promotion or measured savings evidence. SHA256SUMS is the authoritative content inventory and is verified before handoff.
