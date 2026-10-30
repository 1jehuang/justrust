// Native Vec<T>: a growable heap buffer written in ordinary Rust source.
use super::mem::{element_stride, raw_realloc};
use super::option::Option;

impl<A> super::iter::FromIterator<A> for Vec<A> {
    fn from_iter<I: super::iter::IntoIterator<Item = A>>(source: I) -> Vec<A> {
        let mut iterator = source.into_iter();
        let mut values = Vec::new();
        loop {
            match iterator.next() {
                Option::Some(value) => values.push(value),
                Option::None => return values,
            }
        }
    }
}

pub struct Vec<T> {
    ptr: *mut T,
    len: usize,
    cap: usize,
}

fn slot<T>(base: *mut T, index: usize) -> *mut T {
    (base as usize + index * element_stride::<T>()) as *mut T
}

impl<T> Vec<T> {
    pub fn new() -> Vec<T> {
        Vec {
            ptr: 0usize as *mut T,
            len: 0usize,
            cap: 0usize,
        }
    }

    pub fn with_capacity(capacity: usize) -> Vec<T> {
        let mut v = Vec { ptr: 0usize as *mut T, len: 0usize, cap: 0usize };
        v.reserve(capacity);
        v
    }

    pub fn len(&self) -> usize {
        self.len
    }

    pub fn capacity(&self) -> usize {
        self.cap
    }

    pub fn is_empty(&self) -> bool {
        self.len == 0usize
    }

