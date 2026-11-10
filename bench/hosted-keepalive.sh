#!/usr/bin/env bash
# A bounded authenticated lease for active benchmark work, not an idle pin.
set -euo pipefail
JR=$1 DEST=$2 PIDFILE=$3 BOOT=$4 MAX_SECONDS=$5
shift 5
[[ $BOOT =~ ^[0-9a-f-]{36}$ && $PIDFILE =~ ^[A-Za-z0-9._/-]+$ ]] || exit 2
[[ $MAX_SECONDS =~ ^[0-9]+$ ]] && (( MAX_SECONDS > 0 && MAX_SECONDS <= 86400 )) || exit 2
INTERVAL=${JUSTRUST_BENCH_LEASE_INTERVAL:-60}
[[ $INTERVAL =~ ^[0-9]+$ ]] && (( INTERVAL > 0 && INTERVAL <= 240 )) || exit 2
DEADLINE=$((SECONDS + MAX_SECONDS))
while (( SECONDS < DEADLINE )); do
  # A reused PID after a guest restart must not renew someone else's work.
  # The pidfile belongs to the generated script and disappears on normal exit.
  timeout 30 "$@" "$DEST" "test \"\$(cat /proc/sys/kernel/random/boot_id)\" = '$BOOT' && state=\$(ps -p \"\$(cat '$PIDFILE' 2>/dev/null)\" -o stat= 2>/dev/null) && test -n \"\$state\" && test \"\${state#Z}\" = \"\$state\"" || exit 0
  # Plain remote env can hit a live SSH cache without touching the API.
  # --refresh always reaches the normal authenticated connect/lease path.
  timeout 60 "$JR" remote env --refresh >/dev/null || exit 1
  LEFT=$((DEADLINE - SECONDS))
  (( LEFT > 0 )) || break
  (( LEFT < INTERVAL )) && sleep "$LEFT" || sleep "$INTERVAL"
done
