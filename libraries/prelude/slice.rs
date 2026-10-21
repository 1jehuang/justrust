//! Raw-parts slice constructors. The caller must satisfy Rust's allocation,
//! alignment, initialization, aliasing and lifetime requirements, including
//! a non-null aligned pointer for empty slices. These functions do not read T.

/// Shared iteration retains the owner's borrow, not an unchecked pointer loan.
pub struct Iter<'a, T> {
    values: &'a super::vec::Vec<T>,
    index: usize,
}

impl<'a, T> Iter<'a, T> {
    pub(super) fn new(values: &'a super::vec::Vec<T>) -> Iter<'a, T> {
        Iter { values, index: 0usize }
    }
}

impl<'a, T> super::iter::IntoIterator for Iter<'a, T> {
    type Item = &'a T;
    type IntoIter = Iter<'a, T>;
    fn into_iter(self) -> Iter<'a, T> {
        self
    }
}

impl<'a, T> super::iter::Iterator for Iter<'a, T> {
    type Item = &'a T;
    fn next(&mut self) -> super::option::Option<&'a T> {
        if self.index < self.values.len() {
            let index = self.index;
            self.index += 1usize;
            super::option::Option::Some(self.values.get_ref(index))
        } else {
            super::option::Option::None
        }
    }
}

pub unsafe fn from_raw_parts<'a, T>(data: *const T, len: usize) -> &'a [T] {
    unsafe { __justrust_slice_from_raw_parts!(data, len) }
}

pub unsafe fn from_raw_parts_mut<'a, T>(data: *mut T, len: usize) -> &'a mut [T] {
    unsafe { __justrust_slice_from_raw_parts_mut!(data, len) }
}
