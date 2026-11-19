use super::*;
fn fixture(k: usize) -> Vec<u8> {
    let n = 2048 * row(k);
    let skip = if k == 1 { 12 } else { 0 };
    let at = 1024;
    let mut b = vec![0; at + skip + n];
    b[..7].copy_from_slice(b"\x7fELF\x02\x01\x01");
    b[40..48].copy_from_slice(&64u64.to_le_bytes());
    b[58..64].copy_from_slice(&[64, 0, 3, 0, 1, 0]);
    b[132] = 3;
    b[152..160].copy_from_slice(&384u64.to_le_bytes());
    b[160] = 32;
    b[385..385 + NAMES[k].len()].copy_from_slice(NAMES[k]);
    b[192] = 1;
    b[196] = [4, 1, 2][k];
    b[216..224].copy_from_slice(&(at as u64).to_le_bytes());
    b[224..232].copy_from_slice(&((skip + n) as u64).to_le_bytes());
    b[248] = 24;
    if k == 1 {
        b[at..at + 4].copy_from_slice(&[1, 0x1b, 3, 0x3b]);
        b[at + 8..at + 12].copy_from_slice(&2048u32.to_le_bytes());
    }
    let mut seed = 1u32;
    for v in &mut b[at + skip..] {
        seed = seed.wrapping_mul(1664525).wrapping_add(1013904223);
        *v = (seed >> 24) as u8;
    }
    b
}
#[test]
fn one_byte_and_mode_only_do_not_allocate_row_index() {
    for k in 0..3 {
        let old = fixture(k);
        let mut data = old[BLOCK..2 * BLOCK].to_vec();
        let mut r = Rows::detect(&old, &mut Cursor::new(&old), old.len() as u64);
        // Mode bits live in the outer artifact protocol. A mode-only edit
        // presents the identical byte stream to the sender.
        assert!(r.old[k].is_some());
        for flip in [0, 1] {
            data[0] ^= flip;
            assert_eq!(r.choose(&old, BLOCK, &data, BLOCK as u32), BLOCK as u32);
            assert_eq!(r.table.capacity(), 0);
        }
    }
}
#[test]
fn weak_prediction_changes_source_then_releases_index() {
    for k in 0..3 {
        let old = fixture(k);
        let mut r = Rows::detect(&old, &mut Cursor::new(&old), old.len() as u64);
        let at = BLOCK - row(k) * 4;
        let mut data = old[at..at + BLOCK].to_vec();
        data[1] ^= 9;
        assert_eq!(r.choose(&old, BLOCK, &data, NONE), at as u32);
        assert!(!r.table.is_empty());
        assert_eq!(r.choose(&old, old.len(), &data, NONE), NONE);
        assert_eq!(r.table.capacity(), 0);
    }
}
#[test]
fn malformed_sections_fall_back() {
    let old = fixture(0);
    for (at, byte) in [(5, 2), (47, 255), (61, 255), (62, 99), (231, 255), (248, 0)] {
        let mut bad = old.clone();
        bad[at] = byte;
        let mut r = Rows::detect(&old, &mut Cursor::new(&bad), bad.len() as u64);
        assert_eq!(r.choose(&old, BLOCK, &old[BLOCK..2 * BLOCK], NONE), NONE);
        assert_eq!(r.table.capacity(), 0);
    }
}
