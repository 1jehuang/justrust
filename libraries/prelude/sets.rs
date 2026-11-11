//! Native HashSet, BTreeSet, VecDeque and BinaryHeap as ordinary source over
//! a dense Vec. HashSet iteration order is unspecified, as in std.
use super::hash::{BuildHasher, Hash, Hasher, RandomState};
use super::option::Option;
use super::vec::Vec;
use super::cmp::{Ord, Ordering};

fn hash_with<K: ?Sized + Hash, S: BuildHasher>(builder: &S, key: &K) -> u64 {
    let mut state = builder.build_hasher();
    key.hash(&mut state);
    state.finish()
}

pub struct HashSet<T, S = RandomState> {
    hashes: Vec<u64>,
    items: Vec<T>,
    hash_builder: S,
}

impl<T: Hash + Eq> HashSet<T, RandomState> {
    pub fn new() -> HashSet<T, RandomState> {
        HashSet::with_hasher(RandomState::new())
    }
    pub fn with_capacity(_capacity: usize) -> HashSet<T, RandomState> {
        HashSet::with_hasher(RandomState::new())
    }
}

impl<T, S> HashSet<T, S> {
    pub fn with_hasher(hash_builder: S) -> HashSet<T, S> {
        HashSet { hashes: Vec::new(), items: Vec::new(), hash_builder }
    }
    pub fn with_capacity_and_hasher(_capacity: usize, hash_builder: S) -> HashSet<T, S> {
        HashSet::with_hasher(hash_builder)
    }
    pub fn hasher(&self) -> &S {
        &self.hash_builder
    }
}

