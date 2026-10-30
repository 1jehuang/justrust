# Warm hosted request attribution, 2026-10-30

The source-cache improvements do not eliminate a hosted request. Three unchanged
strict all-target Clippy runs on the installed scanner had 62 fresh units and
about 0.03 s Cargo startup:

| Run | Charged, cumulative ms | Server admission ms | Sync ms | Client total ms |
| --- | ---: | ---: | ---: | ---: |
| `20261029-214312289-3063216` | 640.5 | 324 | 356 | 1100.5 |
| `20261029-214313393-3063349` | 470.3 | 298 | 314 | 869.0 |
| `20261029-214314264-3063462` | 662.0 | 266 | 331 | 1076.2 |

These are traced acceptance observations, not a matched suite speedup. One run
was below one second, but sustained subsecond operation is not demonstrated.
Server admission includes charge and queue paths, with identity overlapping.
The difference between client admission and server timing is not automatically
TLS time. It includes transport, process setup and any unmeasured server work.

The next diagnostic adds six bounded numeric curl write-out values under the
existing `JUSTRUST_REMOTE_TRACE` switch: DNS completion, TCP completion, TLS
completion, transfer readiness, first byte and total. They are cumulative
seconds converted to milliseconds, not additive independent phase durations.
An HTTP loopback fixture can legitimately report zero TLS time.

The default command, request body, key-on-stdin handling, HTTP status, refusal,
redirect policy, limits and actual admission POST are unchanged. Invalid or
unsupported numeric metrics are omitted without changing body/status parsing.
Only numeric fixed-name metrics are printed, never arbitrary response headers.
This is attribution, not authorization caching or a performance optimization.

Primary variable semantics: `https://curl.se/docs/manpage.html`, WRITE-OUT.
