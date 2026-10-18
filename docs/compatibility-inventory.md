# Static compatibility inventory

## Purpose and limits

`experiments/compatibility_inventory.py` is a Python standard-library-only,
read-only aid to the G0 compatibility ledger. It lists fixture declarations,
known declared legal-but-unsupported boundaries, source locations, contract
availability, historical evidence availability, and explicit unknowns.
**It does not complete G0, certify current native support, or measure Rust
coverage.** Counts are inventory records, not language features or a denominator.
No percentage is emitted. No compiler, fixture runner, or imported corpus module
is executed by the tool.

This implementation was developed in the assigned scratch checkout on
2026-10-18. Only the new inventory script, its unittest file, and this document
are deliverables. There are no compiler source changes and no Rust builds.

## Locally observed reference identity and initial scope

The following was observed by running `/usr/bin/rustc -vV` on 2026-10-18,
not inferred from a package name or fetched from the internet:

```text
rustc 1.98.1 (48a229cea 2026-09-01) (Arch Linux rust 1:1.98.1-1)
binary: rustc
commit-hash: 48a229ceaefd4985c50990b14116b6d856af0985
commit-date: 2026-09-01
host: x86_64-unknown-linux-gnu
release: 1.98.1
LLVM version: 22.1.8
```

The initial proposed ledger comparison scope is **edition 2024, Linux x86_64,
`x86_64-unknown-linux-gnu`, with this reference compiler identity**. That pins
an initial scope, not a claim that the native compiler supports all of that
baseline. Sysroot contents, target features, linker, ABI, Cargo behavior,
proc macros, library completeness, other targets and editions are not pinned
or validated by this inventory. The tool does not rewrite older contracts:
the shared-slice rejection README explicitly specifies edition 2021. Historical
receipts must retain their own scope rather than silently becoming 2024 results.

## Usage

From the checkout root, with Python 3.9 or newer:

```sh
PYTHONDONTWRITEBYTECODE=1 python3 experiments/compatibility_inventory.py
PYTHONDONTWRITEBYTECODE=1 python3 experiments/compatibility_inventory.py --format json
PYTHONDONTWRITEBYTECODE=1 python3 -m unittest discover -s experiments -p 'test_compatibility_inventory.py' -v
```

`--root PATH` selects another checkout. The default is the script's checkout,
not the caller's current working directory. Output goes to stdout only. Tests
create and clean temporary fixture directories inside this checkout. The script
does not install packages, invoke subprocesses, write caches, or change sources.

JSON schema 1 has `records`, classification `counts`, a fixed unknown native
support statement, and `g0_complete: false`. Records include `source_path`,
`locator`, `classification`, `classification_basis`, `contract`, and `evidence`.
Additional schema-specific fields expose source hashes, matching/missing/conflicting
receipts, expected-stdout availability, and full-source versus fragment status.
Text output gives the same classification, location, contract and evidence summary.
Use JSON for full details and unknown explanations.

## Formats inspected before implementation

There is no global fixture schema. These formats are handled separately:

| Format and inspected examples | Handling |
| --- | --- |
| `tests/fixtures/leading_zeros/manifest.tsv` | Headerless four columns: name, semantic category, stock observation, source SHA256. Resolve sibling `.rs` and `.stdout`; compare source bytes and matching entry in `stock-receipt.json`. Unknown categories stay unknown. Other discovered `manifest.tsv` locations are not assumed to share this format. |
| `tests/fixtures/slice-contract-rejection/*.rs` | Independent `// stock: accept\|reject` and `// category: ...` headers. Only accept plus excluded means legal-unsupported. Reject plus excluded remains Rust-invalid, never legal. Duplicate/missing headers remain unknown. README is historical human-readable evidence, not a machine replay receipt. |
| `experiments/*correctness.py`: literal `FIXTURES` dictionary | Grouped arrays of dictionaries, as in struct and pattern corpora. Preserve declarations; `source` can be only a fragment needing a wrapper. |
| `experiments/*correctness.py`: `FIXTURES = json.loads(...)` | Literal JSON string or a prior literal constant, as in borrowing and generic-carrier corpora. Read full source and any declared hash without importing the module. |
| Dynamic `corpus()` functions, local tuple lists, calculated sources | Explicit unknown record per runner. No execution or pretend interpretation of loops, conditionals, promotions, formatting, and wrappers. The tuple corpus, for example, conditionally promotes a historical boundary; static declaration records cannot establish effective current classification. |
| `fixtures/adler2_adapted/stock-evidence.json` | Match named case and source/oracle hash for sibling `cases/*.rs`. Evidence availability is reported, but absent explicit per-case classifications remain unknown. Adapted recurrence evidence does not imply unchanged-crate or native acceptance. |
| Other `.rs` files under `fixtures/` and `tests/fixtures/` | Explicit unknown, nearest supported README location when available. An `unsupported` filename alone never establishes classification. |

The slice runtime/rejection and leading-zero runner scripts are separately listed
as unknown runner records. Their file corpora are inventoried, but runners are
not executed. Other experiment scripts, Rust inline test strings, arbitrary
Markdown assertions, benchmark manifests and externally retained scratch results
are outside this walk. Unsupported schemas need explicit adapters and tests.

## Interpretation and observed inventory

Inventory counts should be regenerated in the destination checkout after copying
these files, because the shared fixture tree can change concurrently. Counts are
overlapping artifacts and runner placeholders, not unique semantic obligations.
All source-hash comparisons that were available matched during local validation.

Two particularly direct boundary groups are the leading-zero `ufcs` and
`reference` fixtures, with matching manifest/source/receipt entries, and six
shared-slice exclusions: mutable slice, slice return, generic slice, non-u8
slice, range subslice, and iteration. The two bare-by-value slice exclusions are
stock-rejected and therefore **not** counted as legal boundaries.

A generic `boundary` group stays unknown: without a format-specific contract it
could denote something other than legal unsupported Rust. Explicit legal-exclusion
groups in the inspected formats are handled separately.

A `legal-unsupported` record means **the inspected declaration labels it that
way**, not that a fresh native run failed. Historical declarations can be stale
following later support work. Embedded dictionaries are declarations, not
independent stock receipts. Matching a receipt record checks availability and
agreement, not authenticity or full command replay; source hashes establish
byte agreement only. A present README establishes a human-readable contract,
not passing execution. Missing output or an unparsed corpus is unknown, never
implicitly supported or invalid. Grouped fragments may still need runner-added
`main` functions, and dynamic runners can override groups, so the tool emits an
additional unknown `corpus()` record even when static fixtures were extracted.

## Validation and follow-up

Unittest exercises TSV/receipt agreement and conflict, missing source/evidence,
malformed rows and JSON, independent header semantics, duplicate headers,
literal dictionaries, embedded JSON and named JSON constants, source-hash drift,
unknown groups (including ambiguous `boundary`), missing root directories, dynamic
corpora, nonexecution, deterministic walks, Adler evidence,
and actual checkout boundary groups. No Rust compilation or native tests were
run. `/usr/bin/rustc -vV` was identity inspection only.

Follow-up for G0 remains: expand adapters for dynamic corpora, reconcile historical
boundary declarations against current native behavior, pin full baseline inputs,
and obtain independent, exact-source stock/native check/build/runtime evidence
for each claimed obligation. This document intentionally does not check off the
entire G0 milestone.
