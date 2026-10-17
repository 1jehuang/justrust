// Exact-source positive corpus for the bounded shared-byte-slice contract.
// Expected output is checked by the external stock/native CLI harness.

fn byte_at(bytes: &[u8], index: usize) -> u8 {
    bytes[index]
}

fn forward_once(bytes: &[u8], index: usize) -> u8 {
    let copied: &[u8] = bytes;
    byte_at(copied, index)
}

fn forward_nested(bytes: &[u8], index: usize) -> u8 {
    let copied: &[u8] = bytes;
    forward_once(copied, index)
}

fn pair_lengths(first: &[u8], second: &[u8], first_len: usize, second_len: usize) -> bool {
    let copied_first: &[u8] = first;
    let copied_second: &[u8] = second;
    first.len() == first_len
        && second.len() == second_len
        && copied_first.len() == first_len
        && copied_second.len() == second_len
}

fn main() {
    let short: [u8; 3] = [11, 29, 47];
    let long: [u8; 5] = [101, 103, 107, 109, 113];
    let empty: [u8; 0] = [];
    let singleton: [u8; 1] = [251];

    let short_slice: &[u8] = &short;
    let long_slice: &[u8] = &long;
    let empty_slice: &[u8] = &empty;
    let singleton_slice: &[u8] = &singleton;
    let short_copy: &[u8] = short_slice;
    let long_copy: &[u8] = long_slice;
    let empty_copy: &[u8] = empty_slice;
    let singleton_copy: &[u8] = singleton_slice;

    let first: usize = 0;
    let short_last: usize = 2;
    let long_last: usize = 4;
    let short_first_byte: u8 = forward_nested(short_slice, first);
    let short_last_byte: u8 = forward_nested(short_copy, short_last);
    let long_first_byte: u8 = forward_nested(long_copy, first);
    let long_last_byte: u8 = forward_nested(long_slice, long_last);
    let singleton_byte: u8 = forward_nested(singleton_copy, first);

    let different_lengths: bool = pair_lengths(short_slice, long_copy, 3, 5);
    let reversed_lengths: bool = pair_lengths(long_slice, short_copy, 5, 3);
    let boundary_lengths: bool = pair_lengths(empty_slice, singleton_copy, 0, 1);
    let reversed_boundary_lengths: bool = pair_lengths(singleton_slice, empty_copy, 1, 0);

    if short_first_byte == 11
        && short_last_byte == 47
        && long_first_byte == 101
        && long_last_byte == 113
        && singleton_byte == 251
        && short_copy.len() == 3
        && long_copy.len() == 5
        && empty_copy.len() == 0
        && singleton_copy.len() == 1
        && different_lengths
        && reversed_lengths
        && boundary_lengths
        && reversed_boundary_lengths
    {
        println!("slice-contract-runtime:ok");
    } else {
        println!("slice-contract-runtime:mismatch");
    }
}
