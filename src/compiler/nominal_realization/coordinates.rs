//! Layout-free closed type coordinates for nominal realization requests.
//! Decoding grants no metadata, field layout or trait membership authority.
use super::*;

const INTS: [ir::IntTy; 12] = [
    ir::IntTy::I8,
    ir::IntTy::I16,
    ir::IntTy::I32,
    ir::IntTy::I64,
    ir::IntTy::I128,
    ir::IntTy::Isize,
    ir::IntTy::U8,
    ir::IntTy::U16,
    ir::IntTy::U32,
    ir::IntTy::U64,
    ir::IntTy::U128,
    ir::IntTy::Usize,
];

#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) enum Term {
    Int(ir::IntTy),
    Float(ir::FloatTy),
    Bool,
    Unit,
    Never,
    Char,
    Str,
    Ref(ir::BorrowKind, Box<Term>),
    RawPtr(ir::BorrowKind, Box<Term>),
    Tuple(Vec<Term>),
    Array(Box<Term>, usize),
    Slice(Box<Term>),
    Nominal(ir::DeclId, Vec<Term>),
}
impl Term {
    pub(super) fn contains_nominal(&self) -> bool {
        match self {
            Self::Nominal(..) => true,
            Self::Ref(_, t) | Self::RawPtr(_, t) | Self::Array(t, _) | Self::Slice(t) => {
                t.contains_nominal()
            }
            Self::Tuple(ts) => ts.iter().any(Self::contains_nominal),
            _ => false,
        }
    }

