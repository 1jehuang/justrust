//! The statics a patch defines that the executable has no copy of, which every patch after
//! defines again under the same name ([`hir_ty_codegen::LASTING_STATIC`]): patches hold all
//! that differs from the base, and a static new since the base is in each. The running program
//! keeps the first copy it was given, with what was written to it since, so that a static
//! that counts or caches (a component's id) goes on doing so across edits.
//!
//! The copy is in none of the patches: each new one is given a place of its own at the top of
//! the room the program set aside for patches, below the last, and every patch that refers to
//! it carries its first value. The program writes that the first time it is given one at the
//! place, and never again ([`ra_jit_live::format::Code::lasting`]). So a patch the program
//! never took, as one written over by the next before it read it, is nothing to the next:
//! there is no copy of a static in a patch that the next refers to.

use anyhow::{Context, bail, ensure};
use rustc_hash::FxHashMap;

/// Whether `name`, of a symbol of a patch, is of a static a patch defines anew. A Mach-O
/// name has `_` in front.
pub(crate) fn is_lasting(name: &[u8]) -> bool {
    let name = name.strip_prefix(b"_").unwrap_or(name);
    name.starts_with(hir_ty_codegen::LASTING_STATIC.as_bytes())
}

/// The name of the base's copy of a static that `name`, of a symbol of a patch, says the patch
/// left behind ([`hir_ty_codegen::MOVED_STATIC`]), with the `_` of a Mach-O name in front.
pub(crate) fn left_behind(name: &[u8]) -> Option<Vec<u8>> {
    let prefix = hir_ty_codegen::MOVED_STATIC.as_bytes();
    match name.strip_prefix(b"_").and_then(|it| it.strip_prefix(prefix)) {
        Some(rest) => Some([b"_", rest].concat()),
        None => name.strip_prefix(prefix).map(<[u8]>::to_vec),
    }
}

/// Where a running program has each static that patches defined anew, by its name.
#[derive(Clone, Default)]
pub(crate) struct Lasting {
    at: FxHashMap<Box<[u8]>, Kept>,
    /// The lowest place given to one in the room, which they take from its end down; 0 when
    /// the program takes none there (an older `ra-jit-live`, or a splice's).
    low: u64,
}

#[derive(Clone, Copy)]
struct Kept {
    address: u64,
    /// In the room, where the program writes its first value, rather than in what the
    /// executable loads.
    in_room: bool,
}

/// A static a patch defines anew, where the patch has its own copy.
pub(crate) struct Own {
    pub(crate) name: Box<[u8]>,
    pub(crate) size: u64,
    pub(crate) align: u64,
}

impl Lasting {
    /// Those a splice left in the executable, where it put them.
    pub(crate) fn in_executable(at: impl IntoIterator<Item = (Box<[u8]>, u64)>) -> Lasting {
        let at = at.into_iter().map(|(name, address)| (name, Kept { address, in_room: false }));
        Lasting { at: at.collect(), low: 0 }
    }

    /// For a program whose room ends at `end`, when it takes statics there.
    pub(crate) fn with_room(mut self, end: Option<u64>) -> Lasting {
        self.low = end.unwrap_or(0);
        self
    }

    /// Where the room's statics start: the patches go below.
    pub(crate) fn low(&self) -> Option<u64> {
        (self.low != 0).then_some(self.low)
    }

    pub(crate) fn address(&self, name: &[u8]) -> Option<u64> {
        self.at.get(name).map(|it| it.address)
    }

    /// Gives each of `own` that has no place yet one in the room, when the program has one:
    /// without, the patch's own copy is the program's until the next patch.
    pub(crate) fn place(&mut self, own: &[Own]) -> anyhow::Result<()> {
        if self.low == 0 {
            return Ok(());
        }
        for it in own {
            if self.at.contains_key(&it.name) {
                continue;
            }
            ensure!(it.align.is_power_of_two(), "a static aligned to {}", it.align);
            let address =
                (self.low.checked_sub(it.size).context("no room for a static")?) & !(it.align - 1);
            self.low = address;
            self.at.insert(it.name.clone(), Kept { address, in_room: true });
        }
        Ok(())
    }

