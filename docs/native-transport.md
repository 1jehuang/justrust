# Native hosted transport (design, 2026-11-13)

Status: design. Nothing here is implemented. ssh stays the transport until
each stage below is measured against it.

Goal: carry hosted builds over justrust's own protocol on TCP 443 instead of
ssh, so that hosted works on networks that only pass web ports without a
relay hop, build machines need no public ssh ports, and a dropped connection
resumes instead of starting over.

## What ssh carries today

| Use | Where | Shape |
|---|---|---|
| Daemon to agent link | `remote_daemon::ensure_link` | One long-lived `ssh ... justrust __agent`, frames from `remote_proto` on stdin/stdout |
| Durable session | `remote_session` | A broker on the machine owns the agent. ssh carries sequenced input and journal offsets |
| First mirror of a root, and re-verification after a reconnect | `remote_sync` | `rsync --checksum` with `-e ssh` |
| Peer dependency cache | `peercache` | `Cache*` frames on the daemon link |
| Setup and one-off commands | `remote_build` (23 call sites), `remote`, `remote_resume`, `remote_artifacts` | `ssh host 'script'`, some with rsync |
| Installing the client on the machine | `remote_build::ensure_remote_justrust` | rsync or scp of the binary |
| Windows workers | `remote_windows` | ssh exec of a fixed PowerShell invocation |

The framing is already ours. `remote_proto` says the same frames run over
the ssh channel and the daemon's unix socket "so a future hosted backend only
has to provide a byte stream". The agent serves stdin and stdout. The part of
ssh that is hard to replace is not the build stream, it is rsync and the
scattered one-off commands.

## Measurements (one laptop, wifi, 2026-11-07 to 2026-11-13)

These describe this machine during a week of heavy development on an
unreliable network. They are not a fleet-wide sample.

Warm path:

- Three warm no-change `justrust check` runs on this repo spawned zero ssh
  processes from the client. Everything went through the daemon's existing
  link. Wall 2.3 to 2.7 s for the second and third run.
- One ssh command on the warm master: median 166 ms (min 74, max 324, n=10).
  TCP connect to the same port: median 32 ms. So a one-off command costs
  roughly five round trips, but the warm build path does not issue any.

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
  out of three in the observed case: the per-connect key is accepted for 60 s,
  so a reattach after the master died has no valid key. The run was lost
  ("build was not resubmitted").

Conclusion: ssh is not the bottleneck of a warm build, and replacing it will
not speed one up. The cost is at connection boundaries: establishing the
link, re-proving the mirror, and failed reattachment. That is where a native
transport can win, together with the blocked-port and exposure problems. Any
speed claim must be about reconnect and cold-connect wait, not warm builds.

## Design

### Stream

TLS 1.3 over TCP 443, then the existing `remote_proto` frames. No QUIC in the
first version: the networks that block ssh ports commonly block UDP as well,
so QUIC would need a TCP fallback anyway. Multiple logical channels (build
link, file transfer, one-off commands) are multiplexed as frame streams on one
connection, replacing the ssh ControlMaster.

### Reaching a slot on 443

A pool host serves up to 32 slots and has one port 443. The host runs a
router that reads only the TLS ClientHello, picks the slot from the server
name (`slot-N`), and forwards the untouched byte stream to that guest. The
host holds no TLS key and sees only ciphertext, which is the same trust
position as today's DNAT rule. TLS terminates inside the guest.

The router is multitenant infrastructure and lives in `~/justrust-cloud`.
It is new attack surface on the host (a parser facing the internet), so it
must be small, bounded (ClientHello size and time limits, per-source
connection caps) and unable to reach anything but slot guest addresses.

### Identity, same roots as today

- Server: each guest has a long-lived key, as it has an ssh host key now. The
  API returns its public key with the connect response and the client pins
  exactly that key. No certificate authority is trusted.
- Client: each connect generates a fresh ed25519 key and registers it through
  the authenticated API, as now. The client proves possession by signing the
  TLS exporter value, so the proof is bound to this connection and cannot be
  replayed on another.
- Tenant isolation is unchanged: the guest asks the API which keys are valid
  for its slot, with its slot token. The destination is still chosen by the
  server from the account's record, never by the request.

### Resume

After authentication the guest issues a resume secret bound to the session
and the account. A reconnect presents it with the last acknowledged journal
offset and continues the same broker session. This replaces the ssh
reattach that fails once the 60 s key window has passed. The secret is
valid only while the broker lives and only for that session, and a resume
never starts or resubmits a build (the existing rule in `remote_session`).

### Mirror verification without rsync

The agent keeps a manifest (path, size, mtime, content hash) of each mirror.
On connect the daemon sends its manifest digest. A match means no transfer
and no per-file checksum pass. A mismatch exchanges the manifest and sends
only differing files as `Write` frames. This removes rsync from the reconnect
path, which is the 51 minutes above. It does not depend on the new stream and
can ship first over ssh.

The rule "the daemon never guesses that the machine is in sync" stays: the
manifest is the machine's own record of applied writes, verified by digest,
and a missing or unreadable manifest falls back to a full comparison.

### Fallback and versioning

- The transport is negotiated: the connect response says whether the machine
  offers the native endpoint. Old clients and old machines keep using ssh.
- Order of preference once promoted: native on 443, direct ssh, ssh through
  the 443 relay.
- The stream has its own version number next to `remote_proto::VERSION`.

## Stages

Each stage ships behind an opt-in setting and is promoted only on
before/after numbers.

1. **Manifest-based mirror verification over the existing ssh link.** No
   server changes beyond the agent. Measure reconnect sync time on this repo
   (today 10.8 to 22.3 s) before and after.
2. **Native stream for the daemon link**, in the guest and the host router,
   with pinned server key, signed client proof and resume. ssh remains for
   setup commands and first mirrors. Measure link time (today median 2.46 s)
   and reattach success on an interrupted connection.
3. **Move one-off commands and first mirrors onto the stream** (an exec
   channel and bulk file frames), then client installation.
4. **Close the public ssh ports** for accounts on a client that no longer
   needs them. Windows workers are a separate decision.

## Open questions

- Whether the host router is acceptable under the cloud security invariants,
  or whether slots should get per-slot public addresses instead. This needs
  a decision before stage 2.
- Bulk transfer throughput against rsync with compression on a first mirror.
  Not measured.
- Dedicated (non-pool) hosts and Windows workers: same design or ssh only.