    pub(super) fn from_ty(ty: &ir::Ty, b: &ir::PlanBudget, depth: usize) -> Result<Self, Error> {
        b.check_depth(depth).map_err(Error::new)?;
        b.charge_work(1).map_err(Error::new)?;
        Ok(match ty {
            ir::Ty::Int(n) => Self::Int(*n),
            ir::Ty::Float(n) => Self::Float(*n),
            ir::Ty::Bool => Self::Bool,
            ir::Ty::Unit => Self::Unit,
            ir::Ty::Never => Self::Never,
            ir::Ty::Char => Self::Char,
            ir::Ty::Str => Self::Str,
            ir::Ty::Ref { kind, referent } => {
                Self::Ref(*kind, Box::new(Self::from_ty(referent, b, depth + 1)?))
            }
            ir::Ty::RawPtr { kind, pointee } => {
                Self::RawPtr(*kind, Box::new(Self::from_ty(pointee, b, depth + 1)?))
            }
            ir::Ty::Slice { element } => {
                Self::Slice(Box::new(Self::from_ty(element, b, depth + 1)?))
            }
            ir::Ty::Array { element, len } => {
                Self::Array(Box::new(Self::from_ty(element, b, depth + 1)?), *len)
            }
            ir::Ty::Tuple(values) => Self::Tuple(Self::from_types(values, b, depth + 1)?),
            ir::Ty::Struct(n) => Self::Nominal(
                n.def().source_decl,
                Self::from_types(&n.def().type_args, b, depth + 1)?,
            ),
            ir::Ty::Enum(n) => Self::Nominal(
                n.def().source_decl,
                Self::from_types(&n.def().type_args, b, depth + 1)?,
            ),
            ir::Ty::Param(_) | ir::Ty::AssociatedType(_) => {
                return Err(Error::new(
                    "nominal realization key requires normalized closed actuals",
                ));
            }
            ir::Ty::Closure(_) => {
                return Err(Error::new(
                    "unsupported: nominal realization requires genuine early closure metadata",
                ));
            }
            ir::Ty::Opaque(_) => {
                return Err(Error::new(
                    "unsupported: opaque nominal realization coordinate",
                ));
            }
            ir::Ty::Dyn(_) => {
                return Err(Error::new(
                    "unsupported: dynamic nominal realization coordinate",
                ));
            }
        })
    }
    fn from_types(types: &[ir::Ty], b: &ir::PlanBudget, d: usize) -> Result<Vec<Self>, Error> {
        b.retain_bytes(
            types
                .len()
                .checked_mul(std::mem::size_of::<Self>())
                .ok_or_else(|| Error::new("nominal term size overflow"))?,
        )
        .map_err(Error::new)?;
        let mut out = Vec::new();
        out.try_reserve_exact(types.len())
            .map_err(|_| Error::new("nominal term allocation failed"))?;
        for ty in types {
            out.push(Self::from_ty(ty, b, d)?);
        }
        Ok(out)
    }
    pub(super) fn encode(&self, b: &ir::PlanBudget) -> Result<Vec<u64>, Error> {
        let mut words = Vec::new();
        self.write(&mut words, b, 0)?;
        Ok(words)
    }
    fn write(&self, out: &mut Vec<u64>, b: &ir::PlanBudget, d: usize) -> Result<(), Error> {
        b.check_depth(d).map_err(Error::new)?;
        b.charge_work(1).map_err(Error::new)?;
        b.retain_bytes(32).map_err(Error::new)?;
        out.try_reserve(4)
            .map_err(|_| Error::new("nominal coordinate allocation failed"))?;
        match self {
            Self::Int(n) => out.extend([
                1,
                INTS.iter()
                    .position(|x| x == n)
                    .ok_or_else(|| Error::new("unknown integer nominal coordinate"))?
                    as u64,
            ]),
            Self::Float(n) => out.extend([
                2,
                match n {
                    ir::FloatTy::F32 => 0,
                    ir::FloatTy::F64 => 1,
                },
            ]),
            Self::Bool => out.push(3),
            Self::Unit => out.push(4),
            Self::Never => out.push(5),
            Self::Char => out.push(6),
            Self::Str => out.push(7),
            Self::Ref(k, t) | Self::RawPtr(k, t) => {
                out.extend([
                    if matches!(self, Self::Ref(..)) { 8 } else { 9 },
                    if *k == ir::BorrowKind::Shared { 0 } else { 1 },
                ]);
                t.write(out, b, d + 1)?;
            }
            Self::Tuple(ts) => {
                if ts.is_empty() {
                    return Err(Error::new("noncanonical empty nominal tuple coordinate"));
                }
                out.extend([10, ts.len() as u64]);
                for t in ts {
                    t.write(out, b, d + 1)?;
                }
            }
            Self::Array(t, n) => {
                out.extend([11, *n as u64]);
                t.write(out, b, d + 1)?;
            }
            Self::Slice(t) => {
                out.push(12);
                t.write(out, b, d + 1)?;
            }
            Self::Nominal(id, ts) => {
                out.extend([
                    13,
                    id.snapshot.graph_word(),
                    u64::from(id.index),
                    ts.len() as u64,
                ]);
                for t in ts {
                    t.write(out, b, d + 1)?;
                }
            }
        }
        Ok(())
    }
    pub(super) fn decode(words: &[u64], b: &ir::PlanBudget) -> Result<Self, Error> {
        let mut input = Reader { words, at: 0, b };
        let term = input.term(0)?;
        if input.at != words.len() {
            return Err(Error::new("trailing nominal type coordinates"));
        }
        if term.encode(b)?.as_slice() != words {
            return Err(Error::new("noncanonical nominal type coordinates"));
        }
        Ok(term)
    }
}
struct Reader<'a> {
    words: &'a [u64],
    at: usize,
    b: &'a ir::PlanBudget,
}
impl Reader<'_> {
    fn word(&mut self) -> Result<u64, Error> {
        let value = *self
            .words
            .get(self.at)
            .ok_or_else(|| Error::new("truncated nominal type coordinate"))?;
        self.at += 1;
        Ok(value)
    }
    fn size(&mut self) -> Result<usize, Error> {
        usize::try_from(self.word()?).map_err(|_| Error::new("nominal coordinate index overflow"))
    }
    fn terms(&mut self, d: usize) -> Result<Vec<Term>, Error> {
        let count = self.size()?;
        if count > self.words.len().saturating_sub(self.at) {
            return Err(Error::new("nominal coordinate arity exceeds input"));
        }
        self.b
            .retain_bytes(
                count
                    .checked_mul(std::mem::size_of::<Term>())
                    .ok_or_else(|| Error::new("nominal coordinate storage overflow"))?,
            )
            .map_err(Error::new)?;
        let mut out = Vec::new();
        out.try_reserve_exact(count)
            .map_err(|_| Error::new("nominal coordinate allocation failed"))?;
        for _ in 0..count {
            out.push(self.term(d)?);
        }
        Ok(out)
    }
    fn term(&mut self, d: usize) -> Result<Term, Error> {
        self.b.check_depth(d).map_err(Error::new)?;
        self.b.charge_work(1).map_err(Error::new)?;
        Ok(match self.word()? {
            1 => Term::Int(
                *INTS
                    .get(self.size()?)
                    .ok_or_else(|| Error::new("invalid nominal integer coordinate"))?,
            ),
            2 => Term::Float(match self.word()? {
                0 => ir::FloatTy::F32,
                1 => ir::FloatTy::F64,
                _ => return Err(Error::new("invalid nominal float coordinate")),
            }),
            3 => Term::Bool,
            4 => Term::Unit,
            5 => Term::Never,
            6 => Term::Char,
            7 => Term::Str,
            tag @ (8 | 9) => {
                let kind = match self.word()? {
                    0 => ir::BorrowKind::Shared,
                    1 => ir::BorrowKind::Mut,
                    _ => return Err(Error::new("invalid nominal borrow coordinate")),
                };
                let t = Box::new(self.term(d + 1)?);
                if tag == 8 {
                    Term::Ref(kind, t)
                } else {
                    Term::RawPtr(kind, t)
                }
            }
            10 => {
                let ts = self.terms(d + 1)?;
                if ts.is_empty() {
                    return Err(Error::new("noncanonical empty nominal tuple coordinate"));
                }
                Term::Tuple(ts)
            }
            11 => {
                let len = self.size()?;
                Term::Array(Box::new(self.term(d + 1)?), len)
            }
            12 => Term::Slice(Box::new(self.term(d + 1)?)),
            13 => {
                let snapshot = ir::SnapshotId::from_graph_word(self.word()?);
                let index = u32::try_from(self.word()?)
                    .map_err(|_| Error::new("nominal declaration index overflow"))?;
                Term::Nominal(ir::DeclId { snapshot, index }, self.terms(d + 1)?)
            }
            _ => return Err(Error::new("unknown nominal type coordinate tag")),
        })
    }
}
