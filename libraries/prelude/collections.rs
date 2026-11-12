//! Native std::collections. HashMap<K, V> is ordinary source: a dense entry
//! vector with cached hashes. Iteration order is unspecified, like std's.
use super::hash::{BuildHasher, Hash, Hasher, RandomState};
use super::option::Option;
use super::vec::Vec;

fn hash_with<K: ?Sized + Hash, S: BuildHasher>(builder: &S, key: &K) -> u64 {
    let mut state = builder.build_hasher();
    key.hash(&mut state);
    state.finish()
}

pub struct HashMap<K, V, S = RandomState> {
    hashes: Vec<u64>,
    keys: Vec<K>,
    values: Vec<V>,
    hash_builder: S,
}

impl<K: Hash + Eq, V> HashMap<K, V, RandomState> {
    pub fn new() -> HashMap<K, V, RandomState> {
        HashMap::with_hasher(RandomState::new())
    }

    pub fn with_capacity(capacity: usize) -> HashMap<K, V, RandomState> {
        HashMap::with_capacity_and_hasher(capacity, RandomState::new())
    }
}

impl<K: Hash + Eq, V, const N: usize> super::convert::From<[(K, V); N]>
    for HashMap<K, V, RandomState>
{
    fn from(entries: [(K, V); N]) -> HashMap<K, V, RandomState> {
        let mut map: HashMap<K, V, RandomState> = HashMap::new();
        for (key, value) in entries {
            map.insert(key, value);
        }
        map
    }
}

impl<K, V, S> HashMap<K, V, S> {
    pub fn with_hasher(hash_builder: S) -> HashMap<K, V, S> {
        HashMap { hashes: Vec::new(), keys: Vec::new(), values: Vec::new(), hash_builder }
    }

    pub fn with_capacity_and_hasher(_capacity: usize, hash_builder: S) -> HashMap<K, V, S> {
        HashMap::with_hasher(hash_builder)
    }

    pub fn hasher(&self) -> &S {
        &self.hash_builder
    }

    pub fn capacity(&self) -> usize {
        self.keys.capacity()
    }

    pub fn clear(&mut self) {
        self.hashes.clear();
        self.keys.clear();
        self.values.clear();
    }
}

impl<K: Hash + Eq, V, S: BuildHasher> HashMap<K, V, S> {

    fn find_key(&self, key: &K) -> Option<usize> {
        let h = hash_with(&self.hash_builder, key);
        let mut i = 0usize;
        while i < self.keys.len() {
            if self.hashes[i] == h && self.keys.get_ref(i).eq(key) {
                return Option::Some(i);
            }
            i += 1usize;
        }
        Option::None
    }

    fn find<Q: ?Sized + Hash + Eq>(&self, key: &Q) -> Option<usize>
    where
        K: super::borrow::Borrow<Q>,
    {
        let h = hash_with(&self.hash_builder, key);
        let mut i = 0usize;
        while i < self.keys.len() {
            let stored: &Q = self.keys.get_ref(i).borrow();
            if self.hashes[i] == h && stored.eq(key) {
                return Option::Some(i);
            }
            i += 1usize;
        }
        Option::None
    }

    pub fn len(&self) -> usize {
        self.keys.len()
    }

    pub fn is_empty(&self) -> bool {
        self.keys.len() == 0usize
    }

    pub fn contains_key<Q: ?Sized + Hash + Eq>(&self, key: &Q) -> bool
    where
        K: super::borrow::Borrow<Q>,
    {
        match self.find(key) {
            Option::Some(_) => true,
            Option::None => false,
        }
    }

    pub fn get<Q: ?Sized + Hash + Eq>(&self, key: &Q) -> Option<&V>
    where
        K: super::borrow::Borrow<Q>,
    {
        match self.find(key) {
            Option::Some(i) => Option::Some(self.values.get_ref(i)),
            Option::None => Option::None,
        }
    }

