//! The arguments of a call as the bytes it is looked up by. They are never read back, so all
//! they have to be is the same for the same values, in every run, and another for others.
//!
//! A std `HashMap` or `HashSet` of `RandomState` gives its entries in an order its seed
//! decides, another in every run, so its entries are written sorted by their bytes. ra-jit
//! refuses to cache a function that sees that order (it reaches what gives the entries of one
//! in order), so a function it caches gives the same for a map whatever the order. The entries
//! of any other map or sequence are written in their order, which can be what the function
//! reads: an `IndexMap`'s is the order they were put in.
//!
//! Each value is written with a byte telling what it is, and each sequence with where it
//! ends, so two values never give the same bytes, even of a type whose `Serialize` writes
//! another shape for another value.

use std::fmt;

use serde::{
    Serialize,
    ser::{self, Serializer},
};

/// The bytes `value` is looked up by, or `None` when its `Serialize` fails.
pub(crate) fn encode<T: Serialize + ?Sized>(value: &T) -> Option<Vec<u8>> {
    let mut out = Vec::new();
    value.serialize(Writer(&mut out)).ok()?;
    Some(out)
}

/// What the bytes of a value begin with.
#[repr(u8)]
enum Tag {
    Bool,
    I8,
    I16,
    I32,
    I64,
    I128,
    U8,
    U16,
    U32,
    U64,
    U128,
    F32,
    F64,
    Char,
    Str,
    Bytes,
    None,
    Some,
    Unit,
    UnitStruct,
    UnitVariant,
    NewtypeStruct,
    NewtypeVariant,
    Seq,
    Tuple,
    TupleStruct,
    TupleVariant,
    Map,
    Struct,
    StructVariant,
}

/// Each element of a sequence follows `MORE`, and `END` follows the last.
const MORE: u8 = 1;
const END: u8 = 0;

struct Writer<'a>(&'a mut Vec<u8>);

impl Writer<'_> {
    fn tag(&mut self, tag: Tag) {
        self.0.push(tag as u8);
    }

    fn len(&mut self, len: usize) {
        self.0.extend_from_slice(&(len as u64).to_le_bytes());
    }

    fn element<T: Serialize + ?Sized>(&mut self, value: &T) -> Result<(), Error> {
        self.0.push(MORE);
        value.serialize(Writer(&mut *self.0))
    }

    /// The entries of a seeded map or set, each written alone, sorted.
    fn sorted(&mut self, tag: Tag, mut entries: Vec<Vec<u8>>) {
        entries.sort_unstable();
        self.tag(tag);
        for entry in entries {
            self.0.push(MORE);
            self.0.extend_from_slice(&entry);
        }
        self.0.push(END);
    }
}

/// Whether `I`, what serde writes a map or set from, is std's `HashMap` or `HashSet` of
/// `RandomState`. Its name says: rustc's and ra-jit's leave out the default hasher, and name
/// any other.
fn seeded<I: ?Sized>() -> bool {
    let name = std::any::type_name::<I>().trim_start_matches('&');
    let (args, hasher) =
        if let Some(args) = name.strip_prefix("std::collections::hash::map::HashMap<") {
            (args, 2)
        } else if let Some(args) = name.strip_prefix("std::collections::hash::set::HashSet<") {
            (args, 1)
        } else {
            return false;
        };
    let Some(args) = args.strip_suffix('>') else { return false };
    let Some(args) = top_level(args) else { return false };
    match args.get(hasher) {
        None => args.len() == hasher,
        Some(it) => *it == "std::hash::random::RandomState",
    }
}

/// The generic arguments in `args`, split at their commas, or `None` when the brackets in it
/// don't match.
fn top_level(args: &str) -> Option<Vec<&str>> {
    let mut depth = 0usize;
    let mut out = Vec::new();
    let mut start = 0;
    let bytes = args.as_bytes();
    for (at, &byte) in bytes.iter().enumerate() {
        match byte {
            b'<' | b'(' | b'[' | b'{' => depth += 1,
            // The arrow of `fn(u8) -> u8`.
            b'>' if at > 0 && bytes[at - 1] == b'-' => {}
            b'>' | b')' | b']' | b'}' => depth = depth.checked_sub(1)?,
            b',' if depth == 0 => {
                out.push(args[start..at].trim());
                start = at + 1;
            }
            _ => {}
        }
    }
    (depth == 0).then(|| {
        out.push(args[start..].trim());
        out
    })
}

