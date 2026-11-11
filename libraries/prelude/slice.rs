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

pub trait SliceSort<T> {
    fn sort_by<F: FnMut(&T, &T) -> super::cmp::Ordering>(&mut self, compare: F);
    fn sort_unstable_by<F: FnMut(&T, &T) -> super::cmp::Ordering>(&mut self, compare: F);
    fn sort_by_key<K: super::cmp::Ord, F: FnMut(&T) -> K>(&mut self, key: F);
    fn sort_unstable_by_key<K: super::cmp::Ord, F: FnMut(&T) -> K>(&mut self, key: F);
}

impl<T> SliceSort<T> for [T] {
    fn sort_by<F: FnMut(&T, &T) -> super::cmp::Ordering>(&mut self, mut compare: F) {
        let mut i = 1usize;
        while i < self.len() {
            let mut j = i;
            while j > 0usize {
                if !compare(&self[j - 1usize], &self[j]).is_gt() { break; }
                self.swap(j - 1usize, j);
                j -= 1usize;
            }
            i += 1usize;
        }
    }

    fn sort_unstable_by<F: FnMut(&T, &T) -> super::cmp::Ordering>(&mut self, compare: F) {
        self.sort_by(compare);
    }

    fn sort_by_key<K: super::cmp::Ord, F: FnMut(&T) -> K>(&mut self, mut key: F) {
        self.sort_by(|a, b| key(a).cmp(&key(b)));
    }

    fn sort_unstable_by_key<K: super::cmp::Ord, F: FnMut(&T) -> K>(&mut self, key: F) {
        self.sort_by_key(key);
    }
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
    fn split_first_mut(self) -> super::option::Option<(&'a mut T, &'a mut [T])>;
    fn split_last_mut(self) -> super::option::Option<(&'a mut T, &'a mut [T])>;
    fn first_mut(self) -> super::option::Option<&'a mut T>;
    fn last_mut(self) -> super::option::Option<&'a mut T>;
}

impl<'a, T> SliceMutRefMethods<'a, T> for &'a mut [T] {
    fn split_at_mut(self, mid: usize) -> (&'a mut [T], &'a mut [T]) {
        let len = self.len();
        assert!(mid <= len);
        let data = __justrust_slice_as_mut_ptr!(self);
        // The checked midpoint partitions one allocation into adjacent,
        // nonoverlapping ranges, addressed by the same physical element
        // stride as slice indexing (any element type, including `Vec<T>`). Both returned loans retain the receiver's
        // lifetime, including when either range is empty.
        unsafe {
            let right = iter_mut_slot(data, mid);
            (from_raw_parts_mut(data, mid), from_raw_parts_mut(right, len - mid))
        }
    }
    fn split_first_mut(self) -> super::option::Option<(&'a mut T, &'a mut [T])> {
        let len = self.len();
        if len == 0usize {
            return super::option::Option::None;
        }
        let data = __justrust_slice_as_mut_ptr!(self);
        unsafe {
            let rest = iter_mut_slot(data, 1usize);
            super::option::Option::Some((&mut *data, from_raw_parts_mut(rest, len - 1usize)))
        }
    }
    fn split_last_mut(self) -> super::option::Option<(&'a mut T, &'a mut [T])> {
        let len = self.len();
        if len == 0usize {
            return super::option::Option::None;
        }
        let data = __justrust_slice_as_mut_ptr!(self);
        unsafe {
            let last = iter_mut_slot(data, len - 1usize);
            super::option::Option::Some((&mut *last, from_raw_parts_mut(data, len - 1usize)))
        }
    }
    fn first_mut(self) -> super::option::Option<&'a mut T> {
        if self.len() == 0usize {
            return super::option::Option::None;
        }
        let data = __justrust_slice_as_mut_ptr!(self);
        unsafe { super::option::Option::Some(&mut *data) }
    }
    fn last_mut(self) -> super::option::Option<&'a mut T> {
        let len = self.len();
        if len == 0usize {
            return super::option::Option::None;
        }
        let data = __justrust_slice_as_mut_ptr!(self);
        unsafe { super::option::Option::Some(&mut *iter_mut_slot(data, len - 1usize)) }
    }
}

/// Element-returning `[T]` methods. Implemented for `&'a [T]` so the trait
/// carries the element without a `[T]` trait-argument boundary.
pub trait SliceRefMethods<'a, T> {
    fn get(self, index: usize) -> super::option::Option<&'a T>;
    fn first(self) -> super::option::Option<&'a T>;
    fn last(self) -> super::option::Option<&'a T>;
    fn split_at(self, mid: usize) -> (&'a [T], &'a [T]);
    fn split_first(self) -> super::option::Option<(&'a T, &'a [T])>;
    fn split_last(self) -> super::option::Option<(&'a T, &'a [T])>;
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

    fn split_first(self) -> super::option::Option<(&'a T, &'a [T])> {
        if self.len() == 0usize {
            super::option::Option::None
        } else {
            super::option::Option::Some((&self[0usize], &self[1usize..]))
        }
    }
    fn split_last(self) -> super::option::Option<(&'a T, &'a [T])> {
        let len = self.len();
        if len == 0usize {
            super::option::Option::None
        } else {
            super::option::Option::Some((&self[len - 1usize], &self[..len - 1usize]))
        }
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

pub trait SliceChunks<'a, T> {
    fn chunks(self, size: usize) -> Chunks<'a, T>;
    fn chunks_exact(self, size: usize) -> ChunksExact<'a, T>;
    fn windows(self, size: usize) -> Windows<'a, T>;
}

