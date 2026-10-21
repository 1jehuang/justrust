//! Typed node keys for the compiler graph. This is the only module under
//! `src/compiler` allowed to construct a raw engine [`Key`]
//! (`tests/graph_integrity.rs` enforces it). Parameterless keys are only
//! expressible through the engine's sealed [`RootKey`] types: the `check`
//! and `build` roots plus recorded whole-program debt that may only shrink.

#[allow(unused_imports)]
pub(crate) use crate::engine::{
    BuildRoot, CheckRoot, MonoPlanDebtRoot, ParseDebtRoot, ResolveDebtRoot, SignaturesDebtRoot,
};
use crate::engine::{ItemKey, Key};

/// A per-item (declaration, instance or file ordinal) key of node kind `K`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub(crate) struct Item<const K: u32>(pub usize);
impl<const K: u32> ItemKey for Item<K> {
    const KIND: u32 = K;
    fn params(&self) -> (u64, Vec<u64>) {
        (self.0 as u64, Vec::new())
    }
}

impl<const K: u32> Item<K> {
    pub(crate) fn key(&self) -> Key {
        ItemKey::key(self)
    }
}

/// A key of node kind `K` with a head parameter and ordered tail.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
#[allow(dead_code)]
pub(crate) struct Params<const K: u32>(pub u64, pub Vec<u64>);
impl<const K: u32> ItemKey for Params<K> {
    const KIND: u32 = K;
    fn params(&self) -> (u64, Vec<u64>) {
        (self.0, self.1.clone())
    }
}

#[allow(dead_code)]
impl<const K: u32> Params<K> {
    pub(crate) fn key(&self) -> Key {
        ItemKey::key(self)
    }
}

/// Per-item key whose kind is chosen at runtime among per-item kinds.
pub(crate) fn item(kind: u32, index: usize) -> Key {
    params(kind, index as u64, Vec::new())
}

/// Keyed node with runtime kind. The head parameter makes a parameterless
/// key unrepresentable: `params(KIND)` with no head does not compile.
pub(crate) fn params(kind: u32, head: u64, tail: Vec<u64>) -> Key {
    let mut words = Vec::with_capacity(1 + tail.len());
    words.push(head);
    words.extend(tail);
    Key::new(kind, words)
}

/// Keyed node from already-encoded canonical words (a forwarded key or an
/// identity encoder). Empty words are an explicit error, never a
/// whole-program key.
pub(crate) fn words(kind: u32, words: Vec<u64>) -> Result<Key, crate::engine::Error> {
    let mut words = words.into_iter();
    let Some(head) = words.next() else {
        return Err(crate::engine::Error::new(format!(
            "node kind {kind} has a parameterless (whole-program) key; use a RootKey"
        )));
    };
    Ok(params(kind, head, words.collect()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn typed_keys_match_raw_encoding() {
        assert_eq!(CheckRoot.key(), Key::new(super::super::CHECK, vec![]));
        assert_eq!(BuildRoot.key(), Key::new(super::super::BUILD, vec![]));
        assert_eq!(
            SignaturesDebtRoot.key(),
            Key::new(super::super::SIGNATURES, vec![])
        );
        assert_eq!(
            ResolveDebtRoot.key(),
            Key::new(super::super::RESOLVE, vec![])
        );
        assert_eq!(
            MonoPlanDebtRoot.key(),
            Key::new(super::super::MONO_PLAN, vec![])
        );
        assert_eq!(ParseDebtRoot.key(), Key::new(super::super::PARSE, vec![]));
        assert_eq!(Params::<110>(3, vec![1]).key(), Key::new(110, vec![3, 1]));
        assert_eq!(
            Item::<{ super::super::BODY }>(7).key(),
            Key::new(super::super::BODY, vec![7])
        );
        assert_eq!(item(110, 3), Key::new(110, vec![3]));
    }

    #[test]
    fn empty_words_are_rejected() {
        assert!(words(110, vec![]).is_err());
        assert_eq!(words(110, vec![4, 5]).unwrap(), Key::new(110, vec![4, 5]));
    }
}

/// Test-only fixture key with an arbitrary kind and possibly no parameters,
/// for engine fixtures that are not compiler node kinds.
#[cfg(test)]
pub(crate) fn test_key(kind: u32, params: Vec<u64>) -> Key {
    Key::new(kind, params)
}
