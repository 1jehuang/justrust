//! What only the 2015 edition reads. A method of a trait may leave its parameters unnamed
//! (`locale`'s `LanginfoItem::decode` does), and `...` is a range pattern.
//! `cratetest.py harness/breakers/edition2015` runs it.

pub trait Decode: Copy {
    type Out;
    unsafe fn decode(&self, *const u8, Option<&u8>, count: usize) -> Self::Out;
    fn scale(&self, u32, by: u32) -> u32;
}

#[derive(Clone, Copy)]
pub struct Sum;

impl Decode for Sum {
    type Out = u32;
    unsafe fn decode(&self, bytes: *const u8, more: Option<&u8>, mut count: usize) -> u32 {
        let mut sum = more.map_or(0, |it| u32::from(*it));
        while count > 0 {
            count -= 1;
            sum += u32::from(*bytes.offset(count as isize));
        }
        sum
    }
    fn scale(&self, value: u32, by: u32) -> u32 {
        value * by
    }
}

pub fn decoded<D: Decode>(with: D, bytes: &[u8], more: Option<&u8>) -> D::Out {
    unsafe { with.decode(bytes.as_ptr(), more, bytes.len()) }
}

pub fn class(byte: u8) -> u8 {
    match byte {
        b'0'...b'9' => 1,
        b'a'...b'z' => 2,
        _ => 0,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unnamed_parameters() {
        assert_eq!(decoded(Sum, &[1, 2, 3], Some(&10)), 16);
        assert_eq!(decoded(Sum, &[], None), 0);
        assert_eq!(Sum.scale(6, 7), 42);
    }

    #[test]
    fn three_dot_range() {
        assert_eq!([class(b'5'), class(b'q'), class(b'Q'), class(b'9'), class(b':')], [1, 2, 0, 1, 0]);
    }
}
