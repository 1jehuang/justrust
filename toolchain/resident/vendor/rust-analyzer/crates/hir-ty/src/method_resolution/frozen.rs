//! The impls of a crate written as bytes and read back instead of found (frozen dependency
//! artifacts, ra-jit's `docs/glancer.md`, item 1), with `hir_def`'s frozen encoding.
//!
//! The bytes are two parts, each read on its own: the crate's [`TraitImpls`] and its
//! [`InherentImpls`]. As with a frozen `DefMap`, what is read must equal what finding the impls
//! gives: [`Mode::Check`] finds them and [`check`] records a difference, which is a bug.

use std::sync::Arc as StdArc;

use base_db::{Crate, SourceDatabase};
use hir_def::{
    ModuleDefId,
    nameres::frozen::{Dec, Enc, Frozen, Mode, record_difference},
};
use rustc_hash::FxHashMap;
use rustc_type_ir::{FloatTy, IntTy, Mutability, UintTy, fast_reject::SimplifiedType as S};

use super::{InherentImpls, OneTraitImpls, TraitImpls};
use crate::{
    db::HirDatabase,
    next_solver::{SimplifiedType, SolverDefId},
};

type R<T = ()> = Result<T, String>;

/// The crates whose impls are frozen, set by whoever has the bytes.
#[salsa::input(singleton, debug)]
pub struct FrozenImpls {
    #[returns(ref)]
    pub maps: StdArc<FxHashMap<Crate, Frozen>>,
}

/// Has finding the impls of a crate use `maps`.
pub fn set(db: &mut dyn SourceDatabase, maps: FxHashMap<Crate, Frozen>) {
    use salsa::{Durability, Setter};
    let maps = StdArc::new(maps);
    match FrozenImpls::try_get(db) {
        Some(it) => {
            it.set_maps(db).with_durability(Durability::HIGH).to(maps);
        }
        None => {
            _ = FrozenImpls::builder(maps).durability(Durability::HIGH).new(db);
        }
    }
}

fn frozen(db: &dyn HirDatabase, krate: Crate) -> Option<Frozen> {
    FrozenImpls::try_get(db)?.maps(db).get(&krate).cloned()
}

/// Whether the impls of `krate` are read from frozen bytes rather than found.
pub fn served(db: &dyn HirDatabase, krate: Crate) -> bool {
    frozen(db, krate).is_some_and(|it| it.mode != Mode::Check)
}

/// Part `which` of frozen bytes: the first is after its length.
fn part(bytes: &[u8], which: usize) -> R<&[u8]> {
    let len: [u8; 4] = bytes.get(..4).and_then(|it| it.try_into().ok()).ok_or("no length")?;
    let first = 4 + u32::from_le_bytes(len) as usize;
    let part = if which == 0 { bytes.get(4..first) } else { bytes.get(first..) };
    part.ok_or_else(|| "a part past the end".to_owned())
}

fn read<'db, T>(
    db: &'db dyn HirDatabase,
    krate: Crate,
    which: usize,
    f: impl FnOnce(&mut Dec<'_>) -> R<T>,
) -> Option<T> {
    let frozen = frozen(db, krate).filter(|it| it.mode != Mode::Check)?;
    let read = || {
        let mut dec = Dec::new_checked(db, krate, part(&frozen.bytes, which)?)?;
        let it = f(&mut dec)?;
        dec.finish()?;
        Ok::<_, String>(it)
    };
    Some(read().unwrap_or_else(|why| panic!("frozen impls do not read: {why}")))
}

pub(super) fn trait_impls<'db>(db: &'db dyn HirDatabase, krate: Crate) -> Option<TraitImpls<'db>> {
    read(db, krate, 0, |dec| {
        let n = dec.len()?;
        let mut map = FxHashMap::default();
        map.reserve(n);
        for _ in 0..n {
            let ModuleDefId::TraitId(trait_) = dec.def()? else {
                return Err("impls of no trait".to_owned());
            };
            let non_blanket_impls = dec.many(|dec| {
                let self_ty = read_simplified(dec)?;
                let impls = dec.many(|dec| dec.impl_id())?.into_boxed_slice();
                let derived = dec.many(|dec| dec.builtin_derive_impl())?.into_boxed_slice();
                Ok((self_ty, (impls, derived)))
            })?;
            let blanket_impls = dec.many(|dec| dec.impl_id())?.into_boxed_slice();
            let non_blanket_impls = non_blanket_impls.into_iter().collect();
            map.insert(trait_, OneTraitImpls { non_blanket_impls, blanket_impls });
        }
        Ok(TraitImpls { map })
    })
}

