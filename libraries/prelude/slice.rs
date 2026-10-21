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

/// `[T]` methods that need no element bound. A prelude trait, like
/// `StrMethods`, until inherent `impl<T> [T]` blocks are supported.
pub trait SliceMethods {
    fn is_empty(&self) -> bool;
    fn swap(&mut self, a: usize, b: usize);
}

impl<T> SliceMethods for [T] {
    fn is_empty(&self) -> bool {
        self.len() == 0usize
    }
    /// Both indices are bounds-checked (stock's index panic) before any write.
    fn swap(&mut self, a: usize, b: usize) {
        let pa = &mut self[a] as *mut T;
        let pb = &mut self[b] as *mut T;
        unsafe {
            let value = *pa;
            *pa = *pb;
            *pb = value;
        }
    }
}

/// Element-returning `[T]` methods. Implemented for `&'a [T]` so the trait
/// carries the element without a `[T]` trait-argument boundary.
pub trait SliceRefMethods<'a, T> {
    fn first(self) -> super::option::Option<&'a T>;
    fn last(self) -> super::option::Option<&'a T>;
}

impl<'a, T> SliceRefMethods<'a, T> for &'a [T] {
    fn first(self) -> super::option::Option<&'a T> {
        if self.len() == 0usize {
            super::option::Option::None
        } else {
            super::option::Option::Some(&self[0usize])
        }
    }
    fn last(self) -> super::option::Option<&'a T> {
        let len = self.len();
        if len == 0usize {
            super::option::Option::None
        } else {
            super::option::Option::Some(&self[len - 1usize])
        }
    }
}

/// `[T]::contains` (element-wise `PartialEq`, front to back).
pub trait SliceContains<'a, T> {
    fn contains(self, value: &T) -> bool;
}

impl<'a, T: super::cmp::PartialEq> SliceContains<'a, T> for &'a [T] {
    fn contains(self, value: &T) -> bool {
        let mut i = 0usize;
        while i < self.len() {
            if self[i] == *value {
                return true;
            }
            i += 1usize;
        }
        false
    }
}

/// `[T]::to_vec`: clones each element once, in index order.
pub trait SliceToVec<'a, T> {
    fn to_vec(self) -> super::vec::Vec<T>;
}

impl<'a, T: Clone> SliceToVec<'a, T> for &'a [T] {
    fn to_vec(self) -> super::vec::Vec<T> {
        let mut out: super::vec::Vec<T> = super::vec::Vec::with_capacity(self.len());
        let mut i = 0usize;
        while i < self.len() {
            let element = &self[i];
            out.push(element.clone());
            i += 1usize;
        }
        out
    }
}
