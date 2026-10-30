# Precise diagnostics before cold-load fallback refusal

The experimental licensed Blaze-derived resident engine checks all owned source modules when no rustc checked the build. Previously, `cold.refused_reached` could return the generic MIR `HasErrors` refusal before this existing source check ran. A first invalid build therefore hid the useful source diagnostic.

The change moves only that generic fallback after the existing whole-source check. The fallback still runs before linking or execution. Builds that do not qualify for the whole-source check retain the same refusal. Source checking, diagnostic construction, cache policy and invalid-code rejection are unchanged.

## Actual native acceptance, October 30, 2026

Frozen v30 passed the native release check, build, and full resident regression suite. Root-owned artifacts are in `candidate-build-v30` and `candidate-unit-v30` under the session scratch root.

A strict eight-call differential used the frozen v28 engine and v30 candidate, identical pinned 63-file source and toolchain, separate fresh private stores, and error/recovery/error/recovery transitions. Every error required status 101, E0308 and its current unique source nonce. Every recovery required the exact four tests and a current executed runtime marker.

At `/home/jeremy/p`, v28's initial cold error failed the precise-diagnostic oracle, while its later warm error passed. Both v28 recoveries passed. All four v30 observations passed, including the initial cold error and both recoveries. Full source, toolchain, tools and original matrix receipts remained unchanged and source restoration passed. The runner exits 1 because the historical baseline cold error is deliberately not admitted, not because candidate acceptance failed.

An earlier differential at `/home/jeremy/rstp03` positively exercised both candidate error diagnostics, but recovery in both engines hit the Unix socket path-length limit. That entire failed setup is retained separately. The successful rerun changed only the output root to shorten private socket paths, not the runner, fixture, oracle or engine.

This is a bounded diagnostic correctness fix, not a performance improvement or an accepted comprehensive state matrix. Original matrix failures remain rejected. The source-derived resident engine is not the clean-room compiler and is not installed over the shipping client.