pub(super) fn inherent_impls<'db>(
    db: &'db dyn HirDatabase,
    krate: Crate,
) -> Option<InherentImpls<'db>> {
    read(db, krate, 1, |dec| {
        let map = dec.many(|dec| {
            let self_ty = read_simplified(dec)?;
            Ok((self_ty, dec.many(|dec| dec.impl_id())?.into_boxed_slice()))
        })?;
        Ok(InherentImpls { map: map.into_iter().collect() })
    })
}

/// The bytes of the impls of `krate` as they are now: found, or read in [`Mode::Serve`] and
/// [`Mode::Verify`].
pub fn encode(db: &dyn HirDatabase, krate: Crate) -> R<Vec<u8>> {
    let mut enc = Enc::new(db, krate);
    let TraitImpls { map } = &**TraitImpls::for_crate(db, krate);
    let map = enc.sorted(map.iter(), |enc, it| enc.def(ModuleDefId::TraitId(**it)))?;
    enc.usize(map.len())?;
    for (&trait_, one) in map {
        enc.def(ModuleDefId::TraitId(trait_))?;
        let OneTraitImpls { non_blanket_impls, blanket_impls } = one;
        let entries = enc.sorted(non_blanket_impls.iter(), |enc, it| write_simplified(enc, it))?;
        enc.usize(entries.len())?;
        for (self_ty, (impls, derived)) in entries {
            write_simplified(&mut enc, self_ty)?;
            enc.usize(impls.len())?;
            for &it in impls.iter() {
                enc.impl_id(it)?;
            }
            enc.usize(derived.len())?;
            for &it in derived.iter() {
                enc.builtin_derive_impl(it)?;
            }
        }
        enc.usize(blanket_impls.len())?;
        for &it in blanket_impls.iter() {
            enc.impl_id(it)?;
        }
    }
    let traits = enc.finish()?;

    let mut enc = Enc::new(db, krate);
    let InherentImpls { map } = InherentImpls::for_crate(db, krate);
    let map = enc.sorted(map.iter(), |enc, it| write_simplified(enc, it))?;
    enc.usize(map.len())?;
    for (self_ty, impls) in map {
        write_simplified(&mut enc, self_ty)?;
        enc.usize(impls.len())?;
        for &it in impls.iter() {
            enc.impl_id(it)?;
        }
    }
    let inherent = enc.finish()?;

    let len = u32::try_from(traits.len()).map_err(|_| "impls over 4 GiB")?;
    let mut out = len.to_le_bytes().to_vec();
    out.extend_from_slice(&traits);
    out.extend_from_slice(&inherent);
    Ok(out)
}

/// Records a difference between the frozen impls of `krate` and the live ones in
/// [`Mode::Check`], or what was read of them in [`Mode::Verify`].
pub fn check(db: &dyn HirDatabase, krate: Crate) {
    let Some(frozen) = frozen(db, krate) else { return };
    let what = match frozen.mode {
        Mode::Serve => return,
        Mode::Check => "the live ones",
        Mode::Verify => "what was read of them",
    };
    match encode(db, krate) {
        Ok(written) if *written == *frozen.bytes => {}
        Ok(written) => record_difference(db, krate, "impls", what, &written, &frozen.bytes),
        Err(why) => {
            let what = format!("{what}, which cannot be written ({why})");
            record_difference(db, krate, "impls", &what, &[], &[])
        }
    }
}

const INTS: [IntTy; 6] = [IntTy::Isize, IntTy::I8, IntTy::I16, IntTy::I32, IntTy::I64, IntTy::I128];
const UINTS: [UintTy; 6] =
    [UintTy::Usize, UintTy::U8, UintTy::U16, UintTy::U32, UintTy::U64, UintTy::U128];
const FLOATS: [FloatTy; 4] = [FloatTy::F16, FloatTy::F32, FloatTy::F64, FloatTy::F128];

