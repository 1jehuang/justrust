#!/usr/bin/env bash
# Bootstrap a reviewed, immutable hosted client without a published release.
# This is intentionally separate from the tag being built by release CI.
set -euo pipefail
revision=${1:?reviewed client commit required}
[[ "$revision" =~ ^[0-9a-f]{40}$ ]] || {
  echo 'hosted client must be pinned to a full commit SHA' >&2
  exit 1
}
work=$(mktemp -d "${RUNNER_TEMP:-${JCODE_SCRATCH_DIR:-${TMPDIR:-/tmp}}}/justrust-client.XXXXXX")
git init -q "$work/source"
git -C "$work/source" remote add origin https://github.com/1jehuang/justrust.git
git -C "$work/source" sparse-checkout set --no-cone \
  '/Cargo.toml' '/Cargo.lock' '/src/' '/docs/AGENTS-snippet.md' '/README.md' '/LICENSE*'
git -C "$work/source" fetch --quiet --depth=1 --filter=blob:none origin "$revision"
git -C "$work/source" checkout --quiet --detach FETCH_HEAD
[[ $(git -C "$work/source" rev-parse HEAD) == "$revision" ]]
# Do not mark the bootstrap as a published auto-updating release. The output
# being built later gets JUSTRUST_RELEASE_BUILD=1, not this bootstrap client.
JUSTRUST_RELEASE_BUILD=0 CARGO_TARGET_DIR="$work/target" \
  cargo +1.99.0 install --path "$work/source" --locked --root "$work/install"
"$work/install/bin/justrust" --version
if [[ -n ${GITHUB_PATH:-} ]]; then
  printf '%s\n' "$work/install/bin" >> "$GITHUB_PATH"
fi
printf 'hosted bootstrap client: %s\n' "$work/install/bin/justrust"
