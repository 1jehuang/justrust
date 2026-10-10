# Remote compile machine

Phase 4 of [toolchain.md](toolchain.md) starts with one dedicated build
machine per user that justrust creates and controls. The build routing
(sync, remote cargo, artifacts back, local or remote chosen per build) comes
next. This page covers the machine itself.

```text
justrust remote up        create the machine, or start it if stopped
justrust remote down      stop it (disk, target dirs, and caches are kept)
justrust remote status    state, cost, round trip, cores, load, toolchain
                          --waybar for a Waybar module, --watch N to refresh
justrust remote ssh [cmd] shell on the machine, or run one command
justrust remote destroy --yes   terminate it and delete its disk
```

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