fn write_def(enc: &mut Enc<'_>, it: &SolverDefId<'_>) -> R {
    match *it {
        SolverDefId::AdtId(it) => {
            enc.u8(0);
            enc.adt(it)
        }
        SolverDefId::TraitId(it) => {
            enc.u8(1);
            enc.def(it.into())
        }
        SolverDefId::TypeAliasId(it) => {
            enc.u8(2);
            enc.def(it.into())
        }
        _ => Err("a type named by neither an ADT, a trait nor a type alias".to_owned()),
    }
}

fn read_def<'db>(dec: &mut Dec<'_>) -> R<SolverDefId<'db>> {
    Ok(match dec.u8()? {
        0 => SolverDefId::AdtId(dec.adt()?),
        _ => match dec.def()? {
            ModuleDefId::TraitId(it) => SolverDefId::TraitId(it),
            ModuleDefId::TypeAliasId(it) => SolverDefId::TypeAliasId(it),
            _ => return Err("a type named by an item that names none".to_owned()),
        },
    })
}

fn mutability(it: Mutability) -> u8 {
    match it {
        Mutability::Not => 0,
        Mutability::Mut => 1,
    }
}

fn read_mutability(dec: &mut Dec<'_>) -> R<Mutability> {
    Ok(if dec.u8()? == 0 { Mutability::Not } else { Mutability::Mut })
}

fn write_simplified(enc: &mut Enc<'_>, it: &SimplifiedType<'_>) -> R {
    fn index<T: PartialEq>(all: &[T], it: T) -> u32 {
        all.iter().position(|x| *x == it).unwrap() as u32
    }
    match *it {
        S::Bool => enc.u8(0),
        S::Char => enc.u8(1),
        S::Int(it) => {
            enc.u8(2);
            enc.u32(index(&INTS, it));
        }
        S::Uint(it) => {
            enc.u8(3);
            enc.u32(index(&UINTS, it));
        }
        S::Float(it) => {
            enc.u8(4);
            enc.u32(index(&FLOATS, it));
        }
        S::Adt(ref def) => {
            enc.u8(5);
            write_def(enc, def)?;
        }
        S::Foreign(ref def) => {
            enc.u8(6);
            write_def(enc, def)?;
        }
        S::Str => enc.u8(7),
        S::Array => enc.u8(8),
        S::Slice => enc.u8(9),
        S::Ref(it) => {
            enc.u8(10);
            enc.u8(mutability(it));
        }
        S::Ptr(it) => {
            enc.u8(11);
            enc.u8(mutability(it));
        }
        S::Never => enc.u8(12),
        S::Tuple(n) => {
            enc.u8(13);
            enc.usize(n)?;
        }
        S::MarkerTraitObject => enc.u8(14),
        S::Trait(ref def) => {
            enc.u8(15);
            write_def(enc, def)?;
        }
        S::Function(n) => {
            enc.u8(16);
            enc.usize(n)?;
        }
        S::UnsafeBinder => enc.u8(17),
        S::Placeholder => enc.u8(18),
        S::Error => enc.u8(19),
        S::Closure(_) | S::Coroutine(_) | S::CoroutineWitness(_) => {
            return Err("an impl for a closure or a coroutine".to_owned());
        }
    }
    Ok(())
}

fn read_simplified<'db>(dec: &mut Dec<'_>) -> R<SimplifiedType<'db>> {
    Ok(match dec.u8()? {
        0 => S::Bool,
        1 => S::Char,
        2 => S::Int(dec.index(&INTS)?),
        3 => S::Uint(dec.index(&UINTS)?),
        4 => S::Float(dec.index(&FLOATS)?),
        5 => S::Adt(read_def(dec)?),
        6 => S::Foreign(read_def(dec)?),
        7 => S::Str,
        8 => S::Array,
        9 => S::Slice,
        10 => S::Ref(read_mutability(dec)?),
        11 => S::Ptr(read_mutability(dec)?),
        12 => S::Never,
        13 => S::Tuple(dec.len()?),
        14 => S::MarkerTraitObject,
        15 => S::Trait(read_def(dec)?),
        16 => S::Function(dec.len()?),
        17 => S::UnsafeBinder,
        18 => S::Placeholder,
        19 => S::Error,
        it => return Err(format!("simplified type {it}")),
    })
}
