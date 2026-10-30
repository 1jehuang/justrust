# Native TT declarative cache acceptance, 2026-10-30

Correctness and native query reuse evidence only. No speedup claim or default promotion.

- `native-evidence.tar.gz`: full positive and literal-only negative acceptance reports, ten positive raw native profiles, compiler build/version and identity logs, CLI logs, and root's 610-test Python log. Original paths and diagnostic bytes are unchanged.
- `audit.py`: portable audit with no compiler or native fixture execution. Requires the repository harness and query-profile parser, but none of the original scratch paths.
- `audit.json`: successful audit replay, compiler/driver identities, archive hash, and inferred provider avoidance.
- `test_audit.py`: six mutation tests checking that missing modes, changed diagnostic bytes, identity changes, removed native counters, and wrong negative-control failures are rejected.

Run from any directory:

```sh
python3 /path/to/justrust/bench/results/declarative-tt-cache-20261030/audit.py
python3 /path/to/justrust/bench/results/declarative-tt-cache-20261030/test_audit.py -v
```

Positive: 51 revisions x 7 modes = 357 main compile observations, plus five literal proof processes and five independent pure-TT proof processes. Dependency compile observations are additional. Both cold proofs execute one provider. Both warm proofs avoid one provider after successful lookup, including an unrelated body edit. Both explicit off controls have no markers. All ten raw profiles are reparsed and hash-checked by the audit.

Negative: the frozen literal-only driver passes all 357 semantic modes and the literal positive proof, then fails TT calibration with positive ineligible and zero eligible/lookup/provider/success events. The audit checks that exact failure rather than accepting arbitrary errors.

Compiler SHA-256: `570faeb27338cea8625c4a4a7eaeef71fc75470508657ff489c3da96ce5f9144`.
Driver SHA-256: `ac116b9398e1f5c677e73952b8719be9d9e47190fe61fbaeba9b7cf270739595`.

Raw profiles for the negative run were not requested for this bundle. Its full report retains profile identities and counters, and its complete CLI failure log is included. Positive profiles provide independently reparseable native evidence. Compiler binaries, incremental caches, and fixture executables are intentionally excluded.
