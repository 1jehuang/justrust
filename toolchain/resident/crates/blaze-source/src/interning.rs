use rustc_hash::FxHashSet;
use std::borrow::Borrow;
use std::cmp::Ordering;
use std::fmt;
use std::hash::{Hash, Hasher};
use std::ops::Deref;
use std::ptr;
use std::sync::{Mutex, OnceLock};

static STRING_CACHE: OnceLock<Mutex<FxHashSet<&'static str>>> = OnceLock::new();

/// A string kept once for the life of the process, so that names compare by pointer and copy
/// for free. Package, feature and dependency names are what the resolver compares most.
#[derive(Clone, Copy)]
pub struct InternedString {
    inner: &'static str,
}

impl serde::Serialize for InternedString {
    fn serialize<S: serde::Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        s.serialize_str(self.inner)
    }
}

impl InternedString {
    pub fn new(s: &str) -> InternedString {
        let mut cache = STRING_CACHE.get_or_init(Default::default).lock().unwrap();
        let inner = match cache.get(s) {
            Some(&s) => s,
            None => {
                let s: &'static str = Box::leak(s.to_owned().into_boxed_str());
                cache.insert(s);
                s
            }
        };
        InternedString { inner }
    }

    pub fn as_str(&self) -> &'static str {
        self.inner
    }
}

impl<'a> From<&'a str> for InternedString {
    fn from(s: &'a str) -> Self {
        InternedString::new(s)
    }
}

impl From<&String> for InternedString {
    fn from(s: &String) -> Self {
        InternedString::new(s)
    }
}

impl From<String> for InternedString {
    fn from(s: String) -> Self {
        InternedString::new(&s)
    }
}

impl PartialEq for InternedString {
    fn eq(&self, other: &InternedString) -> bool {
        ptr::eq(self.inner, other.inner)
    }
}

impl Eq for InternedString {}

impl PartialEq<str> for InternedString {
    fn eq(&self, other: &str) -> bool {
        self.inner == other
    }
}

impl PartialEq<&str> for InternedString {
    fn eq(&self, other: &&str) -> bool {
        self.inner == *other
    }
}

impl Hash for InternedString {
    // Hashed as the string so that a `FxHashMap<InternedString, _>` can be read with a `&str`.
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.inner.hash(state);
    }
}

impl Ord for InternedString {
    fn cmp(&self, other: &InternedString) -> Ordering {
        self.inner.cmp(other.inner)
    }
}

impl PartialOrd for InternedString {
    fn partial_cmp(&self, other: &InternedString) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

impl Deref for InternedString {
    type Target = str;

    fn deref(&self) -> &'static str {
        self.inner
    }
}

impl AsRef<str> for InternedString {
    fn as_ref(&self) -> &str {
        self.inner
    }
}

impl Borrow<str> for InternedString {
    fn borrow(&self) -> &str {
        self.inner
    }
}

impl fmt::Debug for InternedString {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Debug::fmt(self.inner, f)
    }
}

impl fmt::Display for InternedString {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Display::fmt(self.inner, f)
    }
}

#[cfg(test)]
mod tests {
    use super::InternedString;
    use rustc_hash::FxHashMap;

    #[test]
    fn same_string_same_pointer() {
        let a = InternedString::new("serde");
        let b = InternedString::from(String::from("serde"));
        assert!(std::ptr::eq(a.as_str(), b.as_str()));
        assert_eq!(a, b);
        assert_ne!(a, InternedString::new("serde_json"));
    }

    #[test]
    fn ordered_and_looked_up_as_str() {
        assert!(InternedString::new("a") < InternedString::new("b"));
        let mut map = FxHashMap::default();
        map.insert(InternedString::new("default"), 1);
        assert_eq!(map.get("default"), Some(&1));
    }
}
