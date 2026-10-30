//! eza's `SortField::value_variants`: `&[Self::Name(SortCase::AaBbCc), ..]` is a borrow
//! that outlives the function, since a tuple variant made of constants is a constant.
//! ra-jit promoted a unit variant and a record literal but not a tuple constructor, built
//! the array in the frame and returned its address, and clap matched on what the next
//! call left there.

#[derive(Debug, Clone, Copy)]
pub enum Case {
    Lower,
    Upper,
}

#[derive(Debug, Clone, Copy)]
pub enum Field {
    None,
    Name(Case),
    Size,
}

#[derive(Debug, Clone, Copy)]
pub struct Pair(u8, Case);

#[inline(never)]
fn fields<'a>() -> &'a [Field] {
    &[Field::None, Field::Name(Case::Lower), Field::Name(Case::Upper), Field::Size]
}

#[inline(never)]
fn pair<'a>() -> &'a Pair {
    &Pair(7, Case::Upper)
}

#[inline(never)]
fn overwritten() -> u64 {
    std::hint::black_box([u64::MAX; 64]).iter().fold(0, |sum, it| sum ^ it)
}

pub fn run() -> String {
    let (fields, pair) = (fields(), pair());
    let sum = overwritten();
    format!("{fields:?} {pair:?} {sum}")
}