    pub fn get_mut<Q: ?Sized + Hash + Eq>(&mut self, key: &Q) -> Option<&mut V>
    where
        K: super::borrow::Borrow<Q>,
    {
        match self.find(key) {
            Option::Some(i) => self.values.get_mut(i),
            Option::None => Option::None,
        }
    }

    pub fn insert(&mut self, key: K, value: V) -> Option<V> {
        match self.find_key(&key) {
            Option::Some(i) => {
                let old = self.values.swap_remove(i);
                self.values.push(value);
                let last = self.values.len() - 1usize;
                self.values.swap(i, last);
                Option::Some(old)
            }
            Option::None => {
                self.hashes.push(hash_with(&self.hash_builder, &key));
                self.keys.push(key);
                self.values.push(value);
                Option::None
            }
        }
    }

    pub fn remove<Q: ?Sized + Hash + Eq>(&mut self, key: &Q) -> Option<V>
    where
        K: super::borrow::Borrow<Q>,
    {
        match self.find(key) {
            Option::Some(i) => {
                let _h = self.hashes.swap_remove(i);
                let _k = self.keys.swap_remove(i);
                Option::Some(self.values.swap_remove(i))
            }
            Option::None => Option::None,
        }
    }

    pub fn entry<'a>(&'a mut self, key: K) -> Entry<'a, K, V, S> {
        Entry { map: self, key }
    }

    pub fn iter<'a>(&'a self) -> Iter<'a, K, V> {
        Iter { keys: self.keys.as_slice(), values: self.values.as_slice(), index: 0usize }
    }

    pub fn keys<'a>(&'a self) -> super::slice::Iter<'a, K> {
        self.keys.iter()
    }

    pub fn values<'a>(&'a self) -> super::slice::Iter<'a, V> {
        self.values.iter()
    }
}

pub struct Entry<'a, K, V, S = RandomState> {
    map: &'a mut HashMap<K, V, S>,
    key: K,
}

impl<'a, K: Hash + Eq, V, S: BuildHasher> Entry<'a, K, V, S> {
    pub fn or_insert_with<F: FnOnce() -> V>(self, default: F) -> &'a mut V {
        let map = self.map;
        let index = match map.find_key(&self.key) {
            Option::Some(i) => i,
            Option::None => {
                let value = default();
                map.hashes.push(hash_with(&map.hash_builder, &self.key));
                map.keys.push(self.key);
                map.values.push(value);
                map.values.len() - 1usize
            }
        };
        match map.values.get_mut(index) {
            Option::Some(v) => v,
            Option::None => panic!("entry slot missing"),
        }
    }

    pub fn or_insert_with_key<F: FnOnce(&K) -> V>(self, default: F) -> &'a mut V {
        let map = self.map;
        let index = match map.find_key(&self.key) {
            Option::Some(i) => i,
            Option::None => {
                let value = default(&self.key);
                map.hashes.push(hash_with(&map.hash_builder, &self.key));
                map.keys.push(self.key);
                map.values.push(value);
                map.values.len() - 1usize
            }
        };
        match map.values.get_mut(index) {
            Option::Some(v) => v,
            Option::None => panic!("entry slot missing"),
        }
    }

    pub fn or_insert(self, default: V) -> &'a mut V {
        let map = self.map;
        let index = match map.find_key(&self.key) {
            Option::Some(i) => i,
            Option::None => {
                map.hashes.push(hash_with(&map.hash_builder, &self.key));
                map.keys.push(self.key);
                map.values.push(default);
                map.values.len() - 1usize
            }
        };
        match map.values.get_mut(index) {
            Option::Some(v) => v,
            Option::None => panic!("entry slot missing"),
        }
    }

    /// Runs `f` on the value when the key is present, then hands the entry
    /// back so an `or_insert` family call can follow.
    pub fn and_modify<F: FnOnce(&mut V)>(self, f: F) -> Entry<'a, K, V, S> {
        let map = self.map;
        match map.find_key(&self.key) {
            Option::Some(i) => match map.values.get_mut(i) {
                Option::Some(v) => f(v),
                Option::None => panic!("entry slot missing"),
            },
            Option::None => {}
        }
        Entry { map, key: self.key }
    }
}