#[derive(Debug)]
pub(crate) struct Error(String);

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for Error {}

impl ser::Error for Error {
    fn custom<T: fmt::Display>(msg: T) -> Error {
        Error(msg.to_string())
    }
}

macro_rules! fixed {
    ($($method:ident: $ty:ty => $tag:ident),* $(,)?) => {
        $(fn $method(mut self, value: $ty) -> Result<(), Error> {
            self.tag(Tag::$tag);
            self.0.extend_from_slice(&value.to_le_bytes());
            Ok(())
        })*
    };
}

impl<'a> Serializer for Writer<'a> {
    type Ok = ();
    type Error = Error;
    type SerializeSeq = Writer<'a>;
    type SerializeTuple = Writer<'a>;
    type SerializeTupleStruct = Writer<'a>;
    type SerializeTupleVariant = Writer<'a>;
    type SerializeMap = Writer<'a>;
    type SerializeStruct = Writer<'a>;
    type SerializeStructVariant = Writer<'a>;

    fixed! {
        serialize_i8: i8 => I8,
        serialize_i16: i16 => I16,
        serialize_i32: i32 => I32,
        serialize_i64: i64 => I64,
        serialize_i128: i128 => I128,
        serialize_u8: u8 => U8,
        serialize_u16: u16 => U16,
        serialize_u32: u32 => U32,
        serialize_u64: u64 => U64,
        serialize_u128: u128 => U128,
        serialize_f32: f32 => F32,
        serialize_f64: f64 => F64,
    }

    fn serialize_bool(mut self, value: bool) -> Result<(), Error> {
        self.tag(Tag::Bool);
        self.0.push(value as u8);
        Ok(())
    }

    fn serialize_char(mut self, value: char) -> Result<(), Error> {
        self.tag(Tag::Char);
        self.0.extend_from_slice(&(value as u32).to_le_bytes());
        Ok(())
    }

    fn serialize_str(mut self, value: &str) -> Result<(), Error> {
        self.tag(Tag::Str);
        self.len(value.len());
        self.0.extend_from_slice(value.as_bytes());
        Ok(())
    }

    fn serialize_bytes(mut self, value: &[u8]) -> Result<(), Error> {
        self.tag(Tag::Bytes);
        self.len(value.len());
        self.0.extend_from_slice(value);
        Ok(())
    }

    fn serialize_none(mut self) -> Result<(), Error> {
        self.tag(Tag::None);
        Ok(())
    }

    fn serialize_some<T: Serialize + ?Sized>(mut self, value: &T) -> Result<(), Error> {
        self.tag(Tag::Some);
        value.serialize(self)
    }

    fn serialize_unit(mut self) -> Result<(), Error> {
        self.tag(Tag::Unit);
        Ok(())
    }

    fn serialize_unit_struct(mut self, _: &'static str) -> Result<(), Error> {
        self.tag(Tag::UnitStruct);
        Ok(())
    }

    fn serialize_unit_variant(
        mut self,
        _: &'static str,
        index: u32,
        _: &'static str,
    ) -> Result<(), Error> {
        self.tag(Tag::UnitVariant);
        self.0.extend_from_slice(&index.to_le_bytes());
        Ok(())
    }

    fn serialize_newtype_struct<T: Serialize + ?Sized>(
        mut self,
        _: &'static str,
        value: &T,
    ) -> Result<(), Error> {
        self.tag(Tag::NewtypeStruct);
        value.serialize(self)
    }

