// Native Vec<T>: a growable heap buffer written in ordinary Rust source.
use super::mem::{raw_realloc, size_of};
use super::option::Option;

pub struct Vec<T> {
    ptr: *mut T,
    len: usize,
    cap: usize,
}

fn slot<T>(base: *mut T, index: usize) -> *mut T {
    (base as usize + index * size_of::<T>()) as *mut T
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

    pub fn iter<'a>(&'a self) -> super::slice::Iter<'a, T> {
        super::slice::Iter::new(self)
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
            super::mem::align_of::<T>() as *const T
        } else {
            self.ptr as *const T
        }
    }

    pub fn as_mut_ptr(&mut self) -> *mut T {
        if self.ptr as usize == 0usize {
            super::mem::align_of::<T>() as *mut T
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
        let size = size_of::<T>();
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
        if self.len == self.cap {
            self.reserve(1usize);
        }
        let mut i = self.len;
        while i > index {
            let src = slot(self.ptr, i - 1usize);
            let dst = slot(self.ptr, i);
            unsafe {
                *dst = *src;
            }
            i -= 1usize;
        }
        let dst = slot(self.ptr, index);
        unsafe {
            *dst = value;
        }
        self.len += 1usize;
    }

    pub fn remove(&mut self, index: usize) -> T {
        if index >= self.len {
            panic!("removal index out of bounds");
        }
        let src = slot(self.ptr, index);
        let value = unsafe { *src };
        let mut i = index;
        while i + 1usize < self.len {
            let from = slot(self.ptr, i + 1usize);
            let to = slot(self.ptr, i);
            unsafe {
                *to = *from;
            }
            i += 1usize;
        }
        self.len -= 1usize;
        value
    }

    pub fn swap_remove(&mut self, index: usize) -> T {
        if index >= self.len {
            panic!("swap_remove index out of bounds");
        }
        let at = slot(self.ptr, index);
        let value = unsafe { *at };
        self.len -= 1usize;
        if index != self.len {
            let last = slot(self.ptr, self.len);
            unsafe {
                *at = *last;
            }
        }
        value
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

// TODO(drop): enable `impl<T> Drop for Vec<T>` (truncate + raw_free) once
// aggregates containing Drop owners get drop glue. Until then Vec leaks.

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
