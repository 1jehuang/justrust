//! Raw-parts slice constructors. The caller must satisfy Rust's allocation,
//! alignment, initialization, aliasing and lifetime requirements, including
//! a non-null aligned pointer for empty slices. These functions do not read T.

/// Shared iteration retains the owner's borrow, not an unchecked pointer loan.
pub struct Iter<'a, T> {
    values: &'a [T],
    len: usize,
    index: usize,
}

impl<'a, T> Iter<'a, T> {
    /// Retain the typed slice so element references keep their original
    /// carrier provenance, including when T itself contains a reference.
    pub(super) fn new(values: &'a [T]) -> Iter<'a, T> {
        let len = values.len();
        Iter { values, len, index: 0usize }
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
        if self.index < self.len {
            let index = self.index;
            self.index += 1usize;
            super::option::Option::Some(&self.values[index])
        } else {
            super::option::Option::None
        }
    }
}

impl<'a, T> super::iter::DoubleEndedIterator for Iter<'a, T> {
    fn next_back(&mut self) -> super::option::Option<&'a T> {
        if self.index < self.len {
            self.len -= 1usize;
            super::option::Option::Some(&self.values[self.len])
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
    type Element;
    fn as_mut_ptr(&mut self) -> *mut Self::Element;
    fn is_empty(&self) -> bool;
    fn swap(&mut self, a: usize, b: usize);
}

impl<T> SliceMethods for [T] {
    type Element = T;
    fn as_mut_ptr(&mut self) -> *mut T {
        __justrust_slice_as_mut_ptr!(self)
    }

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

/// Mutable element-returning methods retain the receiver's exact lifetime
/// without an associated-type projection in the returned slice carriers.
pub trait SliceMutRefMethods<'a, T> {
    fn split_at_mut(self, mid: usize) -> (&'a mut [T], &'a mut [T]);
}

impl<'a, T> SliceMutRefMethods<'a, T> for &'a mut [T] {
    fn split_at_mut(self, mid: usize) -> (&'a mut [T], &'a mut [T]) {
        let len = self.len();
        assert!(mid <= len);
        let data = __justrust_slice_as_mut_ptr!(self);
        // The checked midpoint partitions one allocation into adjacent,
        // nonoverlapping ranges. Both returned loans retain the receiver's
        // lifetime, including when either range is empty.
        unsafe {
            let right = <*mut T as super::ptr::MutPtr<T>>::add(data, mid);
            (from_raw_parts_mut(data, mid), from_raw_parts_mut(right, len - mid))
        }
    }
}

/// Element-returning `[T]` methods. Implemented for `&'a [T]` so the trait
/// carries the element without a `[T]` trait-argument boundary.
pub trait SliceRefMethods<'a, T> {
    fn get(self, index: usize) -> super::option::Option<&'a T>;
    fn first(self) -> super::option::Option<&'a T>;
    fn last(self) -> super::option::Option<&'a T>;
    fn split_at(self, mid: usize) -> (&'a [T], &'a [T]);
}

impl<'a, T> SliceRefMethods<'a, T> for &'a [T] {
    fn get(self, index: usize) -> super::option::Option<&'a T> {
        if index < self.len() {
            super::option::Option::Some(&self[index])
        } else {
            super::option::Option::None
        }
    }

    fn split_at(self, mid: usize) -> (&'a [T], &'a [T]) {
        assert!(mid <= self.len());
        (&self[..mid], &self[mid..])
    }

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

/// Non-overlapping borrowed chunks, with the final partial chunk retained.
pub struct Chunks<'a, T> {
    values: &'a [T],
    size: usize,
    front: usize,
    back: usize,
}

impl<'a, T> Chunks<'a, T> {
    pub(super) fn new(values: &'a [T], size: usize) -> Chunks<'a, T> {
        assert!(size != 0usize);
        Chunks { values, size, front: 0usize, back: values.len() }
    }
}

impl<'a, T> super::iter::Iterator for Chunks<'a, T> {
    type Item = &'a [T];
    fn next(&mut self) -> super::option::Option<&'a [T]> {
        if self.front == self.back { return super::option::Option::None; }
        let start = self.front;
        let remaining = self.back - start;
        let amount = if self.size < remaining { self.size } else { remaining };
        self.front += amount;
        super::option::Option::Some(&self.values[start..self.front])
    }
}

impl<'a, T> super::iter::DoubleEndedIterator for Chunks<'a, T> {
    fn next_back(&mut self) -> super::option::Option<&'a [T]> {
        if self.front == self.back { return super::option::Option::None; }
        let end = self.back;
        let remainder = (end - self.front) % self.size;
        let amount = if remainder == 0usize { self.size } else { remainder };
        self.back -= amount;
        super::option::Option::Some(&self.values[self.back..end])
    }
}

impl<'a, T> super::iter::IntoIterator for Chunks<'a, T> {
    type Item = &'a [T];
    type IntoIter = Self;
    fn into_iter(self) -> Self { self }
}

pub trait SliceChunks<'a, T> {
    fn chunks(self, size: usize) -> Chunks<'a, T>;
    fn chunks_exact(self, size: usize) -> ChunksExact<'a, T>;
}

impl<'a, T> SliceChunks<'a, T> for &'a [T] {
    fn chunks(self, size: usize) -> Chunks<'a, T> { Chunks::new(self, size) }
    fn chunks_exact(self, size: usize) -> ChunksExact<'a, T> { ChunksExact::new(self, size) }
}

/// Non-overlapping chunks of exactly `size` elements. The final
/// `len % size` elements are never yielded; `remainder` returns them.
pub struct ChunksExact<'a, T> {
    values: &'a [T],
    size: usize,
    front: usize,
    back: usize,
}

impl<'a, T> ChunksExact<'a, T> {
    pub(super) fn new(values: &'a [T], size: usize) -> ChunksExact<'a, T> {
        assert!(size != 0usize);
        let exact = values.len() - values.len() % size;
        ChunksExact { values, size, front: 0usize, back: exact }
    }
    pub fn remainder(&self) -> &'a [T] {
        let exact = self.values.len() - self.values.len() % self.size;
        &self.values[exact..]
    }
}

impl<'a, T> super::iter::Iterator for ChunksExact<'a, T> {
    type Item = &'a [T];
    fn next(&mut self) -> super::option::Option<&'a [T]> {
        if self.back - self.front < self.size { return super::option::Option::None; }
        let start = self.front;
        self.front += self.size;
        super::option::Option::Some(&self.values[start..self.front])
    }
}

impl<'a, T> super::iter::DoubleEndedIterator for ChunksExact<'a, T> {
    fn next_back(&mut self) -> super::option::Option<&'a [T]> {
        if self.back - self.front < self.size { return super::option::Option::None; }
        let end = self.back;
        self.back -= self.size;
        super::option::Option::Some(&self.values[self.back..end])
    }
}

impl<'a, T> super::iter::IntoIterator for ChunksExact<'a, T> {
    type Item = &'a [T];
    type IntoIter = Self;
    fn into_iter(self) -> Self { self }
}

/// `[T]::iter`: a shared element iterator over the borrowed slice.
pub trait SliceIter<'a, T> {
    fn iter(self) -> Iter<'a, T>;
}

impl<'a, T> SliceIter<'a, T> for &'a [T] {
    fn iter(self) -> Iter<'a, T> {
        Iter::new(self)
    }
}
