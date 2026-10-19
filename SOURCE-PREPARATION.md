# Configured source preparation

This is a bounded clean-room source-envelope milestone, not full Rust support or
unchanged `core`, `alloc`, or `std` acceptance. `IMPLEMENTATION.md` remains the
full implementation program. The source graph currently publishes one immutable
prepared snapshot per compilation, not a persistent per-file incremental cache.

## Contract

Text and filesystem inputs enter the same configured SOURCE operation before
parsing. The snapshot owns source configuration, retained original files,
prepared text, attribute records and byte provenance. Module discovery observes
configuration before resolving or reading disabled child files. A recording
reader exercises filesystem operations, rather than inferring absence of reads
from successful compilation of a missing file.

The CLI accepts repeated `--cfg name` and `--cfg 'key="value"'` specifications,
including the `--cfg=...` spelling. Native target/profile predicates are fixed
Linux x86-64 facts, with checked execution and an abort profile. Reserved target
facts cannot be replaced by user configuration. These facts do not imply general
platform ABI, panic, unwind or target compatibility.

Configuration predicates and effective attributes are processed in source
order. A false `cfg` suppresses subsequent predicate evaluation and effective
attribute classification for that item. Inactive `cfg_attr` payloads are not
recursively evaluated. Attribute envelope syntax remains validated. Unknown
unregistered active attributes are syntax errors, while recognized valid forms
outside the admitted policy receive explicit unsupported diagnostics.

The admitted inert policy includes ordinary documentation strings/hidden markers,
crate URL-string settings (`html_favicon_url`, `html_logo_url`, `html_root_url`,
`html_playground_url`, `issue_tracker_base_url`), and parsed `test(attr(...))` /
`test(no_crate_inject)` rustdoc configuration. The existing bounded metadata
parser checks nested attribute grammar and full token consumption. Doctest
attributes are not executed as native attributes: nested `cfg` cannot remove a
native item, and documentation cannot grant identity or bypass unused-body checks.
Unknown settings and unsupported attachment positions retain explicit diagnostics,
while original byte positions and attribute records are preserved.
`tests/doc_metadata.rs` compares original time-core metadata with stock and native
CHECK/BUILD/run at workers 1 and 8; focused parser controls reject malformed
settings, nesting and depth overflow. This does not implement rustdoc/doctests.
Supported inline hints do not promise optimization. Other stock-warning
documentation forms remain explicit policy boundaries.
Active path overrides, derives, representation/language attributes and arbitrary
semantic attributes are not silently discarded. Attribute token trees containing
unimplemented byte/C/character or non-decimal numeric literal classes are
explicitly unsupported even when their semantic payload would be inactive,
rather than accepted without lexical validation.

## Inactive syntax boundary

Configuration suppresses semantic name resolution and body checking, not lexical
or syntactic validity. Inactive items are checked by a bounded syntax-only parser
without source preparation recursion, semantic publication or child IO. This is
not the complete Rust grammar: stock-valid disabled syntax beyond the admitted
parser is explicitly unsupported. It must not be described as invalid Rust or
as complete conditional-compilation compatibility. Validated raw strings in
inactive bodies can be replaced by byte-preserving lexical placeholders for
this structural check, without enabling active raw-string expression semantics.

## Locations and authority

Tokens retain original UTF-8 byte ranges, including Unicode/CRLF input and split
punctuation. Prepared ranges map to original lexical file occurrences. Synthetic
transport punctuation is anchored, not represented as original bytes. Mapping
uses bounded checked ranges and logarithmic segment lookup; unmapped or
cross-origin tokens fail closed. Syntax diagnostics can render original file,
one-based line and Unicode scalar column. Resource diagnostics without a genuine
source location do not fabricate a span.

These locations do not grant semantic authority. Existing declaration identities,
source replay recipes, universal validation and sealed facts remain authoritative.
General semantic diagnostic spans, macro expansion provenance/hygiene, external
crates, library roots and full edition resolution remain open work.

## Resource and validation gates

Inline module nesting preserves the existing 128-level contract. External-file
loading and attribute expansion have separate bounded depth budgets. Source
bytes, file count, retained origin segments, predicate work and user configuration
are bounded independently. The milestone requires ordinary-stack regressions,
recorded disabled-child IO checks, executable stock classification before native
workers 1/8 CHECK/BUILD, graph/accounting checks, full hosted regression, strict
lint and formatting before publication.

## Acceptance checkpoint (2026-10-17)

Hosted all-target check `20261017-052126826-212241`, all 1731 tests across
90 binaries `20261017-052128570-212557`, strict all-target lint
`20261017-052144245-213435`, formatting and diff checks pass on the frozen
source. Independent stock-first conformance `20261017-051709382-193986`
passes 86 fixtures and 344 native CHECK/BUILD cells at workers 1 and 8:
39 supported, 33 invalid, 12 legal unsupported and two stock-warning policy
boundaries. The hosted oracle is rustc 1.101.0-nightly
(1d81eb4ad9cd207e3e638bd32b17ec4fce8412a6), edition 2021, Linux x86-64.
Original-child UTF-8 diagnostics, zero disabled-child IO, ordinary-stack
limits and text/file graph accounting have separate regression coverage.

An earlier full run `20261017-051741486-196166` encountered transient
ETXTBSY spawning the native float oracle and passed isolated runner retry.
Its harness now retries only OS26 failures before successful spawn, bounded
to 100 attempts, without replaying an executed child. The final run above
passes without runner-isolated retries. Emitted arithmetic is unchanged.
Receipts are retained in
`data/runs/xps14/2026-10-17-source-preparation.jsonl`. These are correctness
observations, not speed measurements or full Rust/library acceptance.
