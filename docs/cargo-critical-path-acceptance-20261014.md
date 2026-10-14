# Cargo critical-chain scheduler: actual acceptance, not yet a speedup

October 14, 2026. The default-off fork candidate is
`a45c3546671925c24649e451a673aa71b1385d0d`, based on
`11f22289de172db461915b27d6626c94a501a1f9`. It changes only ready-unit
priority. It does not change dependency edges, Metadata/All release conditions,
jobserver tokens, rustc flags, artifact fingerprints, or test-runner concurrency.

The rank is `cost(unit) + max(rank(immediate consumer))`, with saturation and
cycle detection. The old priority estimated total downstream fanout work rather
than the longest chain. Uniform cost 100 is only a depth heuristic. Optional
bounded package-name hints are not measured per-unit costs, and metadata edges
can release consumers before a producer finishes. Neither score is a proven
compiler causal span.

## Observable acceptance

All compile/check/test/release records below used the ordinary billed hosted
route, `nightly-2026-10-08`, `--locked --features all-static`, and explicit Cargo
configuration for `CFG_RELEASE=1.101.0-nightly` and `CFG_RELEASE_CHANNEL=nightly`.
These transient Cargo config values avoided relying on unforwarded environment
variables. No installed default or submodule pointer was changed.

| Requirement | Actual observation | Run |
|---|---|---|
| Queue rank, multi-output readiness, cycles and overflow | 10 targeted tests passed | `20261013-200738296-853432` |
| Bounded hint parser and filesystem rejection | 5 targeted tests passed after the GlobalContext env-access correction | `20261013-200803294-855336` |
| Real public Cargo interface | 5 tests passed, none ignored, 4,422 unrelated tests filtered | `20261013-201440875-906384` |
| No new strict-lint diagnostics | Exact baseline/candidate parity at all 10 legacy error locations | `20261013-195856311-805183`, `20261013-195952771-813946` |
| Exact committed, portable candidate release | Successful release, identical local/hosted binary and all five source hashes | `20261013-201658478-921138` |
| Real speed improvement | Pending same-binary pinned cold/warm OFF/ON measurements | Not established by acceptance tests |

The public interface tests exercised:

- OFF -> ON -> OFF build/check reuse, every reported artifact Fresh, and
  byte-identical binary, rlib, rmeta, proc-macro library and generated source.
- Independent cold ON builds/checks at `-j1` and `-j2`, including a transitive
  library chain, proc macro, build script and final link. Each binary printed 42.
- Cold actual branch ordering for stock fanout priority, valid hints, and
  missing/invalid hint fallback. Observed order is not a performance result.
- `--keep-going` failure, independent branch completion, blocked dependent,
  source correction/retry, then a Fresh OFF rerun.
- Prewarmed build-script TCP barriers under both internal `-j2` and an inherited
  GNU Make 4.3 jobserver. Two held jobs prevented a third from starting until a
  token was released. The inherited test actually ran rather than being skipped.

These are 20 targeted tests, not the full Cargo test suite. Strict warning-free
Cargo fork lint remains blocked by ten pre-existing diagnostics in cutoff,
fingerprint and daemon code. No allowances or unrelated fixes were used.

## Retained failed attempts and source-sync boundary

The path-redacted snapshot
`bench/results/cargo-critical-path-validation-20261014.json` retains all 12 cited
records, including the unsupported inline-table CLI config attempt, the two new
candidate env-access lint errors before correction, and initial testsuite API
errors. The two successful strict-lint parity runs remain failures, not relabeled
passes.

The initial hosted testsuite also exposed omitted legitimate source modules
under directories named `target`. A bounded authenticated manual copy of eight
Git-listed entries, six regular files and two symlinks, unblocked the candidate
acceptance. Type, size and SHA were matched locally/remotely. That manual copy
is not evidence for a new client's source-sync behavior.

The separate client fix is `92bc376`. It passed hosted 382 tests and strict
all-target lint, and a peer installed the exact committed joint snapshot as
`f3dcac67099a29418eb3c7c230fbe90a765804606111750bf9666bcf86242c83`.
A new-root end-user check/test/clippy/recheck delivered Git-listed
`src/target/value.rs`, while the ignored private sentinel was absent from the
actual remote mirror. Source-sync and native-artifact cache safety define the
new measurement epoch. Do not mix it with older client measurements.

## Candidate executable identity

- SHA-256: `c38c3d63e82c74fc4ba9e2978ad74ccad7f74aa21dec6d2bf978e0c4dcf1e13d`.
- Size: 48,161,072 bytes. ELF64 x86-64 PIE.
- Advertised upstream commit: `f3865b2a4d1acc5276f6b3c67d0e057f4dab3928`.
  Actual built fork source: `a45c3546671925c24649e451a673aa71b1385d0d`.
- Local and hosted executable/source hashes match. The `-vV` runtime OS line
  differs between Arch and Ubuntu, so full-text version equality is not the
  binary identity check.
- The all-static feature vendors Git/SSL/curl dependencies, but the ELF still
  needs libz, libgcc_s, libm, libc and the loader. Both machines resolved them,
  with no missing libraries.

The candidate remains uninstalled and nondefault. Performance evaluation uses a
private copied compiler prefix and this exact Cargo binary, not a replacement
of the service's toolchain. The committed path-redacted records live in
`data/runs/hosted-west-cargo-scheduler/2026-10.jsonl`.

## Performance gate

`bench/scheduler-sweep.py` freezes justrust/rustc/Cargo/marker identities and
runs OFF/ON/ON/OFF on pinned snapshots. A cold multi-crate graph tests scheduling
leverage, while warmed reference/body/noop workflows test regressions where one
dirty unit gives ordering little leverage. All samples and contamination flags
must be retained. Quote `bench/suite.py compare` before any speed claim. No
scheduler speedup, hardware ceiling or default promotion is established here.