    fn serialize_newtype_variant<T: Serialize + ?Sized>(
        mut self,
        _: &'static str,
        index: u32,
        _: &'static str,
        value: &T,
    ) -> Result<(), Error> {
        self.tag(Tag::NewtypeVariant);
        self.0.extend_from_slice(&index.to_le_bytes());
        value.serialize(self)
    }

    fn serialize_seq(mut self, _: Option<usize>) -> Result<Writer<'a>, Error> {
        self.tag(Tag::Seq);
        Ok(self)
    }

    fn serialize_tuple(mut self, _: usize) -> Result<Writer<'a>, Error> {
        self.tag(Tag::Tuple);
        Ok(self)
    }

    fn serialize_tuple_struct(mut self, _: &'static str, _: usize) -> Result<Writer<'a>, Error> {
        self.tag(Tag::TupleStruct);
        Ok(self)
    }

    fn serialize_tuple_variant(
        mut self,
        _: &'static str,
        index: u32,
        _: &'static str,
        _: usize,
    ) -> Result<Writer<'a>, Error> {
        self.tag(Tag::TupleVariant);
        self.0.extend_from_slice(&index.to_le_bytes());
        Ok(self)
    }

    fn serialize_map(mut self, _: Option<usize>) -> Result<Writer<'a>, Error> {
        self.tag(Tag::Map);
        Ok(self)
    }

    fn serialize_struct(mut self, _: &'static str, _: usize) -> Result<Writer<'a>, Error> {
        self.tag(Tag::Struct);
        Ok(self)
    }

    fn serialize_struct_variant(
        mut self,
        _: &'static str,
        index: u32,
        _: &'static str,
        _: usize,
    ) -> Result<Writer<'a>, Error> {
        self.tag(Tag::StructVariant);
        self.0.extend_from_slice(&index.to_le_bytes());
        Ok(self)
    }

    fn collect_seq<I>(mut self, iter: I) -> Result<(), Error>
    where
        I: IntoIterator,
        I::Item: Serialize,
    {
        if !seeded::<I>() {
            let mut seq = self.serialize_seq(None)?;
            for item in iter {
                ser::SerializeSeq::serialize_element(&mut seq, &item)?;
            }
            return ser::SerializeSeq::end(seq);
        }
        let entries = iter
            .into_iter()
            .map(|item| encode(&item).ok_or_else(failed))
            .collect::<Result<_, Error>>()?;
        self.sorted(Tag::Seq, entries);
        Ok(())
    }

    fn collect_map<K, V, I>(mut self, iter: I) -> Result<(), Error>
    where
        K: Serialize,
        V: Serialize,
        I: IntoIterator<Item = (K, V)>,
    {
        if !seeded::<I>() {
            let mut map = self.serialize_map(None)?;
            for (key, value) in iter {
                ser::SerializeMap::serialize_entry(&mut map, &key, &value)?;
            }
            return ser::SerializeMap::end(map);
        }
        let entries = iter
            .into_iter()
            .map(|(key, value)| {
                let mut entry = encode(&key).ok_or_else(failed)?;
                entry.extend(encode(&value).ok_or_else(failed)?);
                Ok(entry)
            })
            .collect::<Result<_, Error>>()?;
        self.sorted(Tag::Map, entries);
        Ok(())
    }

    fn is_human_readable(&self) -> bool {
        false
    }
}

fn failed() -> Error {
    Error("an entry could not be written".to_owned())
}

macro_rules! elements {
    ($($trait:ident :: $method:ident $(($name:ty))?),* $(,)?) => {
        $(impl ser::$trait for Writer<'_> {
            type Ok = ();
            type Error = Error;

            fn $method<T: Serialize + ?Sized>(&mut self, $(_: $name,)? value: &T) -> Result<(), Error> {
                self.element(value)
            }

            fn end(self) -> Result<(), Error> {
                self.0.push(END);
                Ok(())
            }
        })*
    };
}

