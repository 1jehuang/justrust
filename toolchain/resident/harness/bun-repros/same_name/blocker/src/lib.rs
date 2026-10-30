// bun's `bun_sema`, a library, and `bun-sema`, a binary of `bun_sema_standalone` that bun does
// not link: both crates are called `bun_sema`.
pub fn greeting() -> &'static str {
    "linked the library"
}
