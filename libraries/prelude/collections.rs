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

impl<'a, K, V> super::iter::IntoIterator for Iter<'a, K, V> {
    type Item = (&'a K, &'a V);
    type IntoIter = Iter<'a, K, V>;
    fn into_iter(self) -> Iter<'a, K, V> {
        self
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

    pub fn contains_key(&self, key: &K) -> bool {
        match self.search(key) {
            super::result::Result::Ok(_) => true,
            super::result::Result::Err(_) => false,
        }
    }

    pub fn get(&self, key: &K) -> Option<&V> {
        match self.search(key) {
            super::result::Result::Ok(i) => Option::Some(self.values.get_ref(i)),
            super::result::Result::Err(_) => Option::None,
        }
    }

    pub fn get_mut(&mut self, key: &K) -> Option<&mut V> {
        match self.search(key) {
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

    pub fn remove(&mut self, key: &K) -> Option<V> {
        match self.search(key) {
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
        if self.index < self.keys.len() {
            let i = self.index;
            self.index += 1usize;
            Option::Some((&self.keys[i], &self.values[i]))
        } else {
            Option::None
        }
    }
}

impl<'a, K, V> super::iter::IntoIterator for BTreeIter<'a, K, V> {
    type Item = (&'a K, &'a V);
    type IntoIter = BTreeIter<'a, K, V>;
    fn into_iter(self) -> BTreeIter<'a, K, V> {
        self
    }
}

impl<'a, K: super::cmp::Ord, V> super::iter::IntoIterator for &'a BTreeMap<K, V> {
    type Item = (&'a K, &'a V);
    type IntoIter = BTreeIter<'a, K, V>;
    fn into_iter(self) -> BTreeIter<'a, K, V> {
        self.iter()
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

impl<K, V> super::default::Default for BTreeMap<K, V> {
    fn default() -> BTreeMap<K, V> {
        BTreeMap { keys: Vec::new(), values: Vec::new() }
    }
}