    /// Notes where those of `own` are that have no place but the patch's copy, at `copies`.
    pub(crate) fn note(&mut self, copies: impl IntoIterator<Item = (Box<[u8]>, u64)>) {
        for (name, address) in copies {
            self.at.entry(name).or_insert(Kept { address, in_room: false });
        }
    }

    /// The first value of each of `own` that is in the room, from the patch's own copy where
    /// `copy` says the patch laid it out: an offset into `laid`, what the patch was laid out
    /// as from `base`, and `pointers`, where `laid` stores an address, as offsets into it.
    pub(crate) fn first_values(
        &self,
        own: &[Own],
        copy: impl Fn(&[u8]) -> Option<u64>,
        laid: &[u8],
        base: u64,
        pointers: &[u64],
    ) -> anyhow::Result<Vec<ra_jit_live::format::Write>> {
        let mut sorted = pointers.to_vec();
        sorted.sort_unstable();
        let mut values = Vec::new();
        for it in own {
            let Some(kept) = self.at.get(&it.name).filter(|it| it.in_room) else { continue };
            let shown = || String::from_utf8_lossy(&it.name).into_owned();
            let at = copy(&it.name).with_context(|| format!("{} is laid out nowhere", shown()))?;
            let end = at + it.size;
            let bytes = laid.get(at as usize..end as usize).context("a static past the patch")?;
            let from = sorted.partition_point(|it| *it < at);
            let pointers: Vec<u64> =
                sorted[from..].iter().take_while(|it| **it < end).map(|it| it - at).collect();
            // One that points into itself would point to the patch's copy.
            for &pointer in &pointers {
                let field = bytes.get(pointer as usize..pointer as usize + 8);
                let value =
                    u64::from_le_bytes(field.context("a pointer past a static")?.try_into()?);
                if (base + at..base + end).contains(&value) {
                    bail!("the static {} points into itself", shown());
                }
            }
            values.push(ra_jit_live::format::Write {
                address: kept.address,
                bytes: bytes.to_vec(),
                pointers,
            });
        }
        Ok(values)
    }
}

#[cfg(test)]
mod tests {
    use super::{Lasting, Own};

    fn own(name: &str, size: u64, align: u64) -> Own {
        Own { name: name.as_bytes().into(), size, align }
    }

    /// A static keeps its place in every patch after the one it is new in, and the next new
    /// one goes below it, aligned.
    #[test]
    fn a_static_keeps_its_place() {
        let mut lasting = Lasting::default().with_room(Some(0x1_0000));
        lasting.place(&[own("a", 4, 4)]).unwrap();
        assert_eq!(lasting.address(b"a"), Some(0xfffc));
        lasting.place(&[own("a", 4, 4), own("b", 9, 8)]).unwrap();
        assert_eq!(lasting.address(b"a"), Some(0xfffc));
        assert_eq!(lasting.address(b"b"), Some(0xfff0));
        assert_eq!(lasting.low(), Some(0xfff0));
    }

    /// The first value is the patch's copy, with where it stores an address; one in the
    /// executable has none, and one that points into itself is refused.
    #[test]
    fn first_values() {
        let mut lasting =
            Lasting::in_executable([(b"kept".as_slice().into(), 0x500)]).with_room(Some(0x1_0000));
        let statics = [own("kept", 4, 4), own("new", 16, 8)];
        lasting.place(&statics).unwrap();
        let mut laid = vec![0u8; 0x40];
        laid[0x28..0x30].copy_from_slice(&0x9000u64.to_le_bytes());
        let copy = |name: &[u8]| Some(if name == b"new" { 0x20 } else { 0x10 });
        let values = lasting.first_values(&statics, copy, &laid, 0x1000, &[0x28, 0x3]).unwrap();
        assert_eq!(values.len(), 1);
        assert_eq!(values[0].address, 0xfff0);
        assert_eq!(values[0].bytes, laid[0x20..0x30]);
        assert_eq!(values[0].pointers, [8]);
        laid[0x28..0x30].copy_from_slice(&0x1024u64.to_le_bytes());
        assert!(lasting.first_values(&statics, copy, &laid, 0x1000, &[0x28]).is_err());
    }
}
