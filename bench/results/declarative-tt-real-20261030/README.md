# dp41: real declarative TT-cache mechanism evidence

Scope and interpretation: `docs/declarative-tt-real-20261030.md`.

* `state.json` and `runner.py` are exact original receipts/provenance.
* Three `*.raw-output.jsonl` journals and stdout/stderr files retain real test execution.
* Three `*.mm_profdata.gz` files are losslessly compressed selected main-crate profiles.
* The two original interval scripts and their JSON outputs retain offline analysis.
* `sha256.json` seals those 18 original artifacts. It does not assert trust in a
  third party. Profile hashes and exact counts are also checked independently
  against the original state and the validator's explicit expectations.
* `validate.py` is portable, offline and does not execute the retained runner.
  It reparses all profiles and recomputes both descriptive interval reports.
* `test_validate.py` contains nine fast contract/mutation tests.
* `audit.json` is the successful portable replay output.

Run from any working directory using the scripts' full paths. Only this checkout's
Python helpers and standard library are needed. No compiler, original home paths,
network access, hosted service, or native workloads are required.

The manifest can be regenerated explicitly with `validate.py --seal` when
intentionally importing original evidence. Normal validation never changes it.

The uncompressed profile SHA256 values are in state.json, while sha256.json covers
compressed bytes. Original runner and interval scripts contain private absolute
paths as historical provenance, not instructions to run those workloads.

Exact markers: prepare eligible/lookup/success=633, provider=633, rejected=1329;
edit eligible/lookup/success=634, provider=1, rejected=1330; restore
eligible/lookup/success=633, provider=0, rejected=1329. Nested persistence/encoding
intervals are descriptive and non-additive. Neither counts nor intervals measure
time saved. The cache remains default-off.
