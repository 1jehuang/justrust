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

## Installed diagnostic acceptance

Hosted check `20261029-214539335-3079312`, 27 HTTP tests
`20261029-214542908-3079756`, strict all-target Clippy
`20261029-214549926-3079057`, and the full 838-test suite
`20261029-214602319-3081573` passed. The release binary was installed.
Real trace `20261029-214718117-3088664` reconnected after installation and
paid 13.6 s sync, so it is not a warm comparison.

The following installed warm runs both kept 62 units fresh:

| Run | DNS ms | TCP complete ms | TLS complete ms | First byte ms | Server ms | Client total ms |
| --- | ---: | ---: | ---: | ---: | ---: | ---: |
| `20261029-214732269-3092837` | 12.6 | 47.5 | 97.0 | 446.9 | 287 | 894.9 |
| `20261029-214733166-3092974` | 12.3 | 41.4 | 83.9 | 445.5 | 296 | 953.3 |

The first warm run sent one file; the next sent none. Both reported 62 fresh
units. Reusing a transport connection could target the roughly 84–97 ms before
transfer readiness, not the entire 445 ms admission request. No such persistent
HTTP pool is implemented here. It would need endpoint/credential isolation,
rotation, bounded lifetime, refusal/retry correctness, and a daemon-owned native
transport. Never reuse an authorization decision as a substitute for a fresh
admission charge. Much larger gains require authoritative whole-command reuse
or overlapping safe preparation with admission, not more Cargo no-op tuning.

Primary variable semantics: `https://curl.se/docs/manpage.html`, WRITE-OUT.