impl<'a, K: Hash + Eq, V: super::default::Default, S: BuildHasher> Entry<'a, K, V, S> {
    pub fn or_default(self) -> &'a mut V {
        let map = self.map;
        let index = match map.find_key(&self.key) {
            Option::Some(i) => i,
            Option::None => {
                let value = V::default();
                map.hashes.push(hash_with(&map.hash_builder, &self.key));
                map.keys.push(self.key);
                map.values.push(value);
                map.values.len() - 1usize
            }
        };
        match map.values.get_mut(index) {
            Option::Some(v) => v,
            Option::None => panic!("entry slot missing"),
        }
    }
}

pub struct Iter<'a, K, V> {
    keys: &'a [K],
    values: &'a [V],
    index: usize,
}

impl<'a, K, V> super::iter::Iterator for Iter<'a, K, V> {
    type Item = (&'a K, &'a V);
    fn next(&mut self) -> Option<(&'a K, &'a V)> {
        if self.index < self.keys.len() {
            let i = self.index;
            self.index += 1usize;
            Option::Some((&self.keys[i], &self.values[i]))
        } else {
            Option::None
        }
    }
}

impl<'a, K: Hash + Eq, V, S: BuildHasher> super::iter::IntoIterator for &'a HashMap<K, V, S> {
    type Item = (&'a K, &'a V);
    type IntoIter = Iter<'a, K, V>;
    fn into_iter(self) -> Iter<'a, K, V> {
        self.iter()
    }
}

/// BTreeMap<K, V>: an ordered map. Keys are kept sorted in a dense vector
/// with values in a parallel vector, so iteration is ascending key order,
/// as with std. Lookup is binary search by `Ord`.
pub struct BTreeMap<K, V> {
    keys: Vec<K>,
    values: Vec<V>,
}

impl<K: super::cmp::Ord, V> BTreeMap<K, V> {
    pub fn new() -> BTreeMap<K, V> {
        BTreeMap { keys: Vec::new(), values: Vec::new() }
    }

    /// Ok(index) when present, Err(insertion point) otherwise.
    fn search(&self, key: &K) -> super::result::Result<usize, usize> {
        let mut left = 0usize;
        let mut right = self.keys.len();
        while left < right {
            let middle = left + (right - left) / 2usize;
            match self.keys.get_ref(middle).cmp(key) {
                super::cmp::Ordering::Less => left = middle + 1usize,
                super::cmp::Ordering::Greater => right = middle,
                super::cmp::Ordering::Equal => return super::result::Result::Ok(middle),
            }
        }
        super::result::Result::Err(left)
    }

    pub fn len(&self) -> usize {
        self.keys.len()
    }

    pub fn is_empty(&self) -> bool {
        self.keys.len() == 0usize
    }

    /// `search` for a borrowed form of the key, as in `get("name")` on
    /// `BTreeMap<String, V>`. `Borrow` keeps the two orders identical.
    fn search_borrowed<Q: ?Sized + super::cmp::Ord>(
        &self,
        key: &Q,
    ) -> super::result::Result<usize, usize>
    where
        K: super::borrow::Borrow<Q>,
    {
        let mut left = 0usize;
        let mut right = self.keys.len();
        while left < right {
            let middle = left + (right - left) / 2usize;
            let stored: &Q = self.keys.get_ref(middle).borrow();
            match stored.cmp(key) {
                super::cmp::Ordering::Less => left = middle + 1usize,
                super::cmp::Ordering::Greater => right = middle,
                super::cmp::Ordering::Equal => return super::result::Result::Ok(middle),
            }
        }
        super::result::Result::Err(left)
    }