impl<'a, T> SliceChunks<'a, T> for &'a [T] {
    fn chunks(self, size: usize) -> Chunks<'a, T> { Chunks::new(self, size) }
    fn chunks_exact(self, size: usize) -> ChunksExact<'a, T> { ChunksExact::new(self, size) }
    fn windows(self, size: usize) -> Windows<'a, T> { Windows::new(self, size) }
}

/// Overlapping borrowed windows of exactly `size` elements.
pub struct Windows<'a, T> {
    values: &'a [T],
    size: usize,
    front: usize,
    back: usize,
}

impl<'a, T> Windows<'a, T> {
    pub(super) fn new(values: &'a [T], size: usize) -> Windows<'a, T> {
        assert!(size != 0usize);
        let back = if values.len() < size { 0usize } else { values.len() - size + 1usize };
        Windows { values, size, front: 0usize, back }
    }
}

impl<'a, T> super::iter::Iterator for Windows<'a, T> {
    type Item = &'a [T];
    fn next(&mut self) -> super::option::Option<&'a [T]> {
        if self.front == self.back { return super::option::Option::None; }
        let start = self.front;
        self.front += 1usize;
        super::option::Option::Some(&self.values[start..start + self.size])
    }
}

impl<'a, T> super::iter::DoubleEndedIterator for Windows<'a, T> {
    fn next_back(&mut self) -> super::option::Option<&'a [T]> {
        if self.front == self.back { return super::option::Option::None; }
        self.back -= 1usize;
        super::option::Option::Some(&self.values[self.back..self.back + self.size])
    }
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

/// `[T]::iter`: a shared element iterator over the borrowed slice.
pub trait SliceIter<'a, T> {
    fn iter(self) -> Iter<'a, T>;
}

/// Exclusive iteration: each element is handed out exactly once, so the
/// returned `&'a mut T` loans never overlap. `marker` retains the owner's
/// exclusive borrow for `'a`: the slice stays borrowed while the iterator or
/// any yielded element is live.
pub struct IterMut<'a, T> {
    data: *mut T,
    len: usize,
    index: usize,
    marker: &'a [T],
}

/// Element address by physical storage stride, as `Vec` computes its slots.
fn iter_mut_slot<T>(base: *mut T, index: usize) -> *mut T {
    (base as usize + index * super::mem::element_stride::<T>()) as *mut T
}

impl<'a, T> IterMut<'a, T> {
    pub(super) fn new(values: &'a mut [T]) -> IterMut<'a, T> {
        let len = values.len();
        // The pointer primitive consumes the exclusive descriptor. The shared
        // view is rebuilt from it, as `Vec::as_slice` does, and the declared
        // `'a` keeps the caller's exclusive borrow live.
        let data = __justrust_slice_as_mut_ptr!(values);
        let marker = unsafe { from_raw_parts::<T>(data as *const T, len) };
        IterMut { data, len, index: 0usize, marker }
    }

    /// An owner that already holds its buffer pointer and length (`Vec`)
    /// supplies them with the shared view that retains its exclusive borrow.
    pub(super) fn from_parts(data: *mut T, len: usize, marker: &'a [T]) -> IterMut<'a, T> {
        IterMut { data, len, index: 0usize, marker }
    }
}

impl<'a, T> super::iter::Iterator for IterMut<'a, T> {
    type Item = &'a mut T;
    fn next(&mut self) -> super::option::Option<&'a mut T> {
        if self.index < self.len {
            let index = self.index;
            self.index += 1usize;
            unsafe {
                let element = iter_mut_slot(self.data, index);
                super::option::Option::Some(&mut *element)
            }
        } else {
            super::option::Option::None
        }
    }
}

impl<'a, T> super::iter::DoubleEndedIterator for IterMut<'a, T> {
    fn next_back(&mut self) -> super::option::Option<&'a mut T> {
        if self.index < self.len {
            self.len -= 1usize;
            let index = self.len;
            unsafe {
                let element = iter_mut_slot(self.data, index);
                super::option::Option::Some(&mut *element)
            }
        } else {
            super::option::Option::None
        }
    }
}

/// `[T]::iter_mut`.
pub trait SliceIterMut<T> {
    fn iter_mut<'a>(&'a mut self) -> IterMut<'a, T>;
}

impl<T> SliceIterMut<T> for [T] {
    fn iter_mut<'a>(&'a mut self) -> IterMut<'a, T> {
        IterMut::new(self)
    }
}

impl<'a, T> super::iter::IntoIterator for &'a mut [T] {
    type Item = &'a mut T;
    type IntoIter = IterMut<'a, T>;
    fn into_iter(self) -> IterMut<'a, T> {
        IterMut::new(self)
    }
}

impl<'a, T> SliceIter<'a, T> for &'a [T] {
    fn iter(self) -> Iter<'a, T> {
        Iter::new(self)
    }
}


/// Element-wise slice equality. Arrays of equal length compare through it.
impl<T: super::cmp::PartialEq<U>, U> super::cmp::PartialEq<[U]> for [T] {
    fn eq(&self, other: &[U]) -> bool {
        if self.len() != other.len() {
            return false;
        }
        let mut i = 0usize;
        while i < self.len() {
            if <T as super::cmp::PartialEq<U>>::ne(&self[i], &other[i]) {
                return false;
            }
            i += 1usize;
        }
        true
    }

    fn ne(&self, other: &[U]) -> bool {
        if self.len() != other.len() {
            return true;
        }
        let mut i = 0usize;
        while i < self.len() {
            if <T as super::cmp::PartialEq<U>>::ne(&self[i], &other[i]) {
                return true;
            }
            i += 1usize;
        }
        false
    }
}
