#!/usr/bin/env bash
# Peer cache between this machine and the remote build machine, measured
# end to end on a pinned Jcode snapshot (`check -p jcode-base`, 376
# dependency units, pinned nightly).
#
#   bench/peer-cache.sh <snapshot-dir>
#
# The snapshot must be a git checkout under $HOME (so the build machine
# mirrors it at the same path) with a justrust.toml pin. Needs a running
# remote backend. Uses a private JUSTRUST_HOME, so the real caches are not
# touched, and wipes the machine's dependency cache and the snapshot's
# target dir there before each case.
#
# Cases (wall seconds as the agent sees them):
#   remote-cold    remote build, both caches empty (baseline for the machine)
#   local-cold     local build, peer off, local cache empty (baseline here)
#   local-pull     local build, local cache empty, machine cache warm
#   remote-pull    remote build, machine cache empty, local cache warm
#   synced         after a remote build: local build with the peer off,
#                  using only what the daemon's background sync pulled
# Old target dirs are left in the scratch dir: delete peer-bench/t-* after.
set -euo pipefail
snap=${1:?usage: bench/peer-cache.sh <snapshot-dir>}
scratch=${JCODE_SCRATCH_DIR:-/tmp}/peer-bench
home="$scratch/home"
out="$scratch/results.txt"
mkdir -p "$scratch"
: > "$out"

fresh_home() {
    # Stop the private home's sync daemon (it holds the relay and syncs in
    # the background) before wiping its cache.
    [ -d "$home/remote" ] && JUSTRUST_HOME="$home" justrust remote use off >/dev/null 2>&1 || true
    rm -rf "$home"
    mkdir -p "$home/remote" "$home/toolchains" "$home/cache/depcache"
    cp ~/.justrust/remote/{backend.json,state.json,id_ed25519,known_hosts,ip} "$home/remote/" 2>/dev/null || true
    for t in ~/.justrust/toolchains/*/; do ln -sfn "$t" "$home/toolchains/$(basename "$t")"; done
    # The compilers this machine builds with, so the background pull takes
    # entries for them.
    cp ~/.justrust/cache/depcache/peer-toolchains "$home/cache/depcache/" 2>/dev/null || true
}

wipe_remote() {
    justrust remote ssh "rm -rf '$snap/target' ~/.justrust/cache/depcache" >/dev/null 2>&1
}

run() {
    local name=$1; shift
    local t0 t1 line
    t0=$(date +%s.%N)
    line=$(cd "$snap" && env JUSTRUST_HOME="$home" "$@" justrust check -p jcode-base 2>&1 | grep -E '^  units' | head -1)
    t1=$(date +%s.%N)
    printf '%-12s %6.1fs  %s\n' "$name" "$(echo "$t1 - $t0" | python3 -c 'print(eval(input()))')" "$line" | tee -a "$out"
}

# Each local case gets a target dir no earlier case used.
tdir() { echo "$scratch/t-$$-$1"; }

# 1. Baselines, both caches empty.
fresh_home; wipe_remote
run remote-cold JUSTRUST_REMOTE=force JUSTRUST_PEER_CACHE=0
fresh_home
run local-cold JUSTRUST_REMOTE=off JUSTRUST_PEER_CACHE=0 CARGO_TARGET_DIR="$(tdir cold)"

# 2. local-pull: machine warm (step 1), local cache empty, fetched on miss.
fresh_home
run local-pull JUSTRUST_REMOTE=off CARGO_TARGET_DIR="$(tdir pull)"

# 3. remote-pull: local cache warm (step 2), machine cache empty.
wipe_remote
run remote-pull JUSTRUST_REMOTE=force

# 4. synced: after a remote build, no build-time fetch, only what the
#    daemon's background sync pulled into the (empty) local cache.
fresh_home; wipe_remote
run remote-cold2 JUSTRUST_REMOTE=force
sleep 60
run synced JUSTRUST_REMOTE=off JUSTRUST_PEER_CACHE=0 CARGO_TARGET_DIR="$(tdir synced)"

echo "results in $out"
