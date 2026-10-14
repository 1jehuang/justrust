use std::collections::HashMap;
use std::fmt;

// IFACE
pub struct Store<T> {
    items: HashMap<String, T>,
    pub hits: u64,
}

#[derive(Debug, Clone, PartialEq)]
pub enum Event {
    Added(String),
    Removed { key: String, at: u64 },
}

impl<T: Clone + fmt::Debug> Store<T> {
    pub fn new() -> Self {
        // BODY
        Store { items: HashMap::new(), hits: 0 }
    }

    pub fn insert(&mut self, k: &str, v: T) -> Event {
        // BODY
        self.items.insert(k.to_owned(), v);
        Event::Added(k.to_owned())
    }

    pub fn get(&mut self, k: &str) -> Option<T> {
        // BODY
        let r = self.items.get(k).cloned();
        if r.is_some() {
            self.hits += 1;
        }
        r
    }

    #[inline]
    pub fn len(&self) -> usize {
        self.items.len()
    }

    pub fn generic_map<U, F: Fn(&T) -> U>(&self, f: F) -> Vec<U> {
        self.items.values().map(f).collect()
    }
}

pub fn describe(e: &Event) -> String {
    // BODY
    match e {
        Event::Added(k) => format!("added {k}"),
        Event::Removed { key, at } => format!("removed {key} at {at}"),
    }
}

pub const LIMIT: usize = 64;

pub fn run() -> u64 {
    // BODY
    let mut s: Store<u64> = Store::new();
    for i in 0..10u64 {
        s.insert(&format!("k{i}"), i);
    }
    let _ = describe(&Event::Removed { key: "k".into(), at: 3 });
    s.hits
}

// IFACE
pub trait Visitor {
    fn visit(&mut self, e: &Event) -> bool;
}