impl<T: Hash + Eq, S: BuildHasher> HashSet<T, S> {
    fn find<Q: ?Sized + Hash + Eq>(&self, value: &Q) -> Option<usize>
    where
        T: super::borrow::Borrow<Q>,
    {
        let h = hash_with(&self.hash_builder, value);
        let mut i = 0usize;
        while i < self.items.len() {
            let stored: &Q = self.items.get_ref(i).borrow();
            if self.hashes[i] == h && stored.eq(value) {
                return Option::Some(i);
            }
            i += 1usize;
        }
        Option::None
    }
    pub fn len(&self) -> usize {
        self.items.len()
    }
    pub fn is_empty(&self) -> bool {
        self.items.len() == 0usize
    }
    pub fn clear(&mut self) {
        self.hashes.clear();
        self.items.clear();
    }
    pub fn contains<Q: ?Sized + Hash + Eq>(&self, value: &Q) -> bool
    where
        T: super::borrow::Borrow<Q>,
    {
        match self.find(value) {
            Option::Some(_) => true,
            Option::None => false,
        }
    }
    pub fn get<Q: ?Sized + Hash + Eq>(&self, value: &Q) -> Option<&T>
    where
        T: super::borrow::Borrow<Q>,
    {
        match self.find(value) {
            Option::Some(i) => Option::Some(self.items.get_ref(i)),
            Option::None => Option::None,
        }
    }
    pub fn insert(&mut self, value: T) -> bool {
        let h = hash_with(&self.hash_builder, &value);
        let mut i = 0usize;
        while i < self.items.len() {
            if self.hashes[i] == h && self.items.get_ref(i).eq(&value) {
                return false;
            }
            i += 1usize;
        }
        self.hashes.push(h);
        self.items.push(value);
        true
    }
    pub fn remove<Q: ?Sized + Hash + Eq>(&mut self, value: &Q) -> bool
    where
        T: super::borrow::Borrow<Q>,
    {
        match self.find(value) {
            Option::Some(i) => {
                let _h = self.hashes.swap_remove(i);
                let _v = self.items.swap_remove(i);
                true
            }
            Option::None => false,
        }
    }
    pub fn take<Q: ?Sized + Hash + Eq>(&mut self, value: &Q) -> Option<T>
    where
        T: super::borrow::Borrow<Q>,
    {
        match self.find(value) {
            Option::Some(i) => {
                let _h = self.hashes.swap_remove(i);
                Option::Some(self.items.swap_remove(i))
            }
            Option::None => Option::None,
        }
    }
    pub fn iter<'a>(&'a self) -> super::slice::Iter<'a, T> {
        self.items.iter()
    }
    fn filtered<'a>(&'a self, other: &HashSet<T, S>, keep_present: bool, out: &mut Vec<&'a T>) {
        let mut i = 0usize;
        while i < self.items.len() {
            let item = self.items.get_ref(i);
            if other.contains(item) == keep_present {
                out.push(item);
            }
            i += 1usize;
        }
    }
    pub fn intersection<'a>(&'a self, other: &'a HashSet<T, S>) -> super::vec::IntoIter<&'a T> {
        let mut out = Vec::new();
        self.filtered(other, true, &mut out);
        out.into_iter()
    }
    pub fn difference<'a>(&'a self, other: &'a HashSet<T, S>) -> super::vec::IntoIter<&'a T> {
        let mut out = Vec::new();
        self.filtered(other, false, &mut out);
        out.into_iter()
    }
    pub fn union<'a>(&'a self, other: &'a HashSet<T, S>) -> super::vec::IntoIter<&'a T> {
        let mut out = Vec::new();
        self.filtered(other, true, &mut out);
        self.filtered(other, false, &mut out);
        other.filtered(self, false, &mut out);
        out.into_iter()
    }
    pub fn symmetric_difference<'a>(&'a self, other: &'a HashSet<T, S>) -> super::vec::IntoIter<&'a T> {
        let mut out = Vec::new();
        self.filtered(other, false, &mut out);
        other.filtered(self, false, &mut out);
        out.into_iter()
    }
    pub fn is_subset(&self, other: &HashSet<T, S>) -> bool {
        let mut i = 0usize;
        while i < self.items.len() {
            if !other.contains(self.items.get_ref(i)) {
                return false;
            }
            i += 1usize;
        }
        true
    }
    pub fn is_superset(&self, other: &HashSet<T, S>) -> bool {
        other.is_subset(self)
    }
    pub fn is_disjoint(&self, other: &HashSet<T, S>) -> bool {
        let mut i = 0usize;
        while i < self.items.len() {
            if other.contains(self.items.get_ref(i)) {
                return false;
            }
            i += 1usize;
        }
        true
    }
    pub fn retain<F: FnMut(&T) -> bool>(&mut self, mut f: F) {
        let mut i = 0usize;
        while i < self.items.len() {
            if f(self.items.get_ref(i)) {
                i += 1usize;
            } else {
                let _h = self.hashes.swap_remove(i);
                let _v = self.items.swap_remove(i);
            }
        }
    }
}

impl<T: Hash + Eq, S: BuildHasher + super::default::Default> super::iter::FromIterator<T> for HashSet<T, S> {
    fn from_iter<I: super::iter::IntoIterator<Item = T>>(source: I) -> HashSet<T, S> {
        let mut set = HashSet::with_hasher(S::default());
        let mut iterator = source.into_iter();
        loop {
            match iterator.next() {
                Option::Some(value) => {
                    set.insert(value);
                }
                Option::None => return set,
            }
        }
    }
}

impl<'a, T: Hash + Eq, S: BuildHasher> super::iter::IntoIterator for &'a HashSet<T, S> {
    type Item = &'a T;
    type IntoIter = super::slice::Iter<'a, T>;
    fn into_iter(self) -> super::slice::Iter<'a, T> {
        self.items.iter()
    }
}

impl<T, S> super::iter::IntoIterator for HashSet<T, S> {
    type Item = T;
    type IntoIter = super::vec::IntoIter<T>;
    fn into_iter(self) -> super::vec::IntoIter<T> {
        self.items.into_iter()
    }
}

