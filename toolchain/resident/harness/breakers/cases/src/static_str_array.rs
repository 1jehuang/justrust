//! An array of string literals borrowed for `'static` by a method call: each element is a
//! reborrow `&*"..."`, and the array is still a constant. openssl-probe's
//! `candidate_cert_dirs` returns an iterator over one.

use std::path::Path;

fn names() -> impl Iterator<Item = &'static Path> {
    [
        "/var/ssl",
        "/usr/share/ssl",
        #[cfg(target_os = "android")]
        "/data/data/com.termux/files/usr/etc/tls",
        "/etc/ssl",
    ]
    .iter()
    .map(Path::new)
    .filter(|path| path.is_absolute())
}

fn plain() -> std::slice::Iter<'static, &'static str> {
    ["one", "two", "three"].iter()
}

#[inline(never)]
fn clobber(depth: u32) -> u64 {
    let filler = [depth as u64 + 0x5a5a_5a5a; 64];
    if depth == 0 { filler.iter().sum() } else { clobber(depth - 1) ^ filler[7] }
}

pub fn run() -> Vec<String> {
    let dirs = names();
    let words = plain();
    let mut out = vec![clobber(8).to_string()];
    out.extend(dirs.map(|path| path.join("certs").display().to_string()));
    out.extend(words.map(|word| word.to_string()));
    out
}
