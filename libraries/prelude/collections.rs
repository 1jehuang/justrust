//! Native std::collections. HashMap<K, V> is ordinary source: a dense entry
//! vector with cached hashes. Iteration order is unspecified, like std's.
use super::hash::{DefaultHasher, Hash, Hasher};
use super::option::Option;
use super::vec::Vec;

fn hash_of<K: Hash>(key: &K) -> u64 {
    let mut state = DefaultHasher::new();
    key.hash(&mut state);
    state.finish()
}

pub struct HashMap<K, V> {
    hashes: Vec<u64>,
    keys: Vec<K>,
    values: Vec<V>,
}

impl<K: Hash + Eq, V> HashMap<K, V> {
    pub fn new() -> HashMap<K, V> {
        HashMap { hashes: Vec::new(), keys: Vec::new(), values: Vec::new() }
    }

    fn find(&self, key: &K) -> Option<usize> {
        let h = hash_of(key);
        let mut i = 0usize;
        while i < self.keys.len() {
            if self.hashes[i] == h && self.keys.get_ref(i).eq(key) {
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

    pub fn contains_key(&self, key: &K) -> bool {
        match self.find(key) {
            Option::Some(_) => true,
            Option::None => false,
        }
    }

    pub fn get(&self, key: &K) -> Option<&V> {
        match self.find(key) {
            Option::Some(i) => Option::Some(self.values.get_ref(i)),
            Option::None => Option::None,
        }
    }

    pub fn get_mut(&mut self, key: &K) -> Option<&mut V> {
        match self.find(key) {
            Option::Some(i) => self.values.get_mut(i),
            Option::None => Option::None,
        }
    }

    pub fn insert(&mut self, key: K, value: V) -> Option<V> {
        match self.find(&key) {
            Option::Some(i) => {
                let old = self.values.swap_remove(i);
                self.values.push(value);
                let last = self.values.len() - 1usize;
                self.values.swap(i, last);
                Option::Some(old)
            }
            Option::None => {
                self.hashes.push(hash_of(&key));
                self.keys.push(key);
                self.values.push(value);
                Option::None
            }
        }
    }

    pub fn remove(&mut self, key: &K) -> Option<V> {
        match self.find(key) {
            Option::Some(i) => {
                let _h = self.hashes.swap_remove(i);
                let _k = self.keys.swap_remove(i);
                Option::Some(self.values.swap_remove(i))
            }
            Option::None => Option::None,
        }
    }

    pub fn entry<'a>(&'a mut self, key: K) -> Entry<'a, K, V> {
        Entry { map: self, key }
    }

    pub fn iter<'a>(&'a self) -> Iter<'a, K, V> {
        Iter { map: self, index: 0usize }
    }

    pub fn keys<'a>(&'a self) -> super::slice::Iter<'a, K> {
        self.keys.iter()
    }

    pub fn values<'a>(&'a self) -> super::slice::Iter<'a, V> {
        self.values.iter()
    }
}

pub struct Entry<'a, K, V> {
    map: &'a mut HashMap<K, V>,
    key: K,
}

impl<'a, K: Hash + Eq, V> Entry<'a, K, V> {
    pub fn or_insert(self, default: V) -> &'a mut V {
        let map = self.map;
        let index = match map.find(&self.key) {
            Option::Some(i) => i,
            Option::None => {
                map.hashes.push(hash_of(&self.key));
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

impl<'a, K: Hash + Eq, V: super::default::Default> Entry<'a, K, V> {
    pub fn or_default(self) -> &'a mut V {
        self.or_insert(V::default())
    }
}

pub struct Iter<'a, K, V> {
    map: &'a HashMap<K, V>,
    index: usize,
}

impl<'a, K, V> super::iter::Iterator for Iter<'a, K, V> {
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

impl<'a, K, V> super::iter::IntoIterator for Iter<'a, K, V> {
    type Item = (&'a K, &'a V);
    type IntoIter = Iter<'a, K, V>;
    fn into_iter(self) -> Iter<'a, K, V> {
        self
    }
}

impl<'a, K: Hash + Eq, V> super::iter::IntoIterator for &'a HashMap<K, V> {
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
        self.or_insert(V::default())
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

impl<K, V> super::iter::IntoIterator for HashMap<K, V> {
    type Item = (K, V);
    type IntoIter = IntoIter<K, V>;
    fn into_iter(self) -> IntoIter<K, V> {
        IntoIter { keys: self.keys, values: self.values }
    }
}
