#!/usr/bin/env bash
# Run a benchmark on the remote build machine, so numbers from the machine
# remote builds actually use are measured there, not guessed from laptop runs.
#
#   bench/on-remote.sh start [--name NAME] -- <bench command...>
#       e.g. bench/on-remote.sh start -- bench/suite.py run --synth 1 -n 1
#            bench/on-remote.sh start -- bench/versus.py run --synth 1
#   bench/on-remote.sh status [NAME]     is it running, last progress lines
#   bench/on-remote.sh fetch [NAME]      copy new bench/results/*.json back here
#   bench/on-remote.sh attach [NAME]     the tmux session (detach: C-b d)
#
# What `start` does:
#  1. starts the machine if needed (`justrust remote up`), copies this
#     justrust binary there (same arch) and syncs this repo's working tree to
#     ~/bench-justrust (not the build mirror at /home/jeremy/justrust, which
#     the sync daemon owns);
#  2. makes the pinned snapshot trees exist there: for every repo in
#     bench/suite.py SNAPSHOTS, `git archive` of the pinned commit goes to
#     ~/bench-src/<repo>@<rev>.tar, which suite.py extracts instead of
#     calling git (JUSTRUST_BENCH_SRC);
#  3. lowers kernel.perf_event_paranoid to 1 for the instruction counters;
#  4. runs the command in tmux session NAME with output in ~/bench-NAME.log.
#     A running benchmark keeps the load above 1, which keeps the machine's
#     idle watchdog from stopping it.
#
# Results land in ~/bench-justrust/bench/results on the machine; `fetch`
# copies the ones not here yet, renamed with a `-remote-<machine>` suffix.
set -euo pipefail
cd "$(dirname "$0")/.."
JR=${JUSTRUST:-justrust}
RDIR="$HOME/.justrust/remote"
NAME=bench

IP=""
# The machine's current IP (it changes on every start), from a fresh describe.
resolve_ip() {
  IP=$("$JR" remote status --json | jq -r 'select(.state == "running") | .public_ip // empty')
  [[ -n $IP ]] || { echo "on-remote: the machine is not running" >&2; exit 1; }
}
ssh_t() {
  [[ -n $IP ]] || resolve_ip
  ssh -i "$RDIR/id_ed25519" -o UserKnownHostsFile="$RDIR/known_hosts" -o StrictHostKeyChecking=accept-new \
      -o ControlMaster=auto -o ControlPath="$RDIR/cm-bench-%C" -o ControlPersist=600 \
      -o ServerAliveInterval=15 "ubuntu@$IP" "$@"
}
# Same, with stdin closed (inside `while read` loops).
ssh_n() { ssh_t "$@" </dev/null; }
rsync_t() {
  [[ -n $IP ]] || resolve_ip
  rsync -az -e "ssh -i $RDIR/id_ed25519 -o UserKnownHostsFile=$RDIR/known_hosts -o ControlMaster=auto -o ControlPath=$RDIR/cm-bench-%C -o ControlPersist=600" \
      "${@/#R:/ubuntu@$IP:}"
}

cmd=${1:-}; shift || true
if [[ ${1:-} == --name ]]; then NAME=$2; shift 2; fi
if [[ $cmd != start && -n ${1:-} ]]; then NAME=$1; fi

