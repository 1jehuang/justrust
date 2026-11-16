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
    export JUSTRUST_BENCH_RUNTIME_CONTEXT=jcode-isolated-v2
    export JUSTRUST="$work/bin/$label" JUSTRUST_BIN="$work/bin/$label"
    export JUSTRUST_BENCH_REV="$revision"
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
  df -h "$HOME"

  # Complete public histories resolve the exact suite pins, without materializing
  # their current trees. No shallow-tip assumption about 7d38fb7 or abff94eeb.
  local repository
  for repository in jcode jcode-desktop; do
    [[ ! -e "$HOME/$repository" ]] || {
      echo "Refusing to overwrite existing $HOME/$repository" >&2
      return 1
    }
    git clone --filter=blob:none --no-checkout \
      "https://github.com/1jehuang/$repository.git" "$HOME/$repository"
  done
  git -C "$HOME/jcode-desktop" cat-file -e '7d38fb7^{commit}'
  git -C "$HOME/jcode" cat-file -e 'abff94eeb^{commit}'
  python3 "$root/bench/suite.py" preflight
  "$hosted" remote use hosted
  build_binary before "$BASELINE_REF"
  build_binary after "$CANDIDATE_REF"

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
