//! Found by the fuzzer (`fuzz/found/differs-checksum-L1-45-c17.rs`), under the LLVM backend
//! with its passes on: an array of rows of `i128` where several rows repeat one value. LLVM
//! 20.1's SLP vectorizer wrote the last rows with one wide store that left three elements
//! out, and they read back as whatever was on the stack.

#[inline(never)]
fn sum(rows: &[[i128; 5]; 5]) -> i128 {
    rows.iter().flatten().sum()
}

pub fn run() -> i128 {
    let rows: [[i128; 5]; 5] = [[2; 5], [12, 2, 3, 14, 6], [26, 5, 7, 9, 11], [13; 5], [17; 5]];
    sum(&rows)
}
