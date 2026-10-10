# Remote builds

justrust can run `check`, `clippy`, and `test` on a bigger machine and
stream the compact output back, choosing local or remote per build from
measured walls. Phase 4 of [toolchain.md](toolchain.md). Why remote, the
shared cache, and licensing are in [remote-strategy.md](remote-strategy.md).

```text
justrust remote up                  create the AWS machine, or start it if stopped
justrust remote use ssh user@host   build on a machine you already have
                    [--port N] [--identity KEY]
justrust remote use aws|off         switch back to the AWS machine, or turn remote off
justrust remote status              backend, machine health, sync daemon, routing
                    --json, --waybar for a Waybar module, --watch N to refresh
justrust remote ssh [cmd]           shell on the current backend, or run one command
justrust remote down                stop the AWS machine (disk and caches are kept)
justrust remote destroy --yes       terminate the AWS machine and delete its disk
justrust remote check|test|build|clippy <cargo args>
                                    run there explicitly (starts the machine if needed)
```

With no backend configured nothing remote ever happens. Every build runs
locally and routing costs one failed `stat`.

## Measured (2026-10-09, c7i.8xlarge spot in us-west-2, 29 ms round trip)

| case | local (16 threads) | remote |
|---|---:|---:|
| no-op `check -p jcode-base`, warm | 0.3 s | 0.6 s (sync 32 ms) |
| same, before the sync daemon (rsync per call) | 0.3 s | 1.1 s (sync 0.4 s) |
| one-line edit in jcode-base, `check -p jcode-base` | 3.6 to 4.1 s | 3.5 to 3.6 s |
| `test -p jcode-desktop-model`, warm | 4.4 s | 1.6 s |
| cold `check` of Jcode Desktop, 883 units | ~180 s | 44 s |
| one-line edit in justrust, `check` | 2.5 s | 2.5 s |

The sync daemon took sync off the critical path: a warm no-op went from
1.1 s (0.4 s of rsync) to 0.6 s (32 ms of sync, the rest is ssh and remote
cargo startup). Remote wins clearly on cold builds, big recompiles, and
test suites, ties on small edits, and still loses on no-ops. That split is
exactly what routing measures per command.

## Backends

`~/.justrust/remote/backend.json` holds the choice
(`src/remote_backend.rs`). Everything above the backend (daemon, protocol,
routing) only needs an ssh-reachable machine with a shell.

| backend | what | set up with | status |
|---|---|---|---|
| `aws` | one machine in your own AWS account, created and controlled through the `aws` CLI | `justrust remote up` | built |
| `ssh` | any machine you already have. Needs a shell, a C toolchain, and rustup. justrust installs itself there | `justrust remote use ssh user@host` | built |
| `hosted` | justrust's own build fleet with a shared cache, tied to a Jcode subscription | `justrust remote use hosted` | planned, not built. `use hosted` reports that and changes nothing |
| none | every build is local | `justrust remote use off` | |

A machine created before `backend.json` existed counts as `aws`.

`up`, `down`, and `destroy` manage the AWS machine only. With an `ssh`
backend they say so and change nothing (`up` refuses, `down` and
`destroy` only act on an AWS machine that still exists). justrust never
powers an ssh host on or off. `remote ssh` and `remote status` work for
both.

### The AWS machine

- **Region us-west-2.** Measured round trip from the dev laptop: 29 to 42 ms.
  us-west-1 is closer (21 ms) but the account's vCPU quota there is 5.
  Pass `--region`.
- **c7i.8xlarge spot** (32 vCPUs, 64 GB): about $0.50 to $0.55/h spot,
  $1.43/h on demand (`--on-demand`). A persistent spot request with `stop`
  on interruption, so the disk survives.
- **200 GB gp3 at 6000 IOPS and 500 MB/s**, kept across stops, so warm
  target dirs and caches survive.
- **Idle stop after 30 minutes** (`--idle-minutes`). A systemd timer
  records activity (load above 1.0 or an interactive login) and powers off
  after the idle window. Stop, not terminate: the next `remote up` boots in
  about 30 s with everything still on disk.
- **ssh only from this machine's public IP.** The security group
  `justrust-remote` is updated with the current IP on every `up`, and again
  when a status probe times out (new network). The key is
  `~/.justrust/remote/id_ed25519` and is used only for this machine.
- **First boot.** cloud-init installs build-essential, clang, lld, mold,
  cmake, protobuf, poppler, xvfb, and the -sys libraries the Jcode projects
  link against (wayland, xkbcommon, alsa, fontconfig, freetype, xcb,
  vulkan), then rustup stable, and writes `/var/lib/justrust/ready`.
  `status` shows `setup` until then.

## The sync daemon

`justrust __daemon` (`src/remote_daemon.rs`), one per user, started by the
first remote build and kept while builds keep coming. It exits after an
hour without clients or when the backend changes.

1. **Source roots.** `cargo metadata` lists the workspace and every path
   dependency. Each is widened to its git root and nested roots are
   dropped, so Jcode Desktop mirrors `~/jcode-desktop` and `~/jcode`.
   Cached per lockfile mtime.
2. **Same absolute paths.** Roots land at the same paths on the machine, so
   relative path dependencies resolve and diagnostics point at local files.
   A machine where those paths cannot be created gets a prefix instead.
