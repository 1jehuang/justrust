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
    FnPointer(Vec<Term>, Box<Term>),
    Cell(ir::DeclId, Box<Term>),
    ManuallyDrop(ir::DeclId, Box<Term>),
    PhantomData(ir::DeclId, Box<Term>),
    Array(Box<Term>, usize),
    Slice(Box<Term>),
    Nominal(ir::DeclId, Vec<Term>),
    /// Coordinates only. Metadata is replayed from the parent-published template.
    Closure(ir::DeclId, Vec<Term>),
    /// Zero-sized function item: exact declaration and ordered actuals.
    FnItem(ir::DeclId, Vec<Term>),
    /// Closed trait object: principal, ordered actuals, ordered assignments.
    Dyn(ir::DeclId, Vec<Term>, Vec<(ir::DeclId, Term)>),
    /// Closed const generic argument of a nominal application.
    Const(ir::ConstArgument),
}
impl Term {
    pub(super) fn from_ty(ty: &ir::Ty, b: &ir::PlanBudget, depth: usize) -> Result<Self, Error> {
        b.check_depth(depth).map_err(Error::new)?;
        b.charge_work(1).map_err(Error::new)?;
        Ok(match ty {
            ir::Ty::Int(n) => Self::Int(*n),
            ir::Ty::Float(n) => Self::Float(*n),
            ir::Ty::Bool => Self::Bool,
            ir::Ty::Unit => Self::Unit,
            ir::Ty::Never => Self::Never,
            ir::Ty::Const(value) => {
                value.closed_key().map_err(Error::new)?;
                Self::Const(value.clone())
            }
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
            ir::Ty::Cell(marker) => {
                b.retain_bytes(std::mem::size_of::<Self>())
                    .map_err(Error::new)?;
                Self::Cell(
                    marker.declaration(),
                    Box::new(Self::from_ty(marker.argument(), b, depth + 1)?),
                )
            }
            ir::Ty::ManuallyDrop(marker) => {
                b.retain_bytes(std::mem::size_of::<Self>())
                    .map_err(Error::new)?;
                Self::ManuallyDrop(
                    marker.declaration(),
                    Box::new(Self::from_ty(marker.argument(), b, depth + 1)?),
                )
            }
            ir::Ty::PhantomData(marker) => {
                b.retain_bytes(std::mem::size_of::<Self>())
                    .map_err(Error::new)?;
                Self::PhantomData(
                    marker.declaration(),
                    Box::new(Self::from_ty(marker.argument(), b, depth + 1)?),
                )
            }
            ir::Ty::FnPointer(pointer) => {
                b.retain_bytes(std::mem::size_of::<Self>())
                    .map_err(Error::new)?;
                Self::FnPointer(
                    Self::from_types(&pointer.inputs, b, depth + 1)?,
                    Box::new(Self::from_ty(&pointer.output, b, depth + 1)?),
                )
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
            ir::Ty::DependentArray { .. } | ir::Ty::Param(_) | ir::Ty::AssociatedType(_) => {
                return Err(Error::new(
                    "nominal realization key requires normalized closed actuals",
                ));
            }
            ir::Ty::FnItem(item) => Self::FnItem(
                item.declaration,
                Self::from_types(&item.type_args, b, depth + 1)?,
            ),
            ir::Ty::Closure(closure) => Self::Closure(
                closure.provider(),
                Self::from_types(closure.owner_args(), b, depth + 1)?,
            ),
            ir::Ty::Opaque(_) => {
                return Err(Error::new(
                    "unsupported: opaque nominal realization coordinate",
                ));
            }
            ir::Ty::Dyn(object) if object.is_closed_leaf() => {
                let mut assigned = Vec::new();
                for (item, ty) in object.associated_types.iter() {
                    assigned.push((*item, Self::from_ty(ty, b, depth + 1)?));
                }
                Self::Dyn(
                    object.principal,
                    Self::from_types(&object.args, b, depth + 1)?,
                    assigned,
                )
            }
            ir::Ty::Dyn(_) => {
                return Err(Error::new(
                    "unsupported: generic dynamic nominal realization coordinate",
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
        let mut output = CoordinateOutput::Owned(Vec::new());
        self.write(&mut output, b, 0)?;
        match output {
            CoordinateOutput::Owned(words) => Ok(words),
            CoordinateOutput::Compare { .. } => unreachable!(),
        }
    }
    pub(super) fn matches_encoding(
        &self,
        words: &[u64],
        b: &ir::PlanBudget,
    ) -> Result<bool, Error> {
        let mut output = CoordinateOutput::Compare {
            words,
            at: 0,
            equal: true,
        };
        self.write(&mut output, b, 0)?;
        match output {
            CoordinateOutput::Compare { at, equal, .. } => Ok(equal && at == words.len()),
            CoordinateOutput::Owned(_) => unreachable!(),
        }
    }
    fn write(
        &self,
        out: &mut CoordinateOutput<'_>,
        b: &ir::PlanBudget,
        d: usize,
    ) -> Result<(), Error> {
        b.check_depth(d).map_err(Error::new)?;
        b.charge_work(1).map_err(Error::new)?;
        out.reserve(b)?;
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
            Self::Const(value) => {
                out.push(21);
                out.extend(value.closed_key().map_err(Error::new)?);
            }
            Self::Char => out.push(6),
            Self::Str => out.push(7),
            Self::Ref(k, t) | Self::RawPtr(k, t) => {
                out.extend([
                    if matches!(self, Self::Ref(..)) { 8 } else { 9 },
                    if *k == ir::BorrowKind::Shared { 0 } else { 1 },
                ]);
                t.write(out, b, d + 1)?;
            }
            Self::Cell(declaration, argument) => {
                out.extend([18, declaration.snapshot.graph_word(), declaration.index]);
                argument.write(out, b, d + 1)?;
            }
            Self::ManuallyDrop(declaration, argument) => {
                out.extend([17, declaration.snapshot.graph_word(), declaration.index]);
                argument.write(out, b, d + 1)?;
            }
            Self::PhantomData(declaration, argument) => {
                out.extend([16, declaration.snapshot.graph_word(), declaration.index]);
                argument.write(out, b, d + 1)?;
            }
            Self::FnPointer(inputs, output) => {
                out.extend([15, inputs.len() as u64]);
                for t in inputs {
                    t.write(out, b, d + 1)?;
                }
                output.write(out, b, d + 1)?;
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
                out.extend([13, id.snapshot.graph_word(), id.index, ts.len() as u64]);
                for t in ts {
                    t.write(out, b, d + 1)?;
                }
            }
            Self::Closure(provider, args) => {
                out.extend([
                    19,
                    provider.snapshot.graph_word(),
                    provider.index,
                    args.len() as u64,
                ]);
                for argument in args {
                    argument.write(out, b, d + 1)?;
                }
            }
            Self::FnItem(declaration, args) => {
                out.extend([
                    20,
                    declaration.snapshot.graph_word(),
                    declaration.index,
                    args.len() as u64,
                ]);
                for argument in args {
                    argument.write(out, b, d + 1)?;
                }
            }
            Self::Dyn(id, args, assigned) => {
                out.extend([14, id.snapshot.graph_word(), id.index, args.len() as u64]);
                for t in args {
                    t.write(out, b, d + 1)?;
                }
                out.push(assigned.len() as u64);
                for (item, t) in assigned {
                    out.extend([item.snapshot.graph_word(), item.index]);
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
/// Encoding and borrowed replay share one canonical word traversal. Comparing
/// an existing immutable term never creates a second owned term or word buffer.
enum CoordinateOutput<'a> {
    Owned(Vec<u64>),
    Compare {
        words: &'a [u64],
        at: usize,
        equal: bool,
    },
}
impl CoordinateOutput<'_> {
    fn reserve(&mut self, budget: &ir::PlanBudget) -> Result<(), Error> {
        if let Self::Owned(words) = self {
            budget.retain_bytes(32).map_err(Error::new)?;
            words
                .try_reserve(4)
                .map_err(|_| Error::new("nominal coordinate allocation failed"))?;
        }
        Ok(())
    }
    fn push(&mut self, word: u64) {
        match self {
            Self::Owned(words) => words.push(word),
            Self::Compare { words, at, equal } => {
                *equal &= words.get(*at) == Some(&word);
                if let Some(next) = at.checked_add(1) {
                    *at = next;
                } else {
                    *equal = false;
                }
            }
        }
    }
    fn extend(&mut self, words: impl IntoIterator<Item = u64>) {
        for word in words {
            self.push(word);
        }
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
            21 => Term::Const(
                ir::ConstArgument::from_closed_key([
                    self.word()?,
                    self.word()?,
                    self.word()?,
                    self.word()?,
                ])
                .map_err(Error::new)?,
            ),
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
                let index = self.word()?;
                Term::Nominal(ir::DeclId { snapshot, index }, self.terms(d + 1)?)
            }
            14 => {
                let snapshot = ir::SnapshotId::from_graph_word(self.word()?);
                let index = self.word()?;
                let args = self.terms(d + 1)?;
                let count = self.size()?;
                if count > self.words.len().saturating_sub(self.at) {
                    return Err(Error::new("dynamic coordinate arity exceeds input"));
                }
                let mut assigned = Vec::new();
                for _ in 0..count {
                    let snapshot = ir::SnapshotId::from_graph_word(self.word()?);
                    let item = self.word()?;
                    assigned.push((
                        ir::DeclId {
                            snapshot,
                            index: item,
                        },
                        self.term(d + 1)?,
                    ));
                }
                Term::Dyn(ir::DeclId { snapshot, index }, args, assigned)
            }
            19 => {
                let provider = ir::DeclId {
                    snapshot: ir::SnapshotId::from_graph_word(self.word()?),
                    index: self.word()?,
                };
                Term::Closure(provider, self.terms(d + 1)?)
            }
            20 => {
                let declaration = ir::DeclId {
                    snapshot: ir::SnapshotId::from_graph_word(self.word()?),
                    index: self.word()?,
                };
                Term::FnItem(declaration, self.terms(d + 1)?)
            }
            18 => {
                let declaration = ir::DeclId {
                    snapshot: ir::SnapshotId::from_graph_word(self.word()?),
                    index: self.word()?,
                };
                self.b
                    .retain_bytes(std::mem::size_of::<Term>())
                    .map_err(Error::new)?;
                Term::Cell(declaration, Box::new(self.term(d + 1)?))
            }
            17 => {
                let declaration = ir::DeclId {
                    snapshot: ir::SnapshotId::from_graph_word(self.word()?),
                    index: self.word()?,
                };
                self.b
                    .retain_bytes(std::mem::size_of::<Term>())
                    .map_err(Error::new)?;
                Term::ManuallyDrop(declaration, Box::new(self.term(d + 1)?))
            }
            16 => {
                let declaration = ir::DeclId {
                    snapshot: ir::SnapshotId::from_graph_word(self.word()?),
                    index: self.word()?,
                };
                self.b
                    .retain_bytes(std::mem::size_of::<Term>())
                    .map_err(Error::new)?;
                Term::PhantomData(declaration, Box::new(self.term(d + 1)?))
            }
            15 => {
                let inputs = self.terms(d + 1)?;
                if inputs.len() > 32 {
                    return Err(Error::new("function pointer parameter limit exceeded"));
                }
                self.b
                    .retain_bytes(std::mem::size_of::<Term>())
                    .map_err(Error::new)?;
                Term::FnPointer(inputs, Box::new(self.term(d + 1)?))
            }
            _ => return Err(Error::new("unknown nominal type coordinate tag")),
        })
    }
}