    pub fn contains_key<Q: ?Sized + super::cmp::Ord>(&self, key: &Q) -> bool
    where
        K: super::borrow::Borrow<Q>,
    {
        match self.search_borrowed(key) {
            super::result::Result::Ok(_) => true,
            super::result::Result::Err(_) => false,
        }
    }

    pub fn get<Q: ?Sized + super::cmp::Ord>(&self, key: &Q) -> Option<&V>
    where
        K: super::borrow::Borrow<Q>,
    {
        match self.search_borrowed(key) {
            super::result::Result::Ok(i) => Option::Some(self.values.get_ref(i)),
            super::result::Result::Err(_) => Option::None,
        }
    }

    pub fn get_mut<Q: ?Sized + super::cmp::Ord>(&mut self, key: &Q) -> Option<&mut V>
    where
        K: super::borrow::Borrow<Q>,
    {
        match self.search_borrowed(key) {
            super::result::Result::Ok(i) => self.values.get_mut(i),
            super::result::Result::Err(_) => Option::None,
        }
    }

    pub fn insert(&mut self, key: K, value: V) -> Option<V> {
        match self.search(&key) {
            super::result::Result::Ok(i) => {
                self.values.insert(i, value);
                Option::Some(self.values.remove(i + 1usize))
            }
            super::result::Result::Err(i) => {
                self.keys.insert(i, key);
                self.values.insert(i, value);
                Option::None
            }
        }
    }

    pub fn remove<Q: ?Sized + super::cmp::Ord>(&mut self, key: &Q) -> Option<V>
    where
        K: super::borrow::Borrow<Q>,
    {
        match self.search_borrowed(key) {
            super::result::Result::Ok(i) => {
                let _k = self.keys.remove(i);
                Option::Some(self.values.remove(i))
            }
            super::result::Result::Err(_) => Option::None,
        }
    }

    pub fn entry<'a>(&'a mut self, key: K) -> BTreeEntry<'a, K, V> {
        BTreeEntry { map: self, key }
    }

    pub fn iter<'a>(&'a self) -> BTreeIter<'a, K, V> {
        BTreeIter { map: self, index: 0usize }
    }

    /// Entries whose keys fall inside `range`, in ascending key order.
    /// Panics as std does when the range starts after it ends, or when both
    /// ends exclude the same key.
    pub fn range<'a, Q: ?Sized + super::cmp::Ord, R: super::ops::RangeBounds<Q>>(
        &'a self,
        range: R,
    ) -> BTreeRange<'a, K, V>
    where
        K: super::borrow::Borrow<Q>,
    {
        // An untouched empty map has no search domain or endpoint evaluation.
        if self.keys.capacity() == 0usize {
            return BTreeRange { map: self, front: 0usize, back: 0usize };
        }
        // Evaluate user-defined bounds exactly once, in start/end order.
        let start_bound = range.start_bound();
        let end_bound = range.end_bound();
        match start_bound {
            super::ops::Bound::Included(start) => match end_bound {
                super::ops::Bound::Included(end) | super::ops::Bound::Excluded(end) => {
                    match start.cmp(end) {
                        super::cmp::Ordering::Greater => {
                            panic!("range start is greater than range end in BTreeMap")
                        }
                        _ => {}
                    }
                }
                super::ops::Bound::Unbounded => {}
            },
            super::ops::Bound::Excluded(start) => match end_bound {
                super::ops::Bound::Included(end) => match start.cmp(end) {
                    super::cmp::Ordering::Greater => {
                        panic!("range start is greater than range end in BTreeMap")
                    }
                    _ => {}
                },
                super::ops::Bound::Excluded(end) => match start.cmp(end) {
                    super::cmp::Ordering::Greater => {
                        panic!("range start is greater than range end in BTreeMap")
                    }
                    super::cmp::Ordering::Equal => {
                        panic!("range start and end are equal and excluded in BTreeMap")
                    }
                    super::cmp::Ordering::Less => {}
                },
                super::ops::Bound::Unbounded => {}
            },
            super::ops::Bound::Unbounded => {}
        }
        // First index at or after the start bound.
        let mut front = 0usize;
        while front < self.keys.len() {
            let key: &Q = self.keys.get_ref(front).borrow();
            let inside = match start_bound {
                super::ops::Bound::Included(start) => match key.cmp(start) {
                    super::cmp::Ordering::Less => false,
                    _ => true,
                },
                super::ops::Bound::Excluded(start) => match key.cmp(start) {
                    super::cmp::Ordering::Greater => true,
                    _ => false,
                },
                super::ops::Bound::Unbounded => true,
            };
            if inside {
                break;
            }
            front += 1usize;
        }
        // One past the last index at or before the end bound.
        let mut back = self.keys.len();
        while back > front {
            let key: &Q = self.keys.get_ref(back - 1usize).borrow();
            let inside = match end_bound {
                super::ops::Bound::Included(end) => match key.cmp(end) {
                    super::cmp::Ordering::Greater => false,
                    _ => true,
                },
                super::ops::Bound::Excluded(end) => match key.cmp(end) {
                    super::cmp::Ordering::Less => true,
                    _ => false,
                },
                super::ops::Bound::Unbounded => true,
            };
            if inside {
                break;
            }
            back -= 1usize;
        }
        BTreeRange { map: self, front, back }
    }

    pub fn keys<'a>(&'a self) -> super::slice::Iter<'a, K> {
        self.keys.iter()
    }

    pub fn values<'a>(&'a self) -> super::slice::Iter<'a, V> {
        self.values.iter()
    }
}