    pub fn chunks<'a>(&'a self, size: usize) -> super::slice::Chunks<'a, T> {
        super::slice::Chunks::new(self.as_slice(), size)
    }

    pub fn iter<'a>(&'a self) -> super::slice::Iter<'a, T> {
        super::slice::Iter::new(self.as_slice())
    }

    /// Borrow element `index` (panics when out of bounds).
    pub fn get_ref(&self, index: usize) -> &T {
        if index >= self.len {
            panic!("index out of bounds");
        }
        let p = slot(self.ptr, index) as *const T;
        unsafe { &*p }
    }

    pub fn get(&self, index: usize) -> Option<&T> {
        if index < self.len {
            Option::Some(self.get_ref(index))
        } else {
            Option::None
        }
    }

    pub fn get_mut(&mut self, index: usize) -> Option<&mut T> {
        if index < self.len {
            let p = slot(self.ptr, index);
            Option::Some(unsafe { &mut *p })
        } else {
            Option::None
        }
    }

    pub fn first(&self) -> Option<&T> {
        self.get(0usize)
    }

    pub fn last(&self) -> Option<&T> {
        if self.len == 0usize {
            Option::None
        } else {
            self.get(self.len - 1usize)
        }
    }

    pub fn first_mut(&mut self) -> Option<&mut T> {
        self.get_mut(0usize)
    }

    pub fn last_mut(&mut self) -> Option<&mut T> {
        if self.len == 0usize {
            Option::None
        } else {
            let last = self.len - 1usize;
            self.get_mut(last)
        }
    }

    pub fn as_ptr(&self) -> *const T {
        if self.ptr as usize == 0usize {
            super::mem::storage_alignment::<T>() as *const T
        } else {
            self.ptr as *const T
        }
    }

    pub fn as_mut_ptr(&mut self) -> *mut T {
        if self.ptr as usize == 0usize {
            super::mem::storage_alignment::<T>() as *mut T
        } else {
            self.ptr
        }
    }

    pub fn as_slice(&self) -> &[T] {
        let pointer = self.as_ptr();
        unsafe { super::slice::from_raw_parts::<T>(pointer, self.len) }
    }

    pub fn as_mut_slice(&mut self) -> &mut [T] {
        let length = self.len;
        let pointer = self.as_mut_ptr();
        unsafe { super::slice::from_raw_parts_mut::<T>(pointer, length) }
    }

    pub fn reserve(&mut self, additional: usize) {
        let needed = self.len + additional;
        if needed <= self.cap {
            return;
        }
        let mut cap = if self.cap == 0usize { 4usize } else { self.cap * 2usize };
        if cap < needed {
            cap = needed;
        }
        let size = element_stride::<T>();
        let bytes = cap * size;
        let old_bytes = if self.cap == 0usize { 0usize } else if size == 0usize { 8usize } else { self.cap * size };
        let fresh = raw_realloc(self.ptr as usize, old_bytes, if bytes == 0usize { 8usize } else { bytes }) as *mut T;
        if fresh as usize == 0usize {
            panic!("memory allocation failed");
        }
        self.ptr = fresh;
        self.cap = cap;
    }

    pub fn push(&mut self, value: T) {
        if self.len == self.cap {
            self.reserve(1usize);
        }
        let dst = slot(self.ptr, self.len);
        unsafe {
            *dst = value;
        }
        self.len += 1usize;
    }

    pub fn pop(&mut self) -> Option<T> {
        if self.len == 0usize {
            return Option::None;
        }
        self.len -= 1usize;
        let src = slot(self.ptr, self.len);
        Option::Some(unsafe { *src })
    }

    pub fn insert(&mut self, index: usize, value: T) {
        if index > self.len {
            panic!("insertion index out of bounds");
        }
        // Append then rotate into place with modeled exchanges, so element
        // moves keep exact carrier proofs.
        self.push(value);
        let mut i = self.len - 1usize;
        while i > index {
            self.swap(i - 1usize, i);
            i -= 1usize;
        }
    }

    pub fn remove(&mut self, index: usize) -> T {
        if index >= self.len {
            panic!("removal index out of bounds");
        }
        // Built from the exchange and pop operations so the element move keeps
        // their exact carrier proofs; no unmodeled raw store remains.
        let mut i = index;
        while i + 1usize < self.len {
            self.swap(i, i + 1usize);
            i += 1usize;
        }
        match self.pop() {
            Option::Some(value) => value,
            Option::None => panic!("removal index out of bounds"),
        }
    }

    pub fn swap_remove(&mut self, index: usize) -> T {
        if index >= self.len {
            panic!("swap_remove index out of bounds");
        }
        let last = self.len - 1usize;
        self.swap(index, last);
        match self.pop() {
            Option::Some(value) => value,
            Option::None => panic!("swap_remove index out of bounds"),
        }
    }

    pub fn truncate(&mut self, len: usize) {
        while self.len > len {
            self.len -= 1usize;
            let src = slot(self.ptr, self.len);
            let _dropped = unsafe { *src };
        }
    }

    pub fn swap(&mut self, a: usize, b: usize) {
        if a >= self.len || b >= self.len {
            panic!("swap index out of bounds");
        }
        if a == b {
            return;
        }
        let pa = slot(self.ptr, a);
        let pb = slot(self.ptr, b);
        unsafe {
            let tmp = *pa;
            *pa = *pb;
            *pb = tmp;
        }
    }

    pub fn reverse(&mut self) {
        let mut i = 0usize;
        while i < self.len / 2usize {
            let j = self.len - 1usize - i;
            self.swap(i, j);
            i += 1usize;
        }
    }

    pub fn append(&mut self, other: &mut Vec<T>) {
        self.reserve(other.len);
        let mut i = 0usize;
        while i < other.len {
            let src = slot(other.ptr, i);
            let dst = slot(self.ptr, self.len);
            unsafe {
                *dst = *src;
            }
            self.len += 1usize;
            i += 1usize;
        }
        other.len = 0usize;
    }

    pub fn clear(&mut self) {
        self.truncate(0usize);
    }
}

impl<'a, T> super::iter::IntoIterator for &'a Vec<T> {
    type Item = &'a T;
    type IntoIter = super::slice::Iter<'a, T>;
    fn into_iter(self) -> super::slice::Iter<'a, T> {
        self.iter()
    }
}

// Elements drop front to back, as stock, then the buffer is released.
/// Owning iteration: moves elements out front to back. Unyielded elements
/// drop with the iterator, then the buffer is released, as in std.
pub struct IntoIter<T> {
    ptr: *mut T,
    cap: usize,
    index: usize,
    len: usize,
}

