//! zerotrie assigns a tuple from a `match` with an arm that breaks out of the loop:
//! `(next, rest) = match rest.split_first() { Some(x) => x, None => break }`.

macro_rules! some_or {
    ($expr:expr, $otherwise:expr) => {
        match $expr {
            Some(x) => x,
            None => $otherwise,
        }
    };
}

fn sum(bytes: &[u8]) -> (usize, usize) {
    let mut rest = bytes;
    let mut total = 0;
    loop {
        let next;
        (next, rest) = some_or!(rest.split_first(), break);
        total += *next as usize;
    }
    (total, rest.len())
}

fn first_pair(bytes: &[u8]) -> Option<(u8, u8)> {
    let (a, b);
    [a, b] = some_or!(bytes.first_chunk::<2>().copied(), return None);
    Some((a, b))
}

fn coerced<'a>(array: &'a [u8; 3], other: &'a mut [u8; 2]) -> (usize, usize) {
    let (slice, mut_slice): (&[u8], &mut [u8]);
    (slice, mut_slice) = (array, other);
    mut_slice[0] = 9;
    (slice.len(), mut_slice.len())
}

pub fn run() -> ((usize, usize), Option<(u8, u8)>, Option<(u8, u8)>, (usize, usize)) {
    (sum(&[1, 2, 3, 250]), first_pair(&[7, 8, 9]), first_pair(&[7]), coerced(&[0; 3], &mut [0; 2]))
}
