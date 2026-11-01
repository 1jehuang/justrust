# Portable actual three-stack evidence, November 1, 2026

**Primary evidence is `primary-v91/`, the corrected six-round experiment.**
The top-level run is preserved historical evidence of the earlier **confounded**
experiment, not a second primary benchmark and never a fair timing headline.
Do not pool the runs. Original private journals and the first derivative
commitment were not rewritten to conceal the discovery.

## Validate without Rust, network, writable files, HOME or PATH

From this directory, using any absolute Python 3 interpreter path:

    python3 -B validate.py
    python3 -B validate.py --run initial
    python3 -B validate.py --archive /absolute/path/to/relocated/bundle
    python3 -B -m unittest discover -s . -p test_archive.py -v

Default validation selects `primary-v91/`. `--run initial` selects the preserved
first run. Both invoke their exact measured frozen Python closure, not live
checkout helpers. Tuple-valued semantic oracles are unchanged. A read-only
adapter decompresses the journal in memory, leaving artifacts on disk unmodified.
The CLI audit hook denies native process launches, networking and file writes.
Tests exercise relocation with empty HOME/PATH and explicitly prove each denial.

## Corrected primary run

- Actual task `625000jd4x` completed at 12:46:02 UTC, with 126 semantic passes:
  6 samples x 3 stacks x 7 phases. These are actual command outputs, not mocked
  timings. The worker's archive tests launch Python only, never Rust/native jobs.
- Schema `three-stack-v91-1`, frozen in `primary-v91/frozen/`. Shipping justrust
  explicitly disables both `JUSTRUST_BUILDD` and `JUSTRUST_BUILDD_PREBUILD`.
  Expensive identity/seed verification occurs before source mutation, not as an
  unaccounted speculative head start after an edit.
- `recording-inventory.json` has exactly 7 private shipping recordings per round,
  42 total, linked by original metadata hashes to all 42 measured receipts.
  There are no extra `-pre` recordings or buildd directories.
- Controlled-local, source-preseeded targets/daemon/sysroot cold and subsequent
  warm noop, reachable test-body edit, E0308/status101, fresh-nonce recovery,
  exactly-one-named-assertion failure/status101, and fresh-nonce recovery.
  Four independently known named tests and exact libtest counters are required,
  with 54 distinct execution nonces across the run.
- `summary.json` retains all six samples, median/min/max and descriptive ratios
  separately for every phase and stack. No outlier or slow sample was removed.
  There is no pooled cold/warm score, significance claim, hosted-build claim or
  broad/default promotion. Noise is unknown. Quiet-machine and cold OS page
  cache conditions were not established. These scoped results do not establish
  a general shipping-justrust speedup over stock Cargo.

## Preserved initial confounded run

The top-level journal/schema `three-stack-v89-1` uses its own exact v90 frozen
closure. Actual task `563054yab1` passed all 126 semantic outcomes, but its timing
is command latency with unaccounted speculative background work, **not a fair
edit-to-result benchmark**. The default justrust build watcher was enabled.
Identity/seed hashing after edits let it prebuild outside the process timer,
and earlier-round watchers could observe later source edits.

`INITIAL-RECORDINGS.json` independently inventories 88 private recordings:
42 measured plus 46 background prebuilds, with original metadata/output hashes
and sizes. Background outputs remain private, not silently treated as timed
samples. The separate `REVIEW.json` records the discovery and historical hold.
The shutdown receipt in `primary-v91/context/initial-confound-and-cleanup.json`
is explicitly **coordinator-attributed**, not a contemporaneous worker receipt:
only the six run homes and the pilot home were stopped through their private
control API at 12:34:23 UTC. No private caches were deleted.

## Completeness, privacy and integrity

Each run preserves all 126 full stdout/stderr pairs, all 42 shipping raw-output
JSONL and metadata receipts, 6 Blaze cleanup stdout/stderr pairs and 63 projected
source backups. Each has the complete original 63-file source identity, 4,020-file
toolchain identity, tool ELF hashes and 3,638-file public-registry seed manifest.
Cargo/JR share the pinned LLVM rustc. Blaze's Cranelift backend is an intrinsic
stack difference, not an isolated optimization factor. Proc-macro-server coverage
is explicitly excluded. Offline/locked flags are not falsely asserted supported
by the Blaze test CLI. Source-preseeded cold is not network-download cold.

Only explicitly referenced artifacts were copied. Targets, caches, private HOME
trees, credentials, tokens, binaries and inherited environments were not copied.
`PROJECTION.json` in each run records original SHA256/byte size, projected
SHA256/byte size and transformation notes for every retained input. Metadata
and environment use explicit field allowlists. Private path prefixes become
`/RUN`, `/SOURCE`, `/TOOLCHAIN`, `/TOOLS`, `/PREPARATION` and `/HOME`. Public-source
fixtures containing personal path strings are also projected, while their
original source identities remain separately retained. Do not describe projected
source backups or logs as byte-identical originals.

Both private original journals have fixed original SHA256 values. Each immutable
derivative `COMMITMENT.json` enumerates its retained artifacts. `validate.py`
anchors those commitments and the additive review/history identities in code,
so merely rewriting timings, recomputing statistics and resealing every JSON/hash
chain cannot pass against the fixed version-controlled anchor. Tests demonstrate
that coherent-timing-rewrite rejection. This is integrity and consistency under
fixed history, **not cryptographic host or elapsed-time attestation**. Changing
the validator and its anchors defines a different evidence history.

`PRIMARY-RECORDINGS.json` and `INITIAL-RECORDINGS.json` also preserve original
before/after source manifests, equal tool/TC identity hashes and all 63 restoration
checks, without relying on derivative fixture hashes as original identities.

`HISTORY.json` additively preserves available tool-rendered transcripts of three
preflight failures, original rendered hashes/sizes and path-projection notes.
They are preflight failures, not Rust/native benchmark tests. Older immutable
`PROJECTION.json` missing-log notes describe availability when generated and are
superseded by that additive history, not rewritten. The workflow pilot is
excluded from comparisons. Its timings are not inferred. Frozen v90/v91 mocked
logs are retained. The coordinator's separately run full benchmark Python suite
remains separately attributed unless its raw log is supplied.

No worker changed live harness/Rust/docs files or staged/committed this evidence
without the coordinator's explicit index grant. See `RELEASE.md` for validation
and the final ownership/freeze handoff.
