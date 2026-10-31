#!/bin/bash
# End-to-end checks for `justrust split --apply`, against a fixture cargo
# workspace in git (bench/split-fixture.sh). Uses real cargo, real git.
#   bench/split-apply-e2e.sh [justrust-binary]
# Each case gets a fresh directory under $JCODE_SCRATCH_DIR (or /tmp).
set -u
J="${1:-$(dirname "$0")/../target/debug/justrust}"
J="$(cd "$(dirname "$J")" && pwd)/$(basename "$J")"
FIX="$(cd "$(dirname "$0")" && pwd)/split-fixture.sh"
BASE="${JCODE_SCRATCH_DIR:-/tmp}/split-apply-e2e-$$"
mkdir -p "$BASE"
fail=0
pass() { echo "ok   $1"; }
bad() { echo "FAIL $1"; fail=1; }
snapshot() { (cd "$1" && find . -path ./target -prune -o -path ./.git -prune -o -type f -print | sort | xargs sha256sum); }

# 1. Success: split, commit, clean tree, tests and app still work.
d="$BASE/success"; "$FIX" "$d" >/dev/null
pre=$(git -C "$d" rev-parse HEAD)
if (cd "$d" && "$J" split --apply base::emails >"$BASE/success.log" 2>&1); then
  [ -z "$(git -C "$d" status --porcelain)" ] && pass "success: tree clean after commit" || bad "success: tree not clean"
  [ "$(git -C "$d" rev-parse HEAD~1)" = "$pre" ] && pass "success: one commit on top of pre-split" || bad "success: commit parent"
  git -C "$d" log -1 --format=%B | grep -q "Pre-split commit: ${pre:0:12}" && pass "success: message names pre-split" || bad "success: message"
  (cd "$d" && cargo test --workspace --offline -q 2>&1 | grep -q "3 passed") && pass "success: moved tests pass" || bad "success: tests"
  [ "$(cd "$d" && cargo run -q --offline -p app 2>/dev/null)" = "2 3" ] && pass "success: app output unchanged" || bad "success: app output"
  [ ! -e "$d/crates/base/src/emails" ] && pass "success: emptied module dir removed" || bad "success: module dir left"
else
  bad "success: apply failed"; cat "$BASE/success.log"
fi

# 2. Refuses a dirty tree without touching it.
d="$BASE/dirty"; "$FIX" "$d" >/dev/null; echo x > "$d/stray.txt"
before=$(snapshot "$d")
if (cd "$d" && "$J" split --apply base::emails >"$BASE/dirty.log" 2>&1); then bad "dirty: should refuse"; else
  grep -q "uncommitted or untracked" "$BASE/dirty.log" && pass "dirty: refused" || bad "dirty: wrong error"
  [ "$(snapshot "$d")" = "$before" ] && pass "dirty: tree untouched" || bad "dirty: tree changed"
fi

# 3. Root re-exported names used by a dependent are rewritten.
d="$BASE/reexport"; "$FIX" "$d" >/dev/null
echo 'pub fn g() -> base::Email { base::Email("z".into()) }' >> "$d/crates/mid/src/lib.rs"
git -C "$d" commit -qam reexport-user
if (cd "$d" && "$J" split --apply base::emails >"$BASE/reexport.log" 2>&1); then
  grep -q "base_emails::Email" "$d/crates/mid/src/lib.rs" && pass "reexport: user rewritten" || bad "reexport: user not rewritten"
else bad "reexport: apply failed"; cat "$BASE/reexport.log"; fi

# 4. A behavior change caught by tests: restored byte-for-byte, no commit.
d="$BASE/restore"; "$FIX" "$d" >/dev/null
cat >> "$d/crates/base/src/emails/parse.rs" <<'EOF'
#[cfg(test)]
mod pkg { #[test] fn pkg() { assert_eq!(std::any::type_name::<super::Marker>(), "base::emails::parse::Marker"); } }
pub struct Marker;
EOF
git -C "$d" commit -qam behavior-test
pre=$(git -C "$d" rev-parse HEAD); before=$(snapshot "$d")
if (cd "$d" && "$J" split --apply base::emails >"$BASE/restore.log" 2>&1); then bad "restore: should fail"; else
  grep -q "restored the working tree" "$BASE/restore.log" && pass "restore: reported" || bad "restore: message"
  [ "$(snapshot "$d")" = "$before" ] && pass "restore: tree byte-identical" || bad "restore: tree differs"
  [ "$(git -C "$d" rev-parse HEAD)" = "$pre" ] && pass "restore: HEAD unchanged" || bad "restore: HEAD moved"
  [ -z "$(git -C "$d" status --porcelain)" ] && pass "restore: status clean" || bad "restore: status dirty"
fi

# 5. Refusals: entangled module, crate-identity macro.
d="$BASE/refuse"; "$FIX" "$d" >/dev/null
(cd "$d" && "$J" split --apply base::logging >"$BASE/refuse1.log" 2>&1) && bad "refuse: logging should be entangled" || \
  { grep -q "referenced by other modules" "$BASE/refuse1.log" && pass "refuse: entangled module" || bad "refuse: entangled msg"; }
echo 'pub fn n() -> &'"'"'static str { env!("CARGO_PKG_NAME") }' >> "$d/crates/base/src/emails/parse.rs"
git -C "$d" commit -qam pkgname
(cd "$d" && "$J" split --apply base::emails >"$BASE/refuse2.log" 2>&1) && bad "refuse: CARGO_PKG_NAME should refuse" || \
  { grep -q "CARGO_PKG_NAME" "$BASE/refuse2.log" && pass "refuse: crate-identity macro" || bad "refuse: macro msg"; }

echo
[ $fail = 0 ] && echo "all split --apply e2e checks passed ($BASE)" || echo "FAILURES (logs in $BASE)"
exit $fail
