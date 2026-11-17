#!/usr/bin/env bash
# Clean CI-only, controlled local benchmark. Builds run on hosted, not the runner.
# Never install either candidate, publish a release, or relax suite validity checks.
set -euo pipefail

validate_ref() {
  local revision=$1
  [[ "$revision" =~ ^[0-9a-f]{40}$ ]] || {
    echo 'source refs must be full lowercase 40-character commit SHAs' >&2
    return 1
  }
  git -C "$root" cat-file -e "$revision^{commit}"
  git -C "$root" merge-base --is-ancestor "$revision" refs/remotes/origin/stable
}

import_snapshot_pack() {
  local destination=$1 revision=$2 pack=$3 checksum=$4
  [[ "$revision" =~ ^[0-9a-f]{40}$ && "$checksum" =~ ^[0-9a-f]{64}$ ]] || return 1
  [[ ! -e "$destination" ]] || return 1
  printf '%s  %s\n' "$checksum" "$pack" | sha256sum --check
  git init --quiet --template= --initial-branch=bench-snapshot "$destination"
  git -C "$destination" config core.hooksPath /dev/null
  git -C "$destination" index-pack --stdin < "$pack"
  git -C "$destination" cat-file -e "$revision^{commit}"
  # Exact commit/tree only, no parent history, config, remotes or credentials.
  printf '%s\n' "$revision" > "$destination/.git/shallow"
  git -C "$destination" update-ref HEAD "$revision"
  git -C "$destination" fsck --full --no-reflogs
}

export_source() {
  local revision=$1 destination=$2
  mkdir -p "$destination"
  # Only compile inputs, no vendored compiler/Cargo trees and no worktree.
  git -C "$root" archive "$revision" Cargo.toml Cargo.lock src \
    docs/AGENTS-snippet.md scripts/sign-release.py README.md LICENSE |
    tar -x -C "$destination"
}

build_binary() {
  local label=$1 revision=$2 source="$work/source-$1"
  export_source "$revision" "$source"
  (
    cd "$source"
    # Explicit hosted requirement fails closed rather than silently compiling here.
    export JUSTRUST_TOOLCHAIN=1.99.0 JUSTRUST_TOOLCHAIN_REQUIRED=1
    export JUSTRUST_REMOTE_REQUIRED=1 JUSTRUST_RELEASE_BUILD=0
    unset JUSTRUST_LOCAL JUSTRUST_LOCAL_REASON JUSTRUST_REMOTE CARGO_TARGET_DIR
    "$hosted" build --remote --release --locked --bin justrust \
      --config 'profile.release.lto="thin"' \
      --config profile.release.codegen-units=1 --config profile.release.strip=true
    "$hosted" show
  ) 2>&1 | tee "$evidence/build-$label.log"
  # Frozen outside every target/cache directory, selected only by explicit path.
  install -m 0555 "$source/target/release/justrust" "$work/bin/$label"
  "$work/bin/$label" --version | tee "$evidence/version-$label.log"
  sha256sum "$work/bin/$label" >> "$evidence/binaries.sha256"
}

benchmark_binary() {
  local label=$1 revision=$2
  (
    cd "$root"
    # Intentional local-only measurement on this otherwise idle ephemeral runner.
    # synthetic_stack_controls forwards these to every measured child command.
    export JUSTRUST_LOCAL=1
    export JUSTRUST_LOCAL_REASON='Controlled clean GitHub runner before/after local synthetic benchmark'
    export JUSTRUST_NO_UPDATE=1 JUSTRUST_BENCH_DIRTY=0
    export JUSTRUST_BENCH_RUNTIME_CONTEXT=jcode-isolated-v4
    export JUSTRUST="$work/bin/$label" JUSTRUST_BIN="$work/bin/$label"
    export JUSTRUST_BENCH_REV="$revision"
    # GitHub runners install stock Cargo through rustup, not /usr/bin/cargo.
    # The unchanged generator supports this explicit probe-driver selection.
    export VERSUS_STOCK_CARGO="$(command -v cargo)"
    unset JUSTRUST_REMOTE_REQUIRED JUSTRUST_TOOLCHAIN_REQUIRED JUSTRUST_TOOLCHAIN CARGO_TARGET_DIR
    python3 bench/suite.py run --synth 1 --points 40 -n 1 --stop-on-invalid \
      -o "$evidence/$label.json"
  ) 2>&1 | tee "$evidence/$label.log"
}