impl<T> super::iter::Iterator for IntoIter<T> {
    type Item = T;
    fn next(&mut self) -> Option<T> {
        if self.index < self.len {
            let src = slot(self.ptr, self.index);
            self.index += 1usize;
            Option::Some(unsafe { *src })
        } else {
            Option::None
        }
    }
}

impl<T> super::iter::DoubleEndedIterator for IntoIter<T> {
    fn next_back(&mut self) -> Option<T> {
        if self.index < self.len {
            self.len -= 1usize;
            Option::Some(unsafe { *slot(self.ptr, self.len) })
        } else {
            Option::None
        }
    }
}

impl<T> super::iter::IntoIterator for IntoIter<T> {
    type Item = T;
    type IntoIter = IntoIter<T>;
    fn into_iter(self) -> IntoIter<T> {
        self
    }
}

impl<T> Drop for IntoIter<T> {
    fn drop(&mut self) {
        while self.index < self.len {
            let _dropped = unsafe { *slot(self.ptr, self.index) };
            self.index += 1usize;
        }
        if self.cap != 0usize {
            let size = element_stride::<T>();
            let bytes = if size == 0usize { 8usize } else { self.cap * size };
            super::mem::raw_free(self.ptr as usize, bytes);
        }
    }
}

impl<T> super::iter::IntoIterator for Vec<T> {
    type Item = T;
    type IntoIter = IntoIter<T>;
    fn into_iter(mut self) -> IntoIter<T> {
        let iter = IntoIter { ptr: self.ptr, cap: self.cap, index: 0usize, len: self.len };
        // Ownership of the buffer and elements moves to the iterator.
        self.len = 0usize;
        self.cap = 0usize;
        iter
    }
}

// Elements drop front to back, as stock, then the buffer is released.
impl<T> Drop for Vec<T> {
    fn drop(&mut self) {
        let mut i = 0usize;
        while i < self.len {
            let _dropped = unsafe { *slot(self.ptr, i) };
            i += 1usize;
        }
        self.len = 0usize;
        if self.cap != 0usize {
            let size = element_stride::<T>();
            let bytes = if size == 0usize { 8usize } else { self.cap * size };
            super::mem::raw_free(self.ptr as usize, bytes);
        }
    }
}

/// `vec![elem; n]` expansion target.
pub fn from_elem<T: Clone>(elem: T, n: usize) -> Vec<T> {
    let mut v: Vec<T> = Vec::with_capacity(n);
    let mut i = 0usize;
    while i < n {
        v.push(elem.clone());
        i += 1usize;
    }
    v
}

/// `vec![a, b, ...]` expansion seed: the first element fixes `T`, the
/// remaining elements are pushed in source order by the expansion.
pub fn __jr_vec_seed<T>(first: T, capacity: usize) -> Vec<T> {
    let mut v: Vec<T> = Vec::with_capacity(capacity);
    v.push(first);
    v
}

/// Element-wise clone in index order, matching std's observable behavior
/// (each element's `clone` runs once, front to back).
impl<T: Clone> Clone for Vec<T> {
    fn clone(&self) -> Vec<T> {
        let mut out: Vec<T> = Vec::with_capacity(self.len);
        let mut i = 0usize;
        while i < self.len {
            out.push(self.get_ref(i).clone());
            i += 1usize;
        }
        out
    }
}

impl<T> super::ops::Index<usize> for Vec<T> {
    type Output = T;
    fn index(&self, index: usize) -> &T {
        self.get_ref(index)
    }
}

impl<T> super::ops::IndexMut<usize> for Vec<T> {
    fn index_mut(&mut self, index: usize) -> &mut T {
        if index >= self.len {
            panic!("index out of bounds");
        }
        let p = slot(self.ptr, index);
        unsafe { &mut *p }
    }
}

