# Peer-cache warmup contention acceptance

Run through the normal hosted Rust path:

```sh
justrust test --lib foreground_yield_transport_acceptance -- --nocapture
```

The diagnostic lives inline in the existing `relay_tests` module of
`src/peercache.rs` because the relay and cache modules are deliberately private.
It is Unix-only test code, adds no production API, and does not touch installed
clients, hosted caches, accounts, or production daemons. All cache files are
temporary fixtures.

The diagnostic uses two actual `Relay` endpoints, their production request /
response handling, `remote_proto` framing over a Unix socket pair, real gzip
bundles spanning multiple transport chunks, and production content-hash
verification. Four entries are offered by the real journal/list path. A
foreground signal arrives at the beginning of the first outgoing data frame.
Each data frame incurs a deliberate 30 ms transport delay.

The before control supplies an unconditional callback, reproducing the old
warmup policy. The after case supplies the foreground predicate to the same
production `pull_new_while` implementation. Assertions require:

- The control completes all four entries after foreground arrival.
- The predicate completes only the in-flight entry, preserving its integrity.
- Its return latency is bounded by that bundle's measured residual send time
  plus 100 ms for receiving, unpacking, hash verification, and scheduling.
- The interrupted journal cursor is unchanged, all three remaining entries
  resume, and no entry is fetched twice.
- All installed artifact bytes equal their original bytes.

The output reports foreground-to-warmup-return latency, current-bundle
residual, frame count, and completed entries for both policies. The expected
latency contrast is a **focused policy diagnostic**, not a benchmark-suite
result and not an end-to-end speedup claim.

## Limitations

- The signal is an atomic foreground predicate, not a complete build command.
  Measured wait is until optional warmup returns, not until a source-sync
  barrier or build finishes.
- The wire uses production framing on a local Unix socket, not SSH, TLS,
  hosted network infrastructure, or the daemon's complete shared writer.
  Deliberate pacing is a controlled latency model, not measured Internet
  bandwidth or a production contention distribution.
- The control changes only the callback. It is not a checkout or execution of
  the parent commit's complete client binary.
- This acceptance exercises pull warmup. Existing peer-cache unit tests cover
  push yielding and cursor retention, but this diagnostic does not measure
  push transport latency or test a 400-entry window.
- Timing has a 100 ms scheduler/verification allowance. Structural assertions
  about bundle count, cursor retention, and exact resumed artifacts are the
  primary correctness evidence. A slow or heavily loaded host can fail the
  timing assertion without proving a production regression.
- Default promotion and general performance claims still require the pinned
  session-weighted `bench/suite.py` before/after gate and relevant workloads.