main() {
  : "${GITHUB_ACTIONS:?This helper is for clean GitHub Actions runners only}"
  [[ "$GITHUB_ACTIONS" == true ]] || return 1
  : "${RUNNER_TEMP:?}" "${BASELINE_REF:?}" "${CANDIDATE_REF:?}" "${JCODE_API_KEY:?}"
  root=$(git rev-parse --show-toplevel)
  [[ $(git -C "$root" branch --show-current) == stable ]]
  work="$RUNNER_TEMP/source-sync-release-gate"
  evidence="$work/evidence"
  mkdir -p "$evidence" "$work/bin"
  exec > >(tee -a "$evidence/gate.log") 2>&1
  validate_ref "$BASELINE_REF"
  validate_ref "$CANDIDATE_REF"
  hosted=$(command -v justrust)
  [[ "$hosted" == /* ]]
  printf 'baseline=%s\ncandidate=%s\nharness=%s\nbootstrap=%s\n' \
    "$BASELINE_REF" "$CANDIDATE_REF" "$(git -C "$root" rev-parse HEAD)" \
    "${HOSTED_CLIENT_REV:?}" > "$evidence/identity.txt"
  git -C "$root" ls-tree -r HEAD bench > "$evidence/harness-tree.txt"
  rustc -Vv | tee "$evidence/rustc.log"
  python3 "$root/ci/check-shell-environment.py" | tee "$evidence/shell.log"
  python3 "$root/ci/check-test-utilities.py" | tee "$evidence/test-utilities.log"
  df -h "$HOME"

  # Public jcode history lacks this pin, even when fetching its full SHA.
  # Use a private expiring download, never a repin or unrelated local history.
  : "${SOURCE_SYNC_JCODE_PACK_URL:?Private pinned jcode pack URL required}"
  local pack="$work/jcode-pin.pack"
  curl --fail --silent --show-error --location --proto '=https' \
    "$SOURCE_SYNC_JCODE_PACK_URL" -o "$pack"
  import_snapshot_pack "$HOME/jcode" \
    abff94eeb2b432c3f263dd35f200aa6345a6eafd "$pack" \
    6f5d83a8897ec43fe5fa591556a0e9a326bf8b9c4c46bf259b22253a11ff502c
  [[ ! -e "$HOME/jcode-desktop" ]] || return 1
  git clone --filter=blob:none --no-checkout \
    https://github.com/1jehuang/jcode-desktop.git "$HOME/jcode-desktop"
  git -C "$HOME/jcode-desktop" cat-file -e '7d38fb72a0beadbca366da1104c915689cc0fd8e^{commit}'
  # Fixtures make real commits under private HOME. Use the exact fixture pin's
  # author, never a fabricated agent identity or persistent/global Git config.
  local fixture_identity
  fixture_identity=$(python3 "$root/bench/git_fixture_identity.py" "$HOME/jcode-desktop" \
    7d38fb72a0beadbca366da1104c915689cc0fd8e)
  eval "$fixture_identity"
  git -C "$root" cat-file -e 'f97fcc2^{commit}'
  python3 "$root/bench/suite.py" preflight
  "$hosted" remote use hosted
  build_binary before "$BASELINE_REF"
  build_binary after "$CANDIDATE_REF"
  python3 "$root/ci/check-pinned-runtime.py" "$work/bin/before" "$work"

  # One unchanged suite, generator, weights, snapshots and warm target path.
  # Serial execution preserves the suite's normal cross-commit cache behavior.
  # Any invalid/noisy point aborts the gate and is retained, never promoted.
  benchmark_binary before "$BASELINE_REF"
  benchmark_binary after "$CANDIDATE_REF"
  (
    cd "$root"
    python3 bench/suite.py compare "$evidence/before.json" "$evidence/after.json" --require-valid
  ) 2>&1 | tee "$evidence/compare.log"
  sha256sum --check "$evidence/binaries.sha256"
}

if [[ "${BASH_SOURCE[0]}" == "$0" ]]; then
  main "$@"
fi