impl<T, S: super::default::Default> super::default::Default for HashSet<T, S> {
    fn default() -> HashSet<T, S> {
        HashSet::with_hasher(S::default())
    }
}

impl<T: Clone, S: Clone> Clone for HashSet<T, S> {
    fn clone(&self) -> HashSet<T, S> {
        HashSet { hashes: self.hashes.clone(), items: self.items.clone(), hash_builder: self.hash_builder.clone() }
    }
}

impl<T: Hash + Eq, S: BuildHasher> super::cmp::PartialEq for HashSet<T, S> {
    fn eq(&self, other: &HashSet<T, S>) -> bool {
        self.len() == other.len() && self.is_subset(other)
    }
}
impl<T: Hash + Eq, S: BuildHasher> super::cmp::Eq for HashSet<T, S> {}

impl<T: super::fmt::Debug, S> super::fmt::Debug for HashSet<T, S> {
    fn fmt(&self, f: &mut super::fmt::Formatter) -> super::fmt::Result {
        let mut list = f.debug_set();
        let mut i = 0usize;
        while i < self.items.len() {
            list = list.entry(self.items.get_ref(i));
            i += 1usize;
        }
        list.finish()
    }
}

/// BTreeSet<T>: an ordered set kept as a sorted dense vector.
pub struct BTreeSet<T> {
    items: Vec<T>,
}

