# Demand-query-cache native acceptance, October 30, 2026

Correctness and branch-mechanism evidence only. No speedup or default-promotion claim.

`native-evidence.tar.gz` retains the complete unmodified acceptance report,
all ten raw native mechanism profiles, successful compiler build log/status,
compiler version, before SHA-256 identities and after identity verification,
acceptance log/status, and root's 630-test Python suite log. Original physical
absolute paths and raw diagnostic bytes remain intact. Compiler binaries,
fixture executables, and incremental caches are intentionally excluded.

Offline replay from any working directory, without the original scratch paths:

```sh
python3 /path/to/justrust/bench/results/demand-query-cache-20261030/audit.py
python3 /path/to/justrust/bench/results/demand-query-cache-20261030/test_audit.py -v
```

`audit.json` records the successful replay, archive SHA-256, immutable identities,
all mechanism counts, and limitations. The audit checks all 288 main compile
observations (32 revisions x nine modes), 198 dependency compiles, independent
runtime/error expectations, and exact status/stdout/stderr comparisons. It
checks same-flag persistent caches remain unchanged across revisions, mode
flags, verify-ICH, source/output paths, and edition. Ten raw profiles are
reparsed to verify SHA-256, byte counts, resolved markers, unknown-label counts,
and metadata. Eight mutation tests reject altered diagnostics, identity drift,
missing same-flag replays, persistent-cache resets, absent native markers,
conflated mechanism caches, and corrupted raw profiles.

Each of the five off processes recorded exactly one normal cache promotion
and zero skipped-promotion markers. Each on process recorded exactly one
skipped-promotion marker and zero normal promotions. Wide-demand return recorded
1,376 green recomputations on versus 1,242 off. These aggregate counts are
observational, not causal attribution to any specific missing cached value,
cache-hit proof, or a performance result.

Compiler SHA-256: `570faeb27338cea8625c4a4a7eaeef71fc75470508657ff489c3da96ce5f9144`.
Driver SHA-256: `ed21691a9db267ffb71cc3b8975a8cd89768397a7a7f82c9c839083485424efd`.
Both identities remained unchanged across acceptance.
