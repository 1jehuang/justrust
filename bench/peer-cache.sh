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
#   synced         the background sync after remote-cold: local build with
#                  only what the daemon pulled in the background
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

# 1. remote-cold, then let the background sync pull, then measure synced.
fresh_home; wipe_remote
run remote-cold JUSTRUST_REMOTE=force
sleep 20
run synced JUSTRUST_REMOTE=off CARGO_TARGET_DIR="$scratch/t-synced"

# 2. local-cold baseline.
fresh_home
run local-cold JUSTRUST_REMOTE=off JUSTRUST_PEER_CACHE=0 CARGO_TARGET_DIR="$scratch/t-cold"

# 3. local-pull: machine warm (from step 1's remote build is gone with the
#    wipe, so warm it again with the peer off), local empty.
fresh_home; wipe_remote
run remote-warm JUSTRUST_REMOTE=force JUSTRUST_PEER_CACHE=0
fresh_home
run local-pull JUSTRUST_REMOTE=off CARGO_TARGET_DIR="$scratch/t-pull"

# 4. remote-pull: local warm (from step 3), machine empty.
wipe_remote
run remote-pull JUSTRUST_REMOTE=force

echo "results in $out"
