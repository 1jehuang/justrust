# Remote compile machine

Phase 4 of [toolchain.md](toolchain.md) starts with one dedicated build
machine per user that justrust creates and controls. The build routing
(sync, remote cargo, artifacts back, local or remote chosen per build) comes
next. This page covers the machine itself. Why remote, the shared cache, and
licensing are in [remote-strategy.md](remote-strategy.md).

```text
justrust remote up        create the machine, or start it if stopped
justrust remote down      stop it (disk, target dirs, and caches are kept)
justrust remote status    state, cost, round trip, cores, load, toolchain
                          --waybar for a Waybar module, --watch N to refresh
justrust remote ssh [cmd] shell on the machine, or run one command
justrust remote destroy --yes   terminate it and delete its disk
justrust remote check|test|build|clippy <cargo args>
                          sync the source and run `justrust <sub>` there
```

## Remote builds

`justrust remote check -p jcode-base` from a local checkout:

1. **Source roots.** `cargo metadata` lists the workspace and every path
   dependency. Each is widened to its git root and nested roots are
   dropped, so Jcode Desktop syncs `~/jcode-desktop` and `~/jcode`. The list
   is cached per lockfile mtime (0 ms after the first call).
2. **Sync.** One rsync per root, in parallel, over a shared ssh connection
   (ControlMaster, kept 10 minutes). Roots land at the *same absolute
   paths* on the machine, so relative path dependencies resolve and
   diagnostics point at local files. Skipped: `.git`, gitignored files,
   `target/`, and files over 8 MB. The machine's `target/` dirs are
   protected from `--delete`, so they stay warm.
3. **Remote justrust.** The machine builds the same justrust from this
   checkout's source whenever the local binary changes (about 6 s,
   persistent target dir), so remote output, slots, and depcache match.
4. **Run.** `justrust <sub> <args>` runs in the same cwd and its compact
   output streams back. The last line splits the time into sync and remote.

### Measured (2026-10-09, c7i.8xlarge spot, 29 ms round trip)

| case | local (16 threads) | remote |
|---|---:|---:|
| no-op `check -p jcode-base` | 0.3 s | 1.1 s (sync 0.4 s) |
| one-line edit in jcode-base, `check -p jcode-base` | 3.6 to 4.1 s | 3.5 to 3.6 s (sync 0.4 s, build 3.1 s) |
| `test -p jcode-desktop-model`, warm | 4.4 s | 1.6 s |
| cold `check` of Jcode Desktop, 883 units | ~180 s (README) | 44 s |
| one-line edit in justrust, `check` | 2.5 s | 2.5 s |

Fixed overhead per remote call is about 0.8 s: 0.4 s sync (two rsync
round trips plus the file scan) and about 0.4 s for ssh and remote cargo
startup. Remote wins on cold builds and big recompiles, ties on small
edits, and loses on no-ops. Next: a resident sync daemon (inotify, push on
save) to take the sync off the critical path, and per-build local or
remote choice from these measurements.

## Defaults

- **Region us-west-2.** Measured round trip from the dev laptop: 29 to 42 ms.
  us-west-1 is closer (21 ms) but the account's vCPU quota there is 5
  (increase requested). Pass `--region`.
- **c7i.8xlarge spot** (32 vCPUs, 64 GB): about $0.50 to $0.55/h spot,
  $1.43/h on demand (`--on-demand`). A persistent spot request with
  `stop` on interruption, so the disk survives.
- **200 GB gp3 at 6000 IOPS / 500 MB/s**, kept across stops, so warm
  target dirs and caches survive.
- **Idle stop after 30 minutes** (`--idle-minutes`). A systemd timer on the
  machine records activity (load above 1.0 or an interactive login) and
  powers off after the idle window. Stop, not terminate: the next
  `remote up` boots in about 30 s with everything still on disk.
- **ssh only from this machine's public IP.** The security group
  `justrust-remote` is updated with the current IP on every `up`. The key is
  `~/.justrust/remote/id_ed25519` and is used only for this machine.

## Local state

```text
~/.justrust/remote/
  state.json     instance id, region, type, spot, idle minutes
  id_ed25519     ssh key for the machine
  known_hosts    its host key
  probe.json     last ssh probe, reused by `status` for 30 s
  price.json     hourly price, cached for an hour
  user-data.sh   cloud-init used at creation
```

## First boot

cloud-init installs build-essential, clang, lld, mold, cmake, protobuf, and
the -sys libraries the Jcode projects link against (wayland, xkbcommon,
alsa, fontconfig, freetype, xcb, vulkan), then rustup stable. It writes
`/var/lib/justrust/ready` at the end, which `status` reports as the machine
leaving `setup`.