impl<T: Ord> BTreeSet<T> {
    pub fn new() -> BTreeSet<T> {
        BTreeSet { items: Vec::new() }
    }
    fn search(&self, value: &T) -> super::result::Result<usize, usize> {
        let mut left = 0usize;
        let mut right = self.items.len();
        while left < right {
            let middle = left + (right - left) / 2usize;
            match self.items.get_ref(middle).cmp(value) {
                Ordering::Less => left = middle + 1usize,
                Ordering::Greater => right = middle,
                Ordering::Equal => return super::result::Result::Ok(middle),
            }
        }
        super::result::Result::Err(left)
    }
    pub fn len(&self) -> usize {
        self.items.len()
    }
    pub fn is_empty(&self) -> bool {
        self.items.len() == 0usize
    }
    pub fn clear(&mut self) {
        self.items.clear();
    }
    /// `search` for a borrowed form of the element, as in `contains("name")`
    /// on `BTreeSet<String>`. `Borrow` keeps the two orders identical.
    fn search_borrowed<Q: ?Sized + Ord>(&self, value: &Q) -> super::result::Result<usize, usize>
    where
        T: super::borrow::Borrow<Q>,
    {
        let mut left = 0usize;
        let mut right = self.items.len();
        while left < right {
            let middle = left + (right - left) / 2usize;
            let stored: &Q = self.items.get_ref(middle).borrow();
            match stored.cmp(value) {
                Ordering::Less => left = middle + 1usize,
                Ordering::Greater => right = middle,
                Ordering::Equal => return super::result::Result::Ok(middle),
            }
        }
        super::result::Result::Err(left)
    }
    pub fn contains<Q: ?Sized + Ord>(&self, value: &Q) -> bool
    where
        T: super::borrow::Borrow<Q>,
    {
        match self.search_borrowed(value) {
            super::result::Result::Ok(_) => true,
            super::result::Result::Err(_) => false,
        }
    }
    pub fn insert(&mut self, value: T) -> bool {
        match self.search(&value) {
            super::result::Result::Ok(_) => false,
            super::result::Result::Err(i) => {
                self.items.insert(i, value);
                true
            }
        }
    }
    pub fn remove<Q: ?Sized + Ord>(&mut self, value: &Q) -> bool
    where
        T: super::borrow::Borrow<Q>,
    {
        match self.search_borrowed(value) {
            super::result::Result::Ok(i) => {
                let _v = self.items.remove(i);
                true
            }
            super::result::Result::Err(_) => false,
        }
    }
    pub fn get<Q: ?Sized + Ord>(&self, value: &Q) -> Option<&T>
    where
        T: super::borrow::Borrow<Q>,
    {
        match self.search_borrowed(value) {
            super::result::Result::Ok(i) => Option::Some(self.items.get_ref(i)),
            super::result::Result::Err(_) => Option::None,
        }
    }
    pub fn take<Q: ?Sized + Ord>(&mut self, value: &Q) -> Option<T>
    where
        T: super::borrow::Borrow<Q>,
    {
        match self.search_borrowed(value) {
            super::result::Result::Ok(i) => Option::Some(self.items.remove(i)),
            super::result::Result::Err(_) => Option::None,
        }
    }
    pub fn first(&self) -> Option<&T> {
        if self.items.len() == 0usize {
            Option::None
        } else {
            Option::Some(self.items.get_ref(0usize))
        }
    }
    pub fn last(&self) -> Option<&T> {
        let len = self.items.len();
        if len == 0usize {
            Option::None
        } else {
            Option::Some(self.items.get_ref(len - 1usize))
        }
    }
    pub fn pop_first(&mut self) -> Option<T> {
        if self.items.len() == 0usize {
            Option::None
        } else {
            Option::Some(self.items.remove(0usize))
        }
    }
    pub fn pop_last(&mut self) -> Option<T> {
        self.items.pop()
    }
    pub fn iter<'a>(&'a self) -> super::slice::Iter<'a, T> {
        self.items.iter()
    }
    fn filtered<'a>(&'a self, other: &BTreeSet<T>, keep_present: bool, out: &mut Vec<&'a T>) {
        let mut i = 0usize;
        while i < self.items.len() {
            let item = self.items.get_ref(i);
            if other.contains(item) == keep_present {
                out.push(item);
            }
            i += 1usize;
        }
    }
    pub fn intersection<'a>(&'a self, other: &'a BTreeSet<T>) -> super::vec::IntoIter<&'a T> {
        let mut out = Vec::new();
        self.filtered(other, true, &mut out);
        out.into_iter()
    }
    pub fn difference<'a>(&'a self, other: &'a BTreeSet<T>) -> super::vec::IntoIter<&'a T> {
        let mut out = Vec::new();
        self.filtered(other, false, &mut out);
        out.into_iter()
    }
    pub fn is_subset(&self, other: &BTreeSet<T>) -> bool {
        let mut i = 0usize;
        while i < self.items.len() {
            if !other.contains(self.items.get_ref(i)) {
                return false;
            }
            i += 1usize;
        }
        true
    }
}

impl<T: Ord> super::iter::FromIterator<T> for BTreeSet<T> {
    fn from_iter<I: super::iter::IntoIterator<Item = T>>(source: I) -> BTreeSet<T> {
        let mut set = BTreeSet::new();
        let mut iterator = source.into_iter();
        loop {
            match iterator.next() {
                Option::Some(value) => {
                    set.insert(value);
                }
                Option::None => return set,
            }
        }
    }
}

impl<'a, T: Ord> super::iter::IntoIterator for &'a BTreeSet<T> {
    type Item = &'a T;
    type IntoIter = super::slice::Iter<'a, T>;
    fn into_iter(self) -> super::slice::Iter<'a, T> {
        self.items.iter()
    }
}

impl<T> super::iter::IntoIterator for BTreeSet<T> {
    type Item = T;
    type IntoIter = super::vec::IntoIter<T>;
    fn into_iter(self) -> super::vec::IntoIter<T> {
        self.items.into_iter()
    }
}

impl<T: Ord> super::default::Default for BTreeSet<T> {
    fn default() -> BTreeSet<T> {
        BTreeSet::new()
    }
}

impl<T: Clone> Clone for BTreeSet<T> {
    fn clone(&self) -> BTreeSet<T> {
        BTreeSet { items: self.items.clone() }
    }
}

