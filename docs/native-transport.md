# Native hosted transport (implementation status, 2026-11-14 UTC)

Status: candidate implementation and coordinated validation, not production
promotion. The client, guest supervisor, control-plane negotiation and host SNI
router have implementation paths. Their presence in source, or a router canary,
is not evidence that the published client and fleet offer the complete path.
SSH remains the compatibility transport and must not be disabled yet.

Goal: carry hosted builds over justrust's protocol on TCP 443, so networks that
block public SSH ports can connect without an SSH relay, and interrupted builds
can reattach without starting another execution. No native-versus-SSH speedup
has been established by this implementation work.

## Current component boundaries

| Use | Implementation | Current shape |
|---|---|---|
| Command byte stream | `src/remote_native.rs`, `remote_backend::Target` | TLS 1.3 native exec with authenticated stdin/stdout/stderr/exit frames, or the existing SSH path |
| Daemon to agent link | `remote_daemon::ensure_link`, `remote_proto` | Existing agent frames carried by the selected target transport |
| Durable session | `src/remote_session.rs` | A private broker owns the agent, sequenced input and output journal; transport reconnect attaches to that broker |
| First mirror and bulk changes | `src/remote_sync.rs` | Rsync remains the transfer engine; native targets use `__native rsync` as its remote shell |
| Executable uploads | `remote_sync::upload_file`, `remote_build` | Rsync over the selected native or SSH transport |
| Reconnect mirror evidence | `src/remote_checkpoint.rs` | Opt-in `checkpoint-hash`, with token, sizes and source-snapshot SHA-256 hashes |
| Peer dependency cache | `src/peercache.rs` | Existing `Cache*` frames on the daemon link |
| Guest privilege boundary | Native root supervisor | Root-only server key and slot token, authenticated commands executed as `ubuntu` |
| Pool ingress and authorization | `~/justrust-cloud` | Lease-scoped SNI passthrough, guest readiness, authenticated endpoint offers and slot authorization |
| Windows workers | `src/remote_windows.rs` | Separate compatibility path; current native hosted offers reject Windows |

Rsync over native exec is a transitional integration, not a claim that a native
bulk-file protocol has replaced rsync. It preserves rsync traversal, filters,
deltas and deletion semantics while replacing its remote shell. Native rsync
uses secluded arguments so file paths travel in the rsync protocol. Replacing
that engine requires separate correctness and throughput evidence.

## Historical measurements (one laptop, Wi-Fi, 2026-11-07 to 2026-11-13)

These predate the hash-bearing checkpoint implementation and native acceptance
work below. They describe one machine during heavy development on an unreliable
network, not the fleet or a current native-versus-SSH comparison.

Warm path:

- Three warm no-change `justrust check` runs on this repo spawned zero SSH
  processes from the client. Everything went through the daemon's existing
  link. Wall 2.3 to 2.7 s for the second and third run.
- One SSH command on the warm master: median 166 ms (min 74, max 324, n=10).
  TCP connect to the same port: median 32 ms. A one-off command incurred
  additional setup, but those warm builds did not issue one.

Reconnect path, from `~/.justrust/remote/daemon.log`:

| Day | Connects | Closed or lost | Transport interrupted |
|---|---|---|---|
| 11-07 | 4 | 4 | 0 |
| 11-08 | 26 | 4 | 0 |
| 11-09 | 101 | 38 | 0 |
| 11-10 | 75 | 68 | 66 |
| 11-11 | 23 | 4 | 13 |
| 11-12 | 49 | 49 | 62 |
| 11-13 | 34 | 46 | 41 |

- Link time (connect plus agent start) when a sync had to establish it:
  median 2.46 s, p90 25.1 s, n=244.
- `rsync --checksum` re-verification of a mirror: median 1.2 s, p90 25.1 s,
  n=392, 51 minutes in total over the week. For this repo (95,386 files) it
  was 10.8 to 22.3 s per reconnect with zero or one file actually sent.
- Session reattach failed with `Permission denied (publickey)` three times
  out of three in the observed case: the per-connect key was accepted for
  60 s, so reattachment after the master died had no valid key. The run was
  lost ("build was not resubmitted").

These observations motivated work on connection setup, mirror evidence and
reattachment. They do not measure the new transport, the new content hashing,
or a general warm-build benefit.

## Implemented architecture

### TLS command stream

`src/remote_native.rs` implements TLS 1.3 over TCP with ALPN
`justrust-native-v1`. Each authenticated exec connection carries bounded native
frames for stdin, EOF, stdout, stderr, exit status and errors. Existing agent
and durable-session protocols run inside those command byte streams. This is
not yet a multiplexed replacement for SSH ControlMaster with many independent
exec channels on one TLS connection. Interactive `justrust remote ssh` still
uses the explicit SSH compatibility path because native exec does not negotiate
a PTY. A working native build stream alone is not permission to close the SSH
port used by interactive sessions.

