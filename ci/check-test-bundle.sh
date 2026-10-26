#!/usr/bin/env bash
# Native acceptance for the portable bundle contract, using an isolated fixture.
set -euo pipefail
bin=${1:?usage: check-test-bundle.sh /absolute/path/to/justrust}
python=${PYTHON:-python3}
scratch=$(mktemp -d "${RUNNER_TEMP:-${TMPDIR:-/tmp}}/justrust-test-bundle.XXXXXX")
scratch=$(cd "$scratch" && pwd -P)
trap 'rm -rf -- "$scratch"' EXIT
mkdir "$scratch/src"
cat > "$scratch/Cargo.toml" <<'EOF'
[package]
name = "justrust-local-bundle-smoke"
version = "0.0.0"
edition = "2021"
[workspace]
EOF
cat > "$scratch/src/lib.rs" <<'EOF'
#[test]
fn passing() { assert_eq!(20 + 22, 42); }
#[test]
#[ignore]
fn deliberate_failure() { panic!("expected bundle smoke-test failure"); }
EOF
host=$(rustc -vV | awk '/^host:/ {print $2}' | tr -d '\r')
cd "$scratch"
JUSTRUST_TOOLCHAIN=system JUSTRUST_TOOLCHAIN_AUTO_INSTALL=0 \
  "$bin" test --local --offline --no-run --target "$host" \
  --target-dir "$scratch/target" --message-format=json > messages.jsonl
"$bin" test-bundle pack \
  --target "$host" --messages messages.jsonl --artifact-root target --out bundle
"$bin" test-bundle run \
  --bundle bundle --results passed.json -- passing --exact
# The failing invocation must preserve failure, not merely produce a result file.
set +e
"$bin" test-bundle run \
  --bundle bundle --results failed.json -- deliberate_failure --exact --ignored
code=$?
set -e
if [[ "$code" != 1 ]]; then
  echo "expected bundle runner exit 1, got $code" >&2
  exit 1
fi
"$python" - <<'PY'
import json
from pathlib import Path
manifest = json.loads(Path('bundle/manifest.json').read_text())
passed = json.loads(Path('passed.json').read_text())
failed = json.loads(Path('failed.json').read_text())
assert manifest['test_status'] == 'not-tested', manifest
assert passed['test_status'] == 'passed', passed
assert failed['test_status'] == 'failed', failed
assert len(passed['tests']) == len(failed['tests']) == 1
assert passed['tests'][0]['exit_code'] == 0, passed
assert failed['tests'][0]['exit_code'] != 0, failed
print('Native bundle success, failure, and immutable build status verified')
PY