impl<T: super::fmt::Debug> super::fmt::Debug for BTreeSet<T> {
    fn fmt(&self, f: &mut super::fmt::Formatter) -> super::fmt::Result {
        let mut list = f.debug_set();
        let mut i = 0usize;
        while i < self.items.len() {
            list = list.entry(self.items.get_ref(i));
            i += 1usize;
        }
        list.finish()
    }
}

/// VecDeque<T>: a double-ended queue over a dense vector.
pub struct VecDeque<T> {
    items: Vec<T>,
}

impl<T> VecDeque<T> {
    pub fn new() -> VecDeque<T> {
        VecDeque { items: Vec::new() }
    }
    pub fn with_capacity(_capacity: usize) -> VecDeque<T> {
        VecDeque::new()
    }
    pub fn len(&self) -> usize {
        self.items.len()
    }
    pub fn is_empty(&self) -> bool {
        self.items.len() == 0usize
    }
    pub fn clear(&mut self) {
        self.items.clear();
    }
    pub fn push_back(&mut self, value: T) {
        self.items.push(value);
    }
    pub fn push_front(&mut self, value: T) {
        self.items.insert(0usize, value);
    }
    pub fn pop_back(&mut self) -> Option<T> {
        self.items.pop()
    }
    pub fn pop_front(&mut self) -> Option<T> {
        if self.items.len() == 0usize {
            Option::None
        } else {
            Option::Some(self.items.remove(0usize))
        }
    }
    pub fn front(&self) -> Option<&T> {
        if self.items.len() == 0usize {
            Option::None
        } else {
            Option::Some(self.items.get_ref(0usize))
        }
    }
    pub fn back(&self) -> Option<&T> {
        let len = self.items.len();
        if len == 0usize {
            Option::None
        } else {
            Option::Some(self.items.get_ref(len - 1usize))
        }
    }
    pub fn front_mut(&mut self) -> Option<&mut T> {
        self.items.get_mut(0usize)
    }
    pub fn back_mut(&mut self) -> Option<&mut T> {
        let len = self.items.len();
        if len == 0usize {
            Option::None
        } else {
            self.items.get_mut(len - 1usize)
        }
    }
    pub fn get(&self, index: usize) -> Option<&T> {
        if index < self.items.len() {
            Option::Some(self.items.get_ref(index))
        } else {
            Option::None
        }
    }
    pub fn get_mut(&mut self, index: usize) -> Option<&mut T> {
        self.items.get_mut(index)
    }
    pub fn insert(&mut self, index: usize, value: T) {
        self.items.insert(index, value);
    }
    pub fn remove(&mut self, index: usize) -> Option<T> {
        if index < self.items.len() {
            Option::Some(self.items.remove(index))
        } else {
            Option::None
        }
    }
    pub fn swap(&mut self, a: usize, b: usize) {
        self.items.swap(a, b);
    }
    pub fn rotate_left(&mut self, n: usize) {
        assert!(n <= self.items.len());
        let mut k = 0usize;
        while k < n {
            let value = self.items.remove(0usize);
            self.items.push(value);
            k += 1usize;
        }
    }
    pub fn rotate_right(&mut self, n: usize) {
        assert!(n <= self.items.len());
        let mut k = 0usize;
        while k < n {
            match self.items.pop() {
                Option::Some(value) => self.items.insert(0usize, value),
                Option::None => {}
            }
            k += 1usize;
        }
    }
    pub fn truncate(&mut self, len: usize) {
        self.items.truncate(len);
    }
    pub fn iter<'a>(&'a self) -> super::slice::Iter<'a, T> {
        self.items.iter()
    }
    pub fn make_contiguous(&mut self) -> &mut [T] {
        &mut self.items[..]
    }
}

impl<T: super::cmp::PartialEq> VecDeque<T> {
    pub fn contains(&self, value: &T) -> bool {
        self.items.contains(value)
    }
}

