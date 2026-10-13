# Hosted sample timing provenance, 2026-10-13

## Shipped benchmark improvement

`5bad625` preserves existing client trace lines in the optional sample field
`remote_trace.client_elapsed_ms`. New clients also supply
`remote_trace.admission_awaited_ms`. Parsing happens after the timed command
returns. It neither enables tracing automatically nor adds HTTP requests,
changes billing authority, starts compilation, or alters toolchain/cache flags.
Only fixed ASCII names with bounded nonnegative numeric values are retained.
Duplicate or invalid fields are omitted. Raw headers and credentials are never
retained. All 24 harness tests pass, including malformed/unknown/duplicate and
nonmonotonic regressions.

Admission fields describe awaited paths, including service transit. Identity
overlaps charge. Do not add identity to charge, queue and total as independent
work. Client milestones are cumulative timestamps, not individual durations.

## Real normally billed acceptance

Command: `JUSTRUST_REMOTE_TRACE=1 JUSTRUST_BUILDD=0 JUSTRUST_EARLY=0
python3 bench/suite.py run --remote --toolchain nightly-2026-10-08 -n2
--warmup1 desktop-noop-test`. Normal quota, identity, TLS and admission gates
were unchanged. Installed client SHA256 was
`e2bc92690d1e2f0c11f0d7be413fee97e70fb987df0bcfeadfea68e7a4e5f79e`.
The result truthfully records repository HEAD separately and no declared binary
revision. This is instrumentation acceptance, not a speed comparison.

| Run | wall s | client charged ms | client flushed ms | client exit ms | server charge ms | server queue ms | server total ms |
|---|---:|---:|---:|---:|---:|---:|---:|
| `20261013-152106006-3541783284` | .728 | 410.3 | 508.1 | 723.2 | 228 | 59 | 287 |
| `20261013-152106769-3541783225` | .766 | 364.9 | 483.2 | 762.5 | 209 | 54 | 263 |

Each sample used actual hosted execution and curl, compiled zero units with
744 fresh, and passed all three FPS tests. All six client milestones and five
server phases were captured. Warmup `20261013-152103081-3541783220` passed the
same tests and is excluded from the two measured samples.

## Corrected earlier diagnostic verifier

Frozen historical client `6c0988cb186b`, SHA256
`e3376665dd4108ba21cb0b3c140f093d388616fc9c4727957dc4e4da080e8152`,
was tested on the current backend with native/curl/native epochs. Each used
two warmups and three measured pinned no-op tests. All nine measured samples
really ran hosted with zero compiled, 744 fresh, three passed and 1310 filtered.
The harness verified the actual local stderr marker before retaining
`admission_transport=pooled` or `curl`.

| Epoch | median s | range s | measured samples |
|---|---:|---:|---:|
| native before | .810 | .690-.825 | 3 |
| curl | 1.072 | 1.027-1.176 | 3 |
| native recheck | .885 | .693-1.162 | 3 |

These are a small diagnostic with overlapping recheck ranges, not evidence to
restore a library or change defaults. Original first native warmup
`20261013-151815916-3523042466` took15.63s including14.88s sync after reconnect.
It was not hidden or counted as a measured native-pool sample.

The shell task failed only in its subsequent read-only verifier. That verifier
looked for local client trace markers in saved remote-only Cargo output and
used the wrong JSON field. Remote output uses `l`, not `text`, but fixing the
field alone would still not recover local client diagnostics. Verification was
corrected using the actual transport field and remote summaries without running
another billed build. The new capture prevents discarding these numeric client
diagnostics in future experiments.

Its restoration trap did run. Installed default-hosted check
`20261013-151845550-3526469` succeeded with zero compiled and56 fresh, after
8.763s cold sync. The installed source and default admission transport remained
unchanged. All15 diagnostic builds, this restoration, and3 instrumentation
acceptance builds are preserved as19 exact exported records.

10x stock-local is still not achieved. Do not combine these instrumentation
observations with unmatched historical compiler or geography epochs.
