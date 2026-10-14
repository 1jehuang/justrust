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
# Works with the hosted backend (Jcode build fleet) and the legacy aws one:
# `justrust remote env` gives the ssh destination and options of whichever
# machine remote builds use.
#
# What `start` does:
#  1. starts the machine if needed, copies this justrust binary to
#     ~/bench-bin there (same arch, never the agent's own binary) and syncs this repo's working tree to ~/bench-justrust (not the
#     build mirror at /home/jeremy/justrust, which the sync daemon owns);
#  2. makes the pinned snapshot trees exist there: for every repo in
#     bench/suite.py SNAPSHOTS, `git archive` of the pinned commit goes to
#     ~/bench-src/<repo>@<rev>.tar, which suite.py extracts instead of
#     calling git (JUSTRUST_BENCH_SRC);
#  3. aws: lowers kernel.perf_event_paranoid to 1 for the instruction
#     counters (hosted machines are set up that way at boot);
#  4. runs the command in tmux session NAME with output in ~/bench-NAME.log,
#     marking the machine active every minute so its idle stop waits.
#
# Results land in ~/bench-justrust/bench/results on the machine; `fetch`
# copies the ones not here yet, renamed with a `-remote-<machine>` suffix.
set -euo pipefail
cd "$(dirname "$0")/.."
JR=${JUSTRUST:-justrust}
NAME=bench

KIND="" DEST="" SSH=()
# The current backend's ssh destination (starts the machine if needed). One
# ControlMaster carries every later ssh and rsync.
resolve() {
  [[ -n $KIND ]] && return
  local env
  env=$("$JR" remote env) || { echo "on-remote: no remote machine (justrust login)" >&2; exit 1; }
  local JR_REMOTE_KIND JR_REMOTE_DEST JR_REMOTE_SSH
  eval "$env"
  KIND=$JR_REMOTE_KIND DEST=$JR_REMOTE_DEST
  eval "SSH=($JR_REMOTE_SSH)"
}
ssh_t() { resolve; "${SSH[@]}" "$DEST" "$@"; }
# Same, with stdin closed (inside `while read` loops).
ssh_n() { ssh_t "$@" </dev/null; }
rsync_t() {
  resolve
  local e; printf -v e '%q ' "${SSH[@]}"
  rsync -az -e "$e" "${@/#R:/$DEST:}"
}
# Keeps the machine's idle stop from firing: the aws watchdog reads
# /var/lib/justrust/last-active, hosted machines a marker in the home dir.
MARK_ACTIVE='if [ -d /var/lib/justrust ]; then date +%s | sudo -n tee /var/lib/justrust/last-active >/dev/null; else touch ~/.jcode-build-active; fi'

cmd=${1:-}; shift || true
if [[ ${1:-} == --name ]]; then NAME=$2; shift 2; fi
if [[ $cmd != start && -n ${1:-} ]]; then NAME=$1; fi