3. **First mirror by rsync**, then only changes travel. A scan
   (`git ls-files` plus one `stat` per file, about 8 ms for the 2,100 files
   of Jcode) is compared with the manifest of what the machine has, and
   each changed file goes out as one `Write` frame. Above 300 changed files
   it uses rsync again. Skipped: `.git`, gitignored files, `target/`, and
   files over 8 MB.
4. **Push on save.** inotify on the roots triggers that scan as files are
   saved, so by the time a build asks there is usually nothing left to
   send. The scan, not inotify, decides what is pushed, so a missed or
   overflowed watch can delay a push but never lose one.
5. **One ssh session** to `justrust __agent` on the machine
   (`src/remote_agent.rs`) carries every push and every run. Several runs
   can be in flight at once (parallel agents).
6. **Remote justrust** is rebuilt on the machine from this checkout's
   source whenever the local binary changes, so remote output, slots, and
   depcache match.

`justrust remote status` shows the daemon from its socket without touching
the network: connected or idle, roots mirrored, files pushed. The log is
`~/.justrust/remote/daemon.log`.

## Protocol

`src/remote_proto.rs`. Every frame is
`[u32 header len][header JSON][u32 payload len][payload]`, big endian.
The same framing runs over the daemon's unix socket (client and daemon) and
over the ssh channel (daemon and agent), so a hosted backend only has to
provide a byte stream.

- `Hello {version}` first in both directions. The agent refuses other
  versions, and a client restarts a daemon built from a different binary.
- `Write {path, mode}` (payload is the content, written to a temp file and
  renamed), `Delete`, `Mkdir`, applied in order.
- `Barrier {id}` and `BarrierAck {id}`: an ack means every earlier frame is
  applied, so the tree matches.
- `Run {id, cwd, args, env, roots, start, run_id}`. The daemon rescans the
  roots, pushes what is left, waits for the barrier, answers
  `Flushed {files, ms, machine, prefix}`, then relays `Out {stream}` frames
  and `Exit {code}` (payload: the remote run's summary.json, so the run is
  recorded locally too, marked remote, under the same run id).
- `Cancel {id}` sends SIGINT to the run's process group.
- `Error {id, msg}` is an infrastructure failure, never a build failure.
- `Ping` and `Pong {roots, machine, pushed, connected, build}` for status.
- `Shutdown` stops the daemon.

## Routing

`src/route.rs`. Every agent-mode `check`, `clippy`, and `test` asks the
router first. `build` and `run` always stay local because their artifacts
are needed here.

`JUSTRUST_REMOTE` controls it:

- `auto` (default): remote only when predicted faster. Never starts a
  machine, never starts anything when no backend is configured, and never
  touches the network to decide.
- `0` or `off`: always local, nothing is read.
- `1` or `force`: remote for check, clippy, and test, starting the AWS
  machine if needed.

The prediction uses the run index: for this cwd and these exact args, the
median of the last 5 local walls against the median of the last 5 remote
client walls (what the caller waited, sync and ssh included). Remote must
win by 20% and by at least 0.5 s. A command with no recent remote sample
and a local median over 3 s tries remote once to get one. Remote samples
expire after 2 days, so a command that lost is re-explored now and then.
For the AWS backend, auto mode also requires evidence the machine is alive
(a recent probe, run, or IP within the idle window), since it stops itself
without telling us.

`JUSTRUST_REMOTE_EXPLAIN=1` prints why a build stayed local.

## Failure and fallback

Local is always the safe answer.

- Any infrastructure problem (cannot reach the machine, daemon or agent
  error, lost connection) becomes an `Error` frame. The client prints one
  line, builds locally, and auto routing stays local for 10 minutes
  (`~/.justrust/remote/route-fail`).
- If remote output was already shown when the failure hit, the client
  exits with an error instead of rebuilding, so output is never duplicated.
- The daemon never assumes the machine is in sync: after a reconnect every
  root is rescanned against a fresh rsync.
- A stopped AWS machine is never started by auto routing. A stale cached IP
  is not trusted past the idle window.
- `remote use off` stops the daemon and removes `backend.json`.

## Local state

```text
~/.justrust/remote/
  backend.json    aws | ssh {host, port, identity} | hosted
  state.json      AWS: instance id, region, type, spot, idle minutes
  id_ed25519      AWS: ssh key for the machine
  known_hosts     AWS: its host key
  ip              AWS: last known IP of the running machine (cleared on down)
  probe.json      AWS: last ssh probe, reused by status for 30 s
  probe-ssh.json  ssh backend: last probe, reused by status for 30 s
  price.json      AWS: hourly price, cached for an hour
  daemon.sock     sync daemon socket (daemon.log, daemon.lock beside it)
  roots.json      source roots per lockfile
  route-fail      last infrastructure failure (10 minute local backoff)
  events.log      remote runs and machine events
```

## Waybar

`justrust remote status --waybar` prints one line of custom-module JSON:
round trip, temperature when the machine exposes one, cpu, ram, disk,
uptime, probe age, and the sync daemon badge (`sync 3r 17↑` means
connected, 3 roots mirrored, 17 files pushed, `sync idle` means the daemon
holds no session). An ssh backend is prefixed with its short host name.
Nothing is shown when remote builds are not set up. The probe is cached for
30 s and the daemon part reads only the local socket, so a 15 s interval
is cheap. Classes: `unconfigured`, `pending`, `setup`, `running`, `busy`,
`stopped`, `error`.