case "$cmd" in
start)
  [[ ${1:-} == -- ]] && shift
  (( $# )) || { echo "usage: $0 start [--name N] -- <bench command>" >&2; exit 2; }
  [[ $(jq -r .kind "$RDIR/backend.json" 2>/dev/null) == aws ]] || {
    echo "on-remote: needs the aws backend (justrust remote use aws)" >&2; exit 1; }
  "$JR" remote up >/dev/null
  resolve_ip
  for _ in $(seq 60); do ssh_n 'test -f /var/lib/justrust/ready' 2>/dev/null && break; sleep 3; done
  # The idle watchdog only counts tty logins and load > 1: uploads and quiet
  # benchmark phases look idle. Mark the machine active now, and from the
  # run script every minute while the benchmark runs.
  ssh_t 'date +%s | sudo tee /var/lib/justrust/last-active >/dev/null'
  if ssh_t "tmux has-session -t $NAME 2>/dev/null"; then
    echo "on-remote: session $NAME is already running (status/attach, or --name)" >&2; exit 1
  fi
  echo "on-remote: syncing justrust binary and repo" >&2
  # Upload then rename: replaces the file (or a stale symlink) atomically,
  # never writes through a link into some other path.
  rsync_t "$(readlink -f "$(command -v "$JR")")" R:.cargo/bin/.justrust.bench-upload
  ssh_t 'chmod +x ~/.cargo/bin/.justrust.bench-upload && mv -f ~/.cargo/bin/.justrust.bench-upload ~/.cargo/bin/justrust'
  rsync_t --delete --exclude target/ --exclude .git/ --exclude 'bench/results/*.json' ./ R:bench-justrust/
  ssh_t 'mkdir -p bench-justrust/bench/results bench-src'
  # Pinned snapshot trees, as `git archive` tarballs (only what is missing).
  python3 - <<'PY' >"${TMPDIR:-/tmp}/on-remote-pins.$$"
import sys
sys.path.insert(0, "bench")
import suite
for snap in suite.SNAPSHOTS.values():
    for repo, (src, rev) in snap["repos"].items():
        print(repo, src, rev)
PY
  while read -r repo src rev; do
    tar="$repo@$rev.tar"
    if ssh_n "test -s bench-src/$tar"; then continue; fi
    ssh_n 'date +%s | sudo tee /var/lib/justrust/last-active >/dev/null'
    echo "on-remote: pushing $repo@$rev" >&2
    git -C "$src" archive "$rev" | ssh_t "cat >bench-src/$tar.part && mv bench-src/$tar.part bench-src/$tar"
  done <"${TMPDIR:-/tmp}/on-remote-pins.$$"
  rm -f "${TMPDIR:-/tmp}/on-remote-pins.$$"
  ssh_t 'sudo sysctl -q kernel.perf_event_paranoid=1'
  rev=$(git rev-parse --short=12 HEAD)
  dirty=$([[ -n $(git status --porcelain --untracked-files=no) ]] && echo 1 || echo 0)
  {
    echo '#!/bin/bash'
    echo 'cd ~/bench-justrust'
    echo '( while sleep 60; do date +%s | sudo tee /var/lib/justrust/last-active >/dev/null; done ) &'
    echo 'trap "kill $! 2>/dev/null" EXIT'
    echo 'export PATH=$HOME/.cargo/bin:$PATH JUSTRUST_BENCH_SRC=$HOME/bench-src JUSTRUST_REMOTE=0'
    # The machine has rustup's stable, not a distro cargo: that is "stock" there.
    echo '[ -x /usr/bin/cargo ] || export VERSUS_STOCK_CARGO=$HOME/.cargo/bin/cargo'
    echo "export JUSTRUST_BENCH_REV=$rev JUSTRUST_BENCH_DIRTY=$dirty"
    echo 'date'
    printf '%q ' "$@"; echo
    echo 'echo EXIT $?'
    echo 'date'
  } | ssh_t "cat >~/bench-$NAME.sh && chmod +x ~/bench-$NAME.sh"
  ssh_t "tmux new-session -d -s $NAME 'bash ~/bench-$NAME.sh >~/bench-$NAME.log 2>&1'"
  echo "on-remote: started in tmux session $NAME on $IP; log ~/bench-$NAME.log there"
  echo "  $0 status $NAME    $0 fetch $NAME"
  ;;
status)
  if ssh_t "tmux has-session -t $NAME 2>/dev/null"; then echo "running"; else echo "finished (or not started)"; fi
  ssh_t "grep -v '^JCODE_PROGRESS' ~/bench-$NAME.log 2>/dev/null | tail -15; grep '^JCODE_PROGRESS' ~/bench-$NAME.log 2>/dev/null | tail -1"
  ;;
fetch)
  machine=$("$JR" remote status --json | jq -r '.instance_type // "remote"')
  tmp=$(mktemp -d)
  rsync_t "R:bench-justrust/bench/results/" "$tmp/"
  n=0
  for f in "$tmp"/*.json; do
    [[ -e $f ]] || continue
    dst="bench/results/$(basename "${f%.json}")-remote-$machine.json"
    [[ -e $dst ]] && continue
    cp "$f" "$dst"; echo "fetched $dst"; n=$((n + 1))
  done
  rm -r "$tmp"
  echo "on-remote: $n new result(s)"
  ;;
attach)
  resolve_ip
  exec ssh -t -i "$RDIR/id_ed25519" -o UserKnownHostsFile="$RDIR/known_hosts" "ubuntu@$IP" "tmux attach -t $NAME"
  ;;
*)
  sed -n '2,27p' "$0"; exit 2 ;;
esac
