//! toml_edit's tests take `include_bytes!` of fixtures that are not UTF-8, with an extension
//! the daemon loads as text. Such a file had an id and no text, and asking for it panicked.

const LATIN: &[u8] = include_bytes!("not_utf8.md");

pub fn run() -> (usize, &'static [u8]) {
    (LATIN.len(), LATIN)
}