pub struct BTreeEntry<'a, K, V> {
    map: &'a mut BTreeMap<K, V>,
    key: K,
}

impl<'a, K: super::cmp::Ord, V> BTreeEntry<'a, K, V> {
    pub fn or_insert_with<F: FnOnce() -> V>(self, default: F) -> &'a mut V {
        let map = self.map;
        let index = match map.search(&self.key) {
            super::result::Result::Ok(i) => i,
            super::result::Result::Err(i) => {
                let value = default();
                map.keys.insert(i, self.key);
                map.values.insert(i, value);
                i
            }
        };
        match map.values.get_mut(index) {
            Option::Some(v) => v,
            Option::None => panic!("entry slot missing"),
        }
    }

    pub fn or_insert_with_key<F: FnOnce(&K) -> V>(self, default: F) -> &'a mut V {
        let map = self.map;
        let index = match map.search(&self.key) {
            super::result::Result::Ok(i) => i,
            super::result::Result::Err(i) => {
                let value = default(&self.key);
                map.keys.insert(i, self.key);
                map.values.insert(i, value);
                i
            }
        };
        match map.values.get_mut(index) {
            Option::Some(v) => v,
            Option::None => panic!("entry slot missing"),
        }
    }

    pub fn or_insert(self, default: V) -> &'a mut V {
        let map = self.map;
        let index = match map.search(&self.key) {
            super::result::Result::Ok(i) => i,
            super::result::Result::Err(i) => {
                map.keys.insert(i, self.key);
                map.values.insert(i, default);
                i
            }
        };
        match map.values.get_mut(index) {
            Option::Some(v) => v,
            Option::None => panic!("entry slot missing"),
        }
    }

    /// Runs `f` on the value when the key is present, then hands the entry
    /// back so an `or_insert` family call can follow.
    pub fn and_modify<F: FnOnce(&mut V)>(self, f: F) -> BTreeEntry<'a, K, V> {
        let map = self.map;
        match map.search(&self.key) {
            super::result::Result::Ok(i) => match map.values.get_mut(i) {
                Option::Some(v) => f(v),
                Option::None => panic!("entry slot missing"),
            },
            super::result::Result::Err(_) => {}
        }
        BTreeEntry { map, key: self.key }
    }
}

