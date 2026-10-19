# Central schema-index acceptance

Snapshot `0eb022524a0506701332f35462c4cfc49497c0fd` merges only the reviewed source lookup `a890aa0a3` and separately measured stronger size ratchet `62beb50f0` onto published `fc2de370d`. No old semantic-cache or mutable-interner migration ancestry was imported.

All checks below explicitly ran on hosted:

- Check: `20261018-222051890-2020024`.
- Three focused normalized-schema authority/index tests: `20261018-222053820-2020113`.
- Graph integrity plus parameter equality and projected-method authority, 42 passed: `20261018-222112001-2021527`.
- Strict all-target lint: `20261018-222121285-2019835`.
- Complete library suite, 1,395 passed: `20261018-222131058-2023593`.

Formatting passed. Selected owned redacted receipts are in `central.jsonl`. This is scoped and complete-library acceptance, not a new complete integration-suite verdict. The preceding complete integration run on `8e210f7aa` passed 3,439 tests before this lookup change.

The current central graph gate confirms the stronger requirement of 1,208 compiling functions and 1,209 failing at the unchanged 32,000,000 work limit. The exact adjacent boundary was independently measured by the worker on `eb755d7f7` plus this same source change; see `README.md` and `eb755.jsonl`. Existing immutable sorted declaration indices replace repeated full scans, with canonical miss fallback and poisoned-budget rejection preserved. No budget, work-growth or fact-debt ceiling was relaxed. No wall-time speedup is claimed.

PlanCtx, SchemaAdapter and other retained mutable-state debt remain separate unfinished work.