The client pins the SHA-256 of the guest's DER certificate supplied by the
authenticated control plane, rather than trusting a public CA. The server still
proves possession of its private key during TLS. TLS 1.2, missing native ALPN
and a wrong certificate pin are rejected. TLS session resumption and tickets
are disabled in this implementation; application-level resume is separate.

A client Ed25519 SSH identity signs a protocol-namespaced transcript containing
the TLS exporter, a fresh server challenge, the command and any requested resume
identity enrollment. This binds authorization to the connection and command.
The guest fetches fresh `{keys, account_id, lease_id}` authorization using its
slot credential, rather than accepting an account or lease chosen by the
client.

### Root supervisor and lease-scoped resume

The guest supervisor reads its TLS private key and slot token from root-owned,
private files. It starts authenticated commands as the fixed `ubuntu` user,
dropping supplementary groups, GID and UID and constructing a clean command
environment. Neither the slot credential nor the TLS private key is lent to the
build user. Pool provisioning, credential isolation and readiness are owned by
`~/justrust-cloud/pool/NATIVE-TLS.md`.

After proving possession of a currently authorized connect key, a client can
enroll a separate Ed25519 resume public key. Its private key is stored locally
in a private, owner-checked file. Enrollment is bound to the control-plane
account and lease and retained in supervisor memory with an expiry. Every
subsequent authentication fetches current lease authorization and discards
expired or mismatched enrollment. A resume identity cannot enroll another
identity. Supervisor restart loses this in-memory enrollment, and lease change
must not carry it forward.

This credential is not a durable job by itself. `remote_session` still owns the
broker, input sequence acknowledgements and output journal offsets. Reattachment
continues the existing broker session. A missing broker or host reboot is a
lost job, never permission to create or resubmit a build. Application resume
must not be confused with TLS session resumption.

### Lease-scoped SNI routing on 443

The host router in `~/justrust-cloud/pool/sni-router.py` reads a bounded TLS
ClientHello and forwards the original TLS bytes to a fixed guest address on
443. TLS terminates in the guest. The router holds no TLS key and does not
select arbitrary caller-supplied upstream addresses.

Production routing uses a root-owned, expiring route snapshot published by the
pool agent. The SNI name encodes the current lease hash, not merely a reusable
`slot-N` number. Missing, expired, malformed or unassigned routes fail closed.
Static slot routing is a development mode, not the production lease contract.
SNI is a routing hint, not account authorization: certificate pinning and guest
account/lease authorization remain required.

The control plane offers `{address, port: 443, server_name,
certificate_sha256}` only when its readiness and lease checks pass. Client
validation requires the offered host to match the authenticated worker address
and keeps local key/resume paths out of the API-deserialized offer. See
`~/justrust-cloud/pool/SNI-ROUTER.md` and `pool/NATIVE-TLS.md` there for routing,
readiness and coordinated canary requirements. An installed listener alone is
not native guest readiness or production promotion.

### Hash-bearing mirror checkpoints

Link checkpoints remain opt-in with `JUSTRUST_REMOTE_CHECKPOINT=1` and work over
the selected byte stream, including SSH. Commit `d373f2c2c` closes the prior
settled same-size external-mutation hole:

- The daemon captures SHA-256 hashes from its source snapshot before initial
  or bulk rsync. For incremental writes it hashes the exact `Write` payload.
  It retains those hashes instead of rereading mutable local files when
  checkpointing or reconnecting.
- Once a root is in sync, `Checkpoint{path, token}` is applied in frame order.
  The agent writes `<root>/.justrust/mirror-token` only if no earlier write or
  delete under that root remains failed. The daemon tracks paths sent later
  as dirty.
- On reconnect, `Verify` carries the token plus size, SHA-256 and path for
  every settled entry. The agent checks regular-file type, size and content,
  and rechecks that the token survived the scan before replacing it. On
  confirmation, the daemon resends dirty paths and local changes.
- Missing snapshot hashes or failed verification cause the full mirror path.
  A same-size external change, even with preserved mtime, no longer passes
  solely because its length matches.
- The new `checkpoint-hash` capability is deliberately distinct from legacy
  `checkpoint`. Older peers do not interpret the new list format, and a
  daemon without negotiated hash support uses the full mirror path.

This closes the specific settled-content gap, not every concurrent-writer
problem. The final token recheck is conservative detection, not atomic
exclusion of another agent or unmanaged writer during verification and token
replacement. Cross-process writer exclusion remains a separate correctness
concern before broader promotion. Daemon-restart mirror resume in
`remote_resume` is also a separate mechanism, not upgraded to content hashing
by this checkpoint change.