impl<'a, K: super::cmp::Ord, V: super::default::Default> BTreeEntry<'a, K, V> {
    pub fn or_default(self) -> &'a mut V {
        let map = self.map;
        let index = match map.search(&self.key) {
            super::result::Result::Ok(i) => i,
            super::result::Result::Err(i) => {
                let value = V::default();
                map.keys.insert(i, self.key);
                map.values.insert(i, value);
                i
            }
        };
        match map.values.get_mut(index) {
            Option::Some(v) => v,
            Option::None => panic!("entry slot missing"),
        }
    }
}

pub struct BTreeIter<'a, K, V> {
    map: &'a BTreeMap<K, V>,
    index: usize,
}

impl<'a, K, V> super::iter::Iterator for BTreeIter<'a, K, V> {
    type Item = (&'a K, &'a V);
    fn next(&mut self) -> Option<(&'a K, &'a V)> {
        if self.index < self.map.keys.len() {
            let i = self.index;
            self.index += 1usize;
            Option::Some((self.map.keys.get_ref(i), self.map.values.get_ref(i)))
        } else {
            Option::None
        }
    }
}

impl<'a, K: super::cmp::Ord, V> super::iter::IntoIterator for &'a BTreeMap<K, V> {
    type Item = (&'a K, &'a V);
    type IntoIter = BTreeIter<'a, K, V>;
    fn into_iter(self) -> BTreeIter<'a, K, V> {
        self.iter()
    }
}

/// The entries of a `BTreeMap` between two key bounds: indices
/// `front..back` of the map's sorted key vector.
pub struct BTreeRange<'a, K, V> {
    map: &'a BTreeMap<K, V>,
    front: usize,
    back: usize,
}

impl<'a, K, V> super::iter::Iterator for BTreeRange<'a, K, V> {
    type Item = (&'a K, &'a V);
    fn next(&mut self) -> Option<(&'a K, &'a V)> {
        if self.front < self.back {
            let i = self.front;
            self.front += 1usize;
            Option::Some((self.map.keys.get_ref(i), self.map.values.get_ref(i)))
        } else {
            Option::None
        }
    }
}

impl<'a, K, V> super::iter::DoubleEndedIterator for BTreeRange<'a, K, V> {
    fn next_back(&mut self) -> Option<(&'a K, &'a V)> {
        if self.front < self.back {
            self.back -= 1usize;
            let i = self.back;
            Option::Some((self.map.keys.get_ref(i), self.map.values.get_ref(i)))
        } else {
            Option::None
        }
    }
}

/// Owning HashMap iteration. Order is unspecified, as in std.
pub struct IntoIter<K, V> {
    keys: Vec<K>,
    values: Vec<V>,
}

impl<K, V> super::iter::Iterator for IntoIter<K, V> {
    type Item = (K, V);
    fn next(&mut self) -> Option<(K, V)> {
        match self.keys.pop() {
            Option::Some(k) => match self.values.pop() {
                Option::Some(v) => Option::Some((k, v)),
                Option::None => panic!("hash map value slot missing"),
            },
            Option::None => Option::None,
        }
    }
}

impl<K, V, S> super::iter::IntoIterator for HashMap<K, V, S> {
    type Item = (K, V);
    type IntoIter = IntoIter<K, V>;
    fn into_iter(self) -> IntoIter<K, V> {
        IntoIter { keys: self.keys, values: self.values }
    }
}

impl<K: Hash + Eq + super::borrow::Borrow<Q>, Q: ?Sized + Hash + Eq, V, S: BuildHasher> super::ops::Index<&Q>
    for HashMap<K, V, S>
{
    type Output = V;
    fn index(&self, key: &Q) -> &V {
        match self.find(key) {
            Option::Some(i) => self.values.get_ref(i),
            Option::None => panic!("key not found in HashMap"),
        }
    }
}

impl<K: super::cmp::Ord, V> super::ops::Index<&K> for BTreeMap<K, V> {
    type Output = V;
    fn index(&self, key: &K) -> &V {
        match self.search(key) {
            super::result::Result::Ok(i) => self.values.get_ref(i),
            super::result::Result::Err(_) => panic!("key not found in BTreeMap"),
        }
    }
}

