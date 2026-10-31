#!/bin/bash
# Build a fixture cargo workspace for `justrust split --apply` in $1 (must not exist).
set -e
R="$1"
test ! -e "$R"
mkdir -p "$R/crates/base/src/net" "$R/crates/base/src/emails" "$R/crates/mid/src" "$R/app/src"
cd "$R"
cat > Cargo.toml <<'EOF'
[workspace]
members = [
    "crates/base",
    "crates/mid",
    "app",
]
resolver = "2"

[workspace.package]
edition = "2021"
EOF
cat > crates/base/Cargo.toml <<'EOF'
[package]
name = "base"
version = "0.1.0"
edition.workspace = true

[features]
test-support = []

[dependencies]
serde = { version = "1", features = ["derive"] }
EOF
cat > crates/base/src/lib.rs <<'EOF'
pub mod logging;
pub mod emails;
pub mod net;
pub use emails::Email;

#[cfg(any(test, feature = "test-support"))]
pub fn test_lock() {}
EOF
echo 'pub fn log(s: &str) -> usize { s.len() }' > crates/base/src/logging.rs
cat > crates/base/src/emails.rs <<'EOF'
//! Email helpers.
use crate::logging;
mod parse;

#[derive(serde::Serialize, Debug, Clone, PartialEq)]
pub struct Email(pub String);

pub(crate) fn domain(e: &Email) -> &str { parse::after_at(&e.0) }

pub fn send(e: &Email) -> usize { logging::log(domain(e)) }

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn sends() { crate::test_lock(); assert_eq!(send(&Email("a@bc".into())), 2); }
    #[test]
    fn domain_works() { assert_eq!(domain(&Email("x@y".into())), "y"); }
}
EOF
cat > crates/base/src/emails/parse.rs <<'EOF'
pub(super) fn after_at(s: &str) -> &str { s.split('@').nth(1).unwrap_or("") }
#[cfg(test)]
mod tests { #[test] fn at() { assert_eq!(super::after_at("a@b"), "b"); } }
EOF
echo 'pub mod http;' > crates/base/src/net/mod.rs
echo 'pub fn get() -> usize { crate::logging::log("x") }' > crates/base/src/net/http.rs
cat > crates/mid/Cargo.toml <<'EOF'
[package]
name = "mid"
version = "0.1.0"
edition.workspace = true

[dependencies]
base = { path = "../base", default-features = false }
EOF
cat > crates/mid/src/lib.rs <<'EOF'
pub use base::*;
pub fn f() -> usize { crate::emails::send(&crate::emails::Email("q@rs".into())) + crate::net::http::get() }
EOF
cat > app/Cargo.toml <<'EOF'
[package]
name = "app"
version = "0.1.0"
edition.workspace = true

[dependencies]
base = { path = "../crates/base" }
mid = { path = "../crates/mid" }
EOF
cat > app/src/main.rs <<'EOF'
use base::{logging, emails::Email};
fn main() { let e = Email("a@b".into()); println!("{} {}", base::emails::send(&e) + logging::log("z"), mid::f()); }
EOF
printf 'target/\n' > .gitignore
cargo generate-lockfile --offline -q
git init -q
git add -A
git -c user.name=T -c user.email=t@t commit -qm init