The implementation adds source reads before initial/bulk copies, hashing of
incremental payloads and remote content reads on reconnect. There are no
before/after benchmark results for this hash-bearing path. Historical
size-only checkpoint timings in `FINDINGS.md` must not be presented as its
performance.

### Negotiation and compatibility

The authenticated connect response advertises a native endpoint when available.
`JUSTRUST_HOSTED_NATIVE=1` requires an offer, `=0` selects compatibility behavior,
and the candidate's normal selection can use an offered endpoint. This source
behavior is not a statement that native transport is production-promoted.
Old clients and workers without the offer retain SSH. Native authentication
errors must not be hidden by silently replaying a possibly started command on
another transport.

### Explicit per-lease SSH retirement

`JUSTRUST_HOSTED_NATIVE_ONLY=1` is a separate, default-off client opt-in. It
requires both operator gates and a fresh account/lease/certificate-bound offer.
After ordinary native authentication, `__native accept` asks the privileged
supervisor to submit the offer nonce to the control plane with its root-only
slot credential. This is a separately signed operation, not a shell command.
The client checks the returned account, lease, nonce and certificate before
sending its own account-authenticated acknowledgment. The API independently
requires its stored supervisor receipt and fresh host readiness evidence.

Only that accepted lease can request removal of its public SSH DNAT rule.
Host lifecycle locking fences the rule change against slot reassignment.
Native/control-plane failure restores SSH and invalidates promotion evidence.
Legacy clients and interactive terminal requests opt back into compatibility
and wait until SSH is restored. This code is implemented behind gates, not
proof that production ports have been closed or that native acceptance passed.

The native stream has its own protocol namespace and ALPN alongside
`remote_proto::VERSION`. Exec framing, checkpoint capabilities and the durable
session protocol are distinct versioning boundaries. SSH remains available
while those paths are validated together.

## Validation and promotion gates

As of 2026-11-14 02:37 UTC, the hosted filesystem has been recovered without
data deletion and the integrated candidate `bc873aeac` passes hosted all-target
check and clippy. The following real hosted gates passed:

- All-target check: `20261113-183327586-1431030`.
- Sixteen native TLS/security tests: `20261113-183412456-1435514`.
- Thirty-one agent integration tests, including checkpoint corruption:
  `20261113-183457017-1439834`.
- All-target clippy: `20261113-183532221-1442813`.
- 283 remote unit tests, including daemon snapshot regressions:
  `20261113-183557908-1445920`.
- Nineteen native bulk/CLI tests: `20261113-183604429-1446413`.
- Release-profile candidate build: `20261113-183609116-1446699`.

Independent CI also passed Linux fmt/clippy/tests and MSRV on the integrated
commit. Cross-platform CI, production positive native routing, live socket-drop
acceptance, and representative comparison remain separate gates. No release,
default promotion, or public SSH closure follows from these test results.

The explicit unpublished candidate has SHA-256
`c0c10710a1882e09a055ed4928db40b25746894cd411af45e6d6d466167c09fe`.
Its package version remains `0.1.0-alpha.26`, but it is not the published
artifact and has not replaced the default installed production client.

| Stage | Implemented candidate work | Still required before promotion |
|---|---|---|
| 1. Mirror verification on the existing link | Hash-bearing checkpoints in `d373f2c2c`; original size-only gap closed | Concurrent-writer correctness review, compatible fleet agent, before/after measurements of this version |
| 2. Native daemon/session stream and ingress | TLS 1.3 exec, exporter-bound Ed25519 proof, root supervisor, lease-scoped enrollment and SNI routing paths | Coordinated client/guest/router/control-plane acceptance, interrupted-session checks and representative measurements |
| 3. One-off commands, first mirrors and installation | Target-selected native exec and transitional rsync-over-native for bulk copies/uploads | Real cold/incremental build and file-transfer correctness, throughput comparisons and fallback validation; a native bulk engine is not implemented by this adapter |
| 4. Remove public SSH exposure | Default-off supervisor receipt, account/lease-bound acknowledgment and reversible per-slot SSH DNAT control | Published client/fleet native-only acceptance, real rollback and isolation tests, security review and representative measurement; Windows and non-pool hosts need separate decisions |

For general speed claims or default promotion, run the same comprehensive
session-weighted `bench/suite.py run --synth 1 -n 1` workload before and after,
then cite `bench/suite.py compare` and recorded run IDs. Hosted-only transport
measurements use `bench/versus.py run --synth 1 -n 1 --only remote --remote`
and `bench/versus.py compare`, because the suite runner does not support remote
synthetic runs. Keep the candidate bytes, seed, all 40 points, snapshots,
generator, weights, and execution mode matched. Add reconnect, cold
build, interrupted-session and isolation checks relevant to this transport.
Focused historical observations and functional tests do not replace that gate.