pub use super::sets::{BTreeSet, BinaryHeap, HashSet, VecDeque};

impl<K, V, S: super::default::Default> super::default::Default for HashMap<K, V, S> {
    fn default() -> HashMap<K, V, S> {
        HashMap::with_hasher(S::default())
    }
}

impl<K: Clone, V: Clone> Clone for BTreeMap<K, V> {
    fn clone(&self) -> BTreeMap<K, V> {
        BTreeMap { keys: self.keys.clone(), values: self.values.clone() }
    }
}

/// Equal length, then entries pairwise in ascending key order (key before
/// value), stopping at the first difference, as std's iterator comparison.
impl<K: super::cmp::PartialEq, V: super::cmp::PartialEq> super::cmp::PartialEq for BTreeMap<K, V> {
    fn eq(&self, other: &BTreeMap<K, V>) -> bool {
        if self.keys.len() != other.keys.len() {
            return false;
        }
        let mut i = 0usize;
        while i < self.keys.len() {
            if !self.keys.get_ref(i).eq(other.keys.get_ref(i))
                || !self.values.get_ref(i).eq(other.values.get_ref(i))
            {
                return false;
            }
            i += 1usize;
        }
        true
    }
}

impl<K: super::cmp::Eq, V: super::cmp::Eq> super::cmp::Eq for BTreeMap<K, V> {}

impl<K: Clone, V: Clone, S: Clone> Clone for HashMap<K, V, S> {
    fn clone(&self) -> HashMap<K, V, S> {
        HashMap {
            hashes: self.hashes.clone(),
            keys: self.keys.clone(),
            values: self.values.clone(),
            hash_builder: self.hash_builder.clone(),
        }
    }
}

/// Equal length and every entry of `self` found in `other` (by `other`'s
/// hasher) with an equal value, as std.
impl<K: Hash + Eq, V: super::cmp::PartialEq, S: BuildHasher> super::cmp::PartialEq
    for HashMap<K, V, S>
{
    fn eq(&self, other: &HashMap<K, V, S>) -> bool {
        if self.keys.len() != other.keys.len() {
            return false;
        }
        let mut i = 0usize;
        while i < self.keys.len() {
            match other.find_key(self.keys.get_ref(i)) {
                Option::Some(j) => {
                    if !self.values.get_ref(i).eq(other.values.get_ref(j)) {
                        return false;
                    }
                }
                Option::None => return false,
            }
            i += 1usize;
        }
        true
    }
}

impl<K: Hash + Eq, V: super::cmp::Eq, S: BuildHasher> super::cmp::Eq for HashMap<K, V, S> {}

impl<K, V> super::default::Default for BTreeMap<K, V> {
    fn default() -> BTreeMap<K, V> {
        BTreeMap { keys: Vec::new(), values: Vec::new() }
    }
}

/// `{key: value, ..}` in ascending key order, as std prints it.
impl<K: super::fmt::Debug, V: super::fmt::Debug> super::fmt::Debug for BTreeMap<K, V> {
    fn fmt(&self, f: &mut super::fmt::Formatter) -> super::fmt::Result {
        let mut map = f.debug_map();
        let mut i = 0usize;
        while i < self.keys.len() {
            map = map.entry(self.keys.get_ref(i), self.values.get_ref(i));
            i += 1usize;
        }
        map.finish()
    }
}

/// `{key: value, ..}`. Entry order is unspecified, as in std.
impl<K: super::fmt::Debug, V: super::fmt::Debug, S> super::fmt::Debug for HashMap<K, V, S> {
    fn fmt(&self, f: &mut super::fmt::Formatter) -> super::fmt::Result {
        let mut map = f.debug_map();
        let mut i = 0usize;
        while i < self.keys.len() {
            map = map.entry(self.keys.get_ref(i), self.values.get_ref(i));
            i += 1usize;
        }
        map.finish()
    }
}
