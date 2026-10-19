# Integrated acceptance receipts

Snapshot: `952c22c20` on `clean-room-rewrite`.

The full source and target tree were held unchanged throughout validation:

- `justrust test --local --no-fail-fast`: run `20261018-200919120-2033172`, exit 0, **3,361 tests passed, 0 failed, 185 binaries**. The complete run had no isolated-retry warning. Local execution is required by stock differential tests that invoke the locally installed oracle compiler.
- `justrust clippy --all-targets -- -D warnings`: run `20261018-201705326-2351540`, exit 0, no warnings, hosted execution.
- `cargo fmt --check`: passed.

The JSONL contains exactly these two unmodified, path-redacted records from `justrust export`, not unrelated machine history. Export redaction labels the repository `private` and anonymizes crate and binary names.

This snapshot includes checked immediate noncapturing closures, canonical runtime element loans for referenced fixed arrays, paid indexed call-effect lookup, and the published-fact reachable-state ratchet. It does not include subsequent shared-slice, Allocation Drop, or immutable budget receipt increments, which require their own composed validation.

The real full-program corpus remains **3 of 20** stock-output-identical programs. This is a correctness receipt for a supported subset, not full Rust compatibility or a performance claim. An earlier snapshot's transient stock linker allocation failure and isolated reruns are separate evidence, not failures in this run.