case "$cmd" in
start)
  [[ ${1:-} == -- ]] && shift
  (( $# )) || { echo "usage: $0 start [--name N] -- <bench command>" >&2; exit 2; }
  resolve
  for _ in $(seq 60); do ssh_n 'test -f /var/lib/justrust/ready -o -f /opt/jcode-build/ready' 2>/dev/null && break; sleep 3; done
  # The idle stop ignores uploads and quiet benchmark phases. Mark the
  # machine active now, and from the run script every minute while the
  # benchmark runs.
  ssh_t "$MARK_ACTIVE"
  if ssh_t "tmux has-session -t $NAME 2>/dev/null"; then
    echo "on-remote: session $NAME is already running (status/attach, or --name)" >&2; exit 1
  fi
  echo "on-remote: syncing justrust binary and repo" >&2
  # Upload then rename: replaces the file (or a stale symlink) atomically,
  # never writes through a link into some other path.
  # Its own directory: ~/.cargo/bin/justrust belongs to the remote build
  # agent, and replacing it under the running agent broke every remote
  # build until the agent restarted.
  ssh_t 'mkdir -p ~/bench-bin'
  rsync_t "$(readlink -f "$(command -v "$JR")")" R:bench-bin/.justrust.upload
  ssh_t 'chmod +x ~/bench-bin/.justrust.upload && mv -f ~/bench-bin/.justrust.upload ~/bench-bin/justrust'
  # Results include raw profiles and nested evidence, not only top-level
  # JSON. Never delete a completed host experiment while syncing sources.
  rsync_t --delete --exclude target/ --exclude .git/ --exclude 'bench/results/' ./ R:bench-justrust/
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
    ssh_n "$MARK_ACTIVE"
    echo "on-remote: pushing $repo@$rev" >&2
    git -C "$src" archive "$rev" | ssh_t "cat >bench-src/$tar.part && mv bench-src/$tar.part bench-src/$tar"
  done <"${TMPDIR:-/tmp}/on-remote-pins.$$"
  rm -f "${TMPDIR:-/tmp}/on-remote-pins.$$"
  if [[ $KIND == aws ]]; then ssh_t 'sudo sysctl -q kernel.perf_event_paranoid=1'; fi
  if [[ $(ssh_n 'cat /proc/sys/kernel/perf_event_paranoid') -gt 1 ]]; then
    echo "on-remote: perf_event_paranoid > 1 there, instruction counts will be missing" >&2
  fi
  rev=$(git rev-parse --short=12 HEAD)
  dirty=$([[ -n $(git status --porcelain --untracked-files=no) ]] && echo 1 || echo 0)
  {
    echo '#!/bin/bash'
    echo 'cd ~/bench-justrust'
    printf '( while sleep 60; do %s; done ) &\n' "$MARK_ACTIVE"
    echo 'trap "kill $! 2>/dev/null" EXIT'
    echo 'export PATH=$HOME/bench-bin:$HOME/.cargo/bin:$PATH JUSTRUST_BENCH_SRC=$HOME/bench-src JUSTRUST_REMOTE=0'
    # The machine has rustup's stable, not a distro cargo: that is "stock" there.
    echo '[ -x /usr/bin/cargo ] || export VERSUS_STOCK_CARGO=$HOME/.cargo/bin/cargo'
    # The executable copied above may be older than this source checkout.
    # Repository HEAD is not evidence of the installed binary's revision.
    echo "unset JUSTRUST_BENCH_REV"
    echo "export JUSTRUST_BENCH_REPO_REV=$rev JUSTRUST_BENCH_DIRTY=$dirty"
    echo 'date'
    printf '%q ' "$@"; echo
    # Logging the finish time must not turn a failed payload into script success.
    echo 'status=$?'
    echo 'printf "EXIT %s\n" "$status"'
    echo 'date'
    echo 'exit "$status"'
  } | ssh_t "cat >~/bench-$NAME.sh && chmod +x ~/bench-$NAME.sh"
  ssh_t "tmux new-session -d -s $NAME 'bash ~/bench-$NAME.sh >~/bench-$NAME.log 2>&1'"
  echo "on-remote: started in tmux session $NAME on $DEST ($KIND); log ~/bench-$NAME.log there"
  echo "  $0 status $NAME    $0 fetch $NAME"
  if [[ $KIND == hosted ]]; then
    # Hosted machines are stopped by the control plane's cron after
    # BUILD_IDLE_STOP_MINUTES with no connect, charge or run event; the
    # in-machine marker above does not count. Keep connecting from here
    # (an authenticated connect is activity) while the session runs.
    nohup bash -c 'while '"${SSH[*]@Q}"' '"$DEST"' "tmux has-session -t '"$NAME"' 2>/dev/null"; do '"$JR"' remote env >/dev/null 2>&1; sleep 240; done' \
      >"${TMPDIR:-/tmp}/on-remote-keepalive-$NAME.log" 2>&1 &
    echo "on-remote: hosted keepalive pid $! (connects every 4 min until the session ends)"
  fi
  ;;
status)
  if ssh_t "tmux has-session -t $NAME 2>/dev/null"; then echo "running"; else echo "finished (or not started)"; fi
  ssh_t "grep -v '^JCODE_PROGRESS' ~/bench-$NAME.log 2>/dev/null | tail -15; grep '^JCODE_PROGRESS' ~/bench-$NAME.log 2>/dev/null | tail -1"
  ;;
fetch)
  # The machine's own EC2 type (IMDSv2), the same label for both backends.
  machine=$(ssh_n 't=$(curl -sf -m 2 -X PUT -H "X-aws-ec2-metadata-token-ttl-seconds: 60" http://169.254.169.254/latest/api/token) && curl -sf -m 2 -H "X-aws-ec2-metadata-token: $t" http://169.254.169.254/latest/meta-data/instance-type' || true)
  machine=${machine:-remote}
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
  resolve
  exec "${SSH[@]}" -t "$DEST" "tmux attach -t $NAME"
  ;;
*)
  sed -n '2,30p' "$0"; exit 2 ;;
esac
