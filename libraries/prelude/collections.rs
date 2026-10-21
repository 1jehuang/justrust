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