impl<T> super::ops::Index<usize> for VecDeque<T> {
    type Output = T;
    fn index(&self, index: usize) -> &T {
        self.items.get_ref(index)
    }
}


impl<T> super::iter::FromIterator<T> for VecDeque<T> {
    fn from_iter<I: super::iter::IntoIterator<Item = T>>(source: I) -> VecDeque<T> {
        let mut items = Vec::new();
        let mut iterator = source.into_iter();
        loop {
            match iterator.next() {
                Option::Some(value) => items.push(value),
                Option::None => return VecDeque { items },
            }
        }
    }
}

impl<'a, T> super::iter::IntoIterator for &'a VecDeque<T> {
    type Item = &'a T;
    type IntoIter = super::slice::Iter<'a, T>;
    fn into_iter(self) -> super::slice::Iter<'a, T> {
        self.items.iter()
    }
}

impl<T> super::iter::IntoIterator for VecDeque<T> {
    type Item = T;
    type IntoIter = super::vec::IntoIter<T>;
    fn into_iter(self) -> super::vec::IntoIter<T> {
        self.items.into_iter()
    }
}

impl<T> super::default::Default for VecDeque<T> {
    fn default() -> VecDeque<T> {
        VecDeque::new()
    }
}

impl<T: Clone> Clone for VecDeque<T> {
    fn clone(&self) -> VecDeque<T> {
        VecDeque { items: self.items.clone() }
    }
}

impl<T, const N: usize> super::convert::From<[T; N]> for VecDeque<T> {
    fn from(items: [T; N]) -> VecDeque<T> {
        let mut queue = VecDeque::new();
        for item in items {
            queue.push_back(item);
        }
        queue
    }
}

impl<T> super::convert::From<Vec<T>> for VecDeque<T> {
    fn from(items: Vec<T>) -> VecDeque<T> {
        VecDeque { items }
    }
}

impl<T: super::fmt::Debug> super::fmt::Debug for VecDeque<T> {
    fn fmt(&self, f: &mut super::fmt::Formatter) -> super::fmt::Result {
        let mut list = f.debug_list();
        let mut i = 0usize;
        while i < self.items.len() {
            list = list.entry(self.items.get_ref(i));
            i += 1usize;
        }
        list.finish()
    }
}

/// BinaryHeap<T>: a max-heap over a dense vector.
pub struct BinaryHeap<T> {
    items: Vec<T>,
}

impl<T: Ord> BinaryHeap<T> {
    pub fn new() -> BinaryHeap<T> {
        BinaryHeap { items: Vec::new() }
    }
    pub fn with_capacity(_capacity: usize) -> BinaryHeap<T> {
        BinaryHeap::new()
    }
    pub fn len(&self) -> usize {
        self.items.len()
    }
    pub fn is_empty(&self) -> bool {
        self.items.len() == 0usize
    }
    pub fn clear(&mut self) {
        self.items.clear();
    }
    pub fn peek(&self) -> Option<&T> {
        if self.items.len() == 0usize {
            Option::None
        } else {
            Option::Some(self.items.get_ref(0usize))
        }
    }
    fn sift_up(&mut self, mut i: usize) {
        while i > 0usize {
            let parent = (i - 1usize) / 2usize;
            if self.items.get_ref(i).cmp(self.items.get_ref(parent)) == Ordering::Greater {
                self.items.swap(i, parent);
                i = parent;
            } else {
                return;
            }
        }
    }
    fn sift_down(&mut self, mut i: usize) {
        let len = self.items.len();
        loop {
            let left = 2usize * i + 1usize;
            let right = left + 1usize;
            let mut largest = i;
            if left < len && self.items.get_ref(left).cmp(self.items.get_ref(largest)) == Ordering::Greater {
                largest = left;
            }
            if right < len && self.items.get_ref(right).cmp(self.items.get_ref(largest)) == Ordering::Greater {
                largest = right;
            }
            if largest == i {
                return;
            }
            self.items.swap(i, largest);
            i = largest;
        }
    }
    pub fn push(&mut self, value: T) {
        self.items.push(value);
        let last = self.items.len() - 1usize;
        self.sift_up(last);
    }
    pub fn pop(&mut self) -> Option<T> {
        let len = self.items.len();
        if len == 0usize {
            return Option::None;
        }
        self.items.swap(0usize, len - 1usize);
        let top = self.items.pop();
        if self.items.len() > 1usize {
            self.sift_down(0usize);
        }
        top
    }
    pub fn into_sorted_vec(mut self) -> Vec<T> {
        let mut out = Vec::new();
        loop {
            match self.pop() {
                Option::Some(value) => out.push(value),
                Option::None => break,
            }
        }
        out.reverse();
        out
    }
    pub fn into_vec(self) -> Vec<T> {
        self.items
    }
    pub fn iter<'a>(&'a self) -> super::slice::Iter<'a, T> {
        self.items.iter()
    }
}