// Length is checked before any element comparison. Equal-length inputs are
// visited left to right and stop at the first unequal pair, as std does.
impl<T: super::cmp::PartialEq<U>, U> super::cmp::PartialEq<Vec<U>> for Vec<T> {
    fn eq(&self, other: &Vec<U>) -> bool {
        if self.len != other.len {
            return false;
        }
        let mut i = 0usize;
        while i < self.len {
            if !self.get_ref(i).eq(other.get_ref(i)) {
                return false;
            }
            i += 1usize;
        }
        true
    }
}

impl<T: super::cmp::Eq> super::cmp::Eq for Vec<T> {}

impl<T: super::cmp::PartialEq> Vec<T> {
    pub fn contains(&self, value: &T) -> bool {
        let mut i = 0usize;
        while i < self.len {
            if self.get_ref(i).eq(value) {
                return true;
            }
            i += 1usize;
        }
        false
    }

    pub fn dedup(&mut self) {
        let mut i = 1usize;
        while i < self.len {
            if self.get_ref(i - 1usize).eq(self.get_ref(i)) {
                let _removed = self.remove(i);
            } else {
                i += 1usize;
            }
        }
    }
}

impl<T> Vec<T> {
    /// Stable insertion sort by comparator, like stock's stable `sort_by`.
    pub fn sort_by<F: FnMut(&T, &T) -> super::cmp::Ordering>(&mut self, mut compare: F) {
        let mut i = 1usize;
        while i < self.len {
            let mut j = i;
            while j > 0usize {
                if !compare(self.get_ref(j - 1usize), self.get_ref(j)).is_gt() {
                    break;
                }
                self.swap(j - 1usize, j);
                j -= 1usize;
            }
            i += 1usize;
        }
    }

    /// Stable sort by extracted key. The key is recomputed per comparison.
    pub fn sort_by_key<K: super::cmp::Ord, F: FnMut(&T) -> K>(&mut self, mut key: F) {
        let mut i = 1usize;
        while i < self.len {
            let mut j = i;
            while j > 0usize {
                let left = key(self.get_ref(j - 1usize));
                let right = key(self.get_ref(j));
                if !left.cmp(&right).is_gt() {
                    break;
                }
                self.swap(j - 1usize, j);
                j -= 1usize;
            }
            i += 1usize;
        }
    }
}

impl<T: super::cmp::Ord> Vec<T> {
    pub fn sort(&mut self) {
        // Stable insertion sort. Keep equal elements in their original order.
        let mut i = 1usize;
        while i < self.len {
            let mut j = i;
            while j > 0usize {
                if !self.get_ref(j - 1usize).cmp(self.get_ref(j)).is_gt() {
                    break;
                }
                self.swap(j - 1usize, j);
                j -= 1usize;
            }
            i += 1usize;
        }
    }

    pub fn binary_search(&self, value: &T) -> super::result::Result<usize, usize> {
        let mut left = 0usize;
        let mut right = self.len;
        while left < right {
            let middle = left + (right - left) / 2usize;
            match self.get_ref(middle).cmp(value) {
                super::cmp::Ordering::Less => { left = middle + 1usize; }
                super::cmp::Ordering::Greater => { right = middle; }
                super::cmp::Ordering::Equal => return super::result::Result::Ok(middle),
            }
        }
        super::result::Result::Err(left)
    }
}

impl<T: Clone> Vec<T> {
    /// Clones each element of `other` once, in index order, onto the end.
    pub fn extend_from_slice(&mut self, other: &[T]) {
        self.reserve(other.len());
        let mut i = 0usize;
        while i < other.len() {
            let element = &other[i];
            self.push(element.clone());
            i += 1usize;
        }
    }
}

/// `&Vec<T>` coerces to `&[T]` at expected-type sites, as in std.
impl<T> super::ops::Deref for Vec<T> {
    type Target = [T];
    fn deref(&self) -> &[T] {
        self.as_slice()
    }
}

impl<T> super::ops::DerefMut for Vec<T> {
    fn deref_mut(&mut self) -> &mut [T] {
        self.as_mut_slice()
    }
}
