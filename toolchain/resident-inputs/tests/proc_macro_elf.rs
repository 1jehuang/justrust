#[path = "../src/proc_macro.rs"]
mod proc_macro;
use proc_macro::validate_proc_macro_elf as validate;

fn put16(b: &mut [u8], at: usize, v: u16) {
    b[at..at + 2].copy_from_slice(&v.to_le_bytes());
}
fn put32(b: &mut [u8], at: usize, v: u32) {
    b[at..at + 4].copy_from_slice(&v.to_le_bytes());
}
fn put64(b: &mut [u8], at: usize, v: u64) {
    b[at..at + 8].copy_from_slice(&v.to_le_bytes());
}
fn fixture(names: &[&str], extra: &[(u64, u64)]) -> Vec<u8> {
    let mut strings = vec![0];
    let mut entries = vec![(5, 0x10400)];
    for name in names {
        entries.push((1, strings.len() as u64));
        strings.extend_from_slice(name.as_bytes());
        strings.push(0);
    }
    entries.push((10, strings.len() as u64));
    entries.extend_from_slice(extra);
    entries.push((0, 0));
    let mut b = vec![0; 0x400 + strings.len()];
    b[..7].copy_from_slice(b"\x7fELF\x02\x01\x01");
    put16(&mut b, 16, 3);
    put16(&mut b, 18, 62);
    put32(&mut b, 20, 1);
    put64(&mut b, 32, 64);
    put16(&mut b, 52, 64);
    put16(&mut b, 54, 56);
    put16(&mut b, 56, 2);
    put32(&mut b, 64, 1);
    put64(&mut b, 64 + 16, 0x10000);
    let len = b.len() as u64;
    put64(&mut b, 64 + 32, len);
    put64(&mut b, 64 + 40, len);
    put32(&mut b, 120, 2);
    put64(&mut b, 128, 0x100);
    put64(&mut b, 136, 0x10100);
    put64(&mut b, 152, entries.len() as u64 * 16);
    put64(&mut b, 160, entries.len() as u64 * 16);
    for (i, (tag, value)) in entries.into_iter().enumerate() {
        put64(&mut b, 0x100 + i * 16, tag);
        put64(&mut b, 0x108 + i * 16, value);
    }
    b[0x400..].copy_from_slice(&strings);
    b
}

#[test]
fn accepts_only_known_runtime_names_in_order() {
    let names = [
        "libgcc_s.so.1",
        "libc.so.6",
        "libm.so.6",
        "libpthread.so.0",
        "libdl.so.2",
        "librt.so.1",
        "ld-linux-x86-64.so.2",
    ];
    assert_eq!(validate(&fixture(&names, &[])).unwrap(), names);
    assert!(validate(&fixture(&[], &[])).unwrap().is_empty());
    for name in ["evil.so", "./libc.so.6", "/lib/libc.so.6", "", "libc.so"] {
        assert!(validate(&fixture(&[name], &[])).is_err(), "{name}");
    }
}
#[test]
fn rejects_loader_overrides() {
    for tag in [15, 29, 0x7fff_fffd, 0x7fff_ffff] {
        assert!(validate(&fixture(&["libc.so.6"], &[(tag, 0)])).is_err());
    }
}
#[test]
fn rejects_wrong_headers_and_every_truncation() {
    let original = fixture(&["libc.so.6"], &[]);
    for n in 0..original.len() {
        assert!(validate(&original[..n]).is_err(), "length {n}");
    }
    for (at, value) in [
        (0, 0),
        (4, 1),
        (5, 2),
        (6, 2),
        (16, 2),
        (18, 183),
        (20, 2),
        (52, 63),
        (54, 55),
    ] {
        let mut b = original.clone();
        b[at] = value;
        assert!(validate(&b).is_err(), "offset {at}");
    }
}
#[test]
fn rejects_table_overflow_budgets_and_bad_mappings() {
    for (at, value) in [
        (32, u64::MAX),
        (128, u64::MAX),
        (136, u64::MAX),
        (152, 17),
        (152, 4097 * 16),
        (0x108, u64::MAX),
        (0x128, 1024 * 1024 + 1),
        (0x118, u64::MAX),
        (96, u64::MAX),
        (104, u64::MAX),
    ] {
        let mut b = fixture(&["libc.so.6"], &[]);
        put64(&mut b, at, value);
        assert!(validate(&b).is_err(), "offset {at}, value {value}");
    }
    let mut b = fixture(&[], &[]);
    put16(&mut b, 56, 1025);
    assert!(validate(&b).is_err());
    assert!(validate(&vec![0; 64 * 1024 * 1024 + 1]).is_err());
    assert!(validate(&fixture(&["libc.so.6"; 33], &[])).is_err());
}
#[test]
fn rejects_malformed_dynamic_and_strings() {
    assert!(validate(&fixture(&[], &[(5, 0x10400)])).is_err());
    assert!(validate(&fixture(&[], &[(10, 1)])).is_err());
    let mut b = fixture(&["libc.so.6"], &[]);
    put64(&mut b, 0x130, 12);
    assert!(validate(&b).is_err());
    let mut b = fixture(&["libc.so.6"], &[(0, 0), (1, 1)]);
    assert!(validate(&b).is_err());
    b = fixture(&["libc.so.6"], &[]);
    *b.last_mut().unwrap() = b'x';
    assert!(validate(&b).is_err());
    b = fixture(&["libc.so.6"], &[]);
    b[0x400] = 1;
    assert!(validate(&b).is_err());
    b = fixture(&["libc.so.6"], &[]);
    b[0x401] = 255;
    assert!(validate(&b).is_err());
    assert!(validate(&fixture(&[&"x".repeat(256)], &[])).is_err());
}