impl<T: Ord> super::iter::FromIterator<T> for BinaryHeap<T> {
    fn from_iter<I: super::iter::IntoIterator<Item = T>>(source: I) -> BinaryHeap<T> {
        let mut heap = BinaryHeap::new();
        let mut iterator = source.into_iter();
        loop {
            match iterator.next() {
                Option::Some(value) => heap.push(value),
                Option::None => return heap,
            }
        }
    }
}

impl<T: Ord> super::default::Default for BinaryHeap<T> {
    fn default() -> BinaryHeap<T> {
        BinaryHeap::new()
    }
}

impl<T: super::fmt::Debug> super::fmt::Debug for BinaryHeap<T> {
    fn fmt(&self, f: &mut super::fmt::Formatter) -> super::fmt::Result {
        let mut list = f.debug_list();
        let mut i = 0usize;
        while i < self.items.len() {
            list = list.entry(self.items.get_ref(i));
            i += 1usize;
        }
        list.finish()
    }
}

impl<T: Hash + Eq, S: BuildHasher> super::iter::Extend<T> for HashSet<T, S> {
    fn extend<I: super::iter::IntoIterator<Item = T>>(&mut self, iter: I) {
        let mut iterator = iter.into_iter();
        loop {
            match iterator.next() {
                Option::Some(value) => {
                    self.insert(value);
                }
                Option::None => return,
            }
        }
    }
    fn extend_one(&mut self, value: T) {
        self.insert(value);
    }
}

impl<T: Ord> super::iter::Extend<T> for BTreeSet<T> {
    fn extend<I: super::iter::IntoIterator<Item = T>>(&mut self, iter: I) {
        let mut iterator = iter.into_iter();
        loop {
            match iterator.next() {
                Option::Some(value) => {
                    self.insert(value);
                }
                Option::None => return,
            }
        }
    }
    fn extend_one(&mut self, value: T) {
        self.insert(value);
    }
}

impl<T> super::iter::Extend<T> for VecDeque<T> {
    fn extend<I: super::iter::IntoIterator<Item = T>>(&mut self, iter: I) {
        let mut iterator = iter.into_iter();
        loop {
            match iterator.next() {
                Option::Some(value) => {
                    self.items.push(value);
                }
                Option::None => return,
            }
        }
    }
    fn extend_one(&mut self, value: T) {
        self.items.push(value);
    }
}

impl<T: Ord> super::iter::Extend<T> for BinaryHeap<T> {
    fn extend<I: super::iter::IntoIterator<Item = T>>(&mut self, iter: I) {
        let mut iterator = iter.into_iter();
        loop {
            match iterator.next() {
                Option::Some(value) => {
                    self.push(value);
                }
                Option::None => return,
            }
        }
    }
    fn extend_one(&mut self, value: T) {
        self.push(value);
    }
}