elements! {
    SerializeSeq::serialize_element,
    SerializeTuple::serialize_element,
    SerializeTupleStruct::serialize_field,
    SerializeTupleVariant::serialize_field,
    SerializeStruct::serialize_field(&'static str),
    SerializeStructVariant::serialize_field(&'static str),
}

impl ser::SerializeMap for Writer<'_> {
    type Ok = ();
    type Error = Error;

    fn serialize_key<T: Serialize + ?Sized>(&mut self, key: &T) -> Result<(), Error> {
        self.element(key)
    }

    fn serialize_value<T: Serialize + ?Sized>(&mut self, value: &T) -> Result<(), Error> {
        value.serialize(Writer(&mut *self.0))
    }

    fn end(self) -> Result<(), Error> {
        self.0.push(END);
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use std::{
        collections::{BTreeMap, HashMap, HashSet, hash_map::RandomState},
        hash::{BuildHasherDefault, DefaultHasher},
    };

    use serde::ser::SerializeMap;

    use super::*;

    #[test]
    fn a_seeded_map_or_set_is_its_entries_whatever_their_order() {
        let forward: HashMap<u32, String> = (0..200).map(|i| (i, i.to_string())).collect();
        let mut backward = HashMap::with_capacity(1000);
        backward.extend((0..200).rev().map(|i| (i, i.to_string())));
        assert_ne!(forward.keys().collect::<Vec<_>>(), backward.keys().collect::<Vec<_>>());
        assert_eq!(encode(&forward), encode(&backward));
        let fewer: HashMap<u32, String> = (0..199).map(|i| (i, i.to_string())).collect();
        assert_ne!(encode(&forward), encode(&fewer));

        let forward: HashSet<u32> = (0..200).collect();
        let backward: HashSet<u32> = (0..200).rev().collect();
        assert_eq!(encode(&(1u8, &forward)), encode(&(1u8, &backward)));
    }

    /// A map whose order is its own, written as serde writes an `IndexMap`.
    struct Ordered(Vec<(u32, u32)>);

    impl Serialize for Ordered {
        fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
            let mut map = serializer.serialize_map(Some(self.0.len()))?;
            for (key, value) in &self.0 {
                map.serialize_entry(key, value)?;
            }
            map.end()
        }
    }

    #[test]
    fn any_other_map_keeps_its_order() {
        assert_ne!(encode(&Ordered(vec![(1, 2), (3, 4)])), encode(&Ordered(vec![(3, 4), (1, 2)])));
        // A fixed hasher orders the entries by what was done to the map, the same in every run.
        let fixed = |capacity| {
            let mut map =
                HashMap::<u32, u32, BuildHasherDefault<DefaultHasher>>::with_capacity_and_hasher(
                    capacity,
                    Default::default(),
                );
            map.extend((0..200).map(|it| (it, it)));
            map
        };
        let (small, large) = (fixed(0), fixed(1000));
        assert_ne!(small.keys().collect::<Vec<_>>(), large.keys().collect::<Vec<_>>());
        assert_ne!(encode(&small), encode(&large));
        assert!(!seeded::<&HashMap<u32, u32, BuildHasherDefault<DefaultHasher>>>());
        assert!(!seeded::<&BTreeMap<u32, u32>>());
        assert!(!seeded::<&Vec<(u32, u32)>>());
    }

    #[test]
    fn the_seeded_ones_are_known_by_their_names() {
        assert!(seeded::<&HashMap<u32, Vec<(u8, String)>>>());
        assert!(seeded::<&HashMap<u32, fn(u8) -> u8>>());
        assert!(seeded::<&HashMap<u32, u32, RandomState>>());
        assert!(seeded::<&HashSet<HashMap<u8, u8>>>());
        assert!(!seeded::<&HashSet<u8, BuildHasherDefault<DefaultHasher>>>());
    }

    #[test]
    fn values_of_other_shapes_have_other_bytes() {
        assert_ne!(encode(&Some(())), encode(&None::<()>));
        assert_ne!(encode(&(1u16,)), encode(&("\u{1}",)));
        assert_ne!(encode(&vec![vec![1u8], vec![]]), encode(&vec![vec![], vec![1u8]]));
    }
}
