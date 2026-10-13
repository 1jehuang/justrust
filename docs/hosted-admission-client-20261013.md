# Native hosted admission tracing, 2026-10-13

## Shipped behavior

Client commit `e6a1b5e` is pushed and installed locally. Under the existing
`JUSTRUST_REMOTE_TRACE=1` setting only the existing `POST build/runs`
requests `X-Justrust-Timing: 1`. It prints five whitelisted, bounded numeric
metrics from the already deployed fleet observer `e75e03e` / `b5757617`:
identity, charge, identity_wait, queue and total. These are awaited paths,
including transit, not pure SQL or CPU. Identity runs concurrently with charge
and is not additive. Normal invocations retain byte-identical curl arguments.
Bearer keys remain stdin-only. TLS verification, redirect policy, timeout,
HTTP body, authoritative quota refusal and charge-before-RunSync are unchanged.
No extra charge, retry, cached admission or account identity shortcut is added.

Malformed, unknown, duplicate, oversized, nonnumeric or control-bearing metrics
are silently ignored. A trace footer separates the status before metrics, so
an injected multiline header cannot replace the status or remove JSON body bytes.
Missing metrics, an older Worker and unsupported curl header output are ignored.
Curl header write-out was added in7.84.0, per its official manual. Current real
curl fixtures test200/402/503, safe/absent/hostile headers, identical JSON/auth,
and exactly one request per fixture. There is no claim of an actual old-curl
binary compatibility test. Pure parser tests cover unavailable output framing.

## Actual hosted acceptance

- Source check, targeted five tests and lint passed before final fixture extension.
- Final full suite:360 passed,0failed,2ignored. All-target clippy with
  `-D warnings` passed. Python benchmark harness:21 tests passed.
- Installed trace check:identity54ms, charge225ms, identity_wait0ms,
  queue67ms, total292ms. Client admission completed413.4ms after roots.
- First installed check spends8.160s synchronizing/reconnecting after replacing
  the executable. Its host compile is zero. Do not hide this cold-client cost.
- Following plain default check, full360test suite and all-target lint execute
  through the hosted service with0compiled/56fresh each. Trace and plain checks
  share existing artifacts. No fallback or toolchain/flag change occurred.
- Actual `~/jcode-desktop`:one extra comment in the FPS module, then default
  hosted `justrust test -p jcode-desktop-ui --lib -- fps_counter`, passes all3
  tests. The exact original SHA-256
  `24de30f7556c2e126bb52f685377f656c6352dfdf83e1bc9f9d03f037d877e8d`
  is restored from a backup, guarded against concurrent edits, never with Git.
  The command took44.5s client/30.7s host with12local crates compiled and734fresh.
  Other user source changes also invalidated provider/base/UI crates, and sync
  took12.858s. This is integration proof, not an isolated timing/speedup claim.
  Existing project warnings are not changed by this service-focused work.

## Harness corrections

The final Rust check/tests/lint all passed before a wrongly rooted Python
unittest import failed. Running from `bench/` passed21 tests. An initial global
`git diff --check` then stopped on unrelated peer EOF whitespace before any
staging. The owned source-only check passed and only `src/remote_hosted.rs`
was committed. Peer findings, benchmark factors and remote daemon edits remain
untouched. These harness failures are not hidden Rust failures.

## Exact exported IDs

All11 records below are verified remote `hosted`, exit0. Full diagnostics and
summaries remain available via `justrust log` / `justrust show`.

- `20261013-144839918-3331621`
- `20261013-144842182-3331778`
- `20261013-144846573-3331438`
- `20261013-144951067-3339142`
- `20261013-144952588-3339330`
- `20261013-145000108-3340087`
- `20261013-145150385-3352742`
- `20261013-145159075-3353831`
- `20261013-145159793-3353900`
- `20261013-145202734-3352735`
- `20261013-145211504-3355155`

## Decision

Keep the numeric observer and normal-client opt-in trace. It makes the next
latency investigation actionable, but it is not itself a measured speedup.
The controlled SSH HTTPS forwarding experiment remains rejected. The fleet,
compiler defaults, analyzer-off state, billing and tenant isolation are unchanged.
**The10x stock-local target remains unmet.**
