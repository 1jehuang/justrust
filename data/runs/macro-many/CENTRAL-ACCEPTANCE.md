# Central zero/many item-product acceptance

Validated merge snapshot: `ef27b76612ca2ae5d555eb1ceb3a67d2e63cabcf`, combining the published generic Drop baseline `ef39779f5` with the isolated macro implementation `b3cbcf2e2`.

- Formatting passed.
- Local stock-dependent item-source macros, item authority, and graph-integrity suites: **31/31**, run `20261018-212431504-4009563`.
- Frozen full local `JUSTRUST_TEST_JOBS=4 justrust test --local --no-fail-fast`: **3,415 passed, zero failed**, 188 binaries, run `20261018-212558280-4050351`. No isolated retry warning appeared.
- Strict all-target local clippy passed with zero warnings, run `20261018-213626324-241369`.

Local validation was deliberate: hosted storage exhaustion had been verified. After narrowly removing audited inactive generated build artifacts, a separate forced hosted check passed in `20261018-213725587-269201`, explicitly reporting `ok on hosted`. This does not change the local routing of the full suite or lint. Timing was heavily contended and is not a speed measurement. The source snapshot remained frozen throughout the full suite and lint.

The increment admits zero or multiple nongeneric function products per supported item macro expansion, with exact completion, per-function source identity, and coverage seals. It does not admit arbitrary generated item forms, proc macros, exported macros, or full hygiene. The real-program corpus remains **3/20**, and full Rust is incomplete.

The selected records are redacted stable-client exports. Per-binary timing lists are summarized as `tests.binary_count`; aggregate verdicts, ids, diagnostics, and resources remain. Original stock/source and isolated authority receipts are preserved alongside these central integration records.
