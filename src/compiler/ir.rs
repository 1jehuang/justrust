//! Checked scalar, aggregate and nominal bodies shared by analysis and codegen.
//!
//! IDs are local to one immutable function, not persistent cache identities.
//! Allocate a new LocalId for every binding, including shadowing. Names are for
//! diagnostics only. This structured representation is a migration boundary,
//! not yet an ownership-checking CFG or a target layout/ABI representation.
use super::frontend::{BinaryOp, UnaryOp};
use std::hash::{Hash, Hasher};
use std::sync::Arc;
/// Combined explicit loan sites and formal reference-parameter origins.
pub const MAX_LOAN_SITES: usize = 4096;
use std::sync::atomic::{AtomicU64, Ordering};

/// Process-local namespace for one validated declaration environment. IDs must
/// not be persisted as cache identities or reused by another environment.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct SnapshotId(u64);

impl SnapshotId {
    pub fn allocate() -> Result<Self, String> {
        static NEXT: AtomicU64 = AtomicU64::new(0);
        Self::allocate_from(&NEXT)
    }

    fn allocate_from(next: &AtomicU64) -> Result<Self, String> {
        let mut id = next.load(Ordering::Relaxed);
        loop {
            let successor = id
                .checked_add(1)
                .ok_or("nominal type snapshot IDs exhausted")?;
            match next.compare_exchange_weak(id, successor, Ordering::Relaxed, Ordering::Relaxed) {
                Ok(_) => return Ok(Self(id)),
                Err(observed) => id = observed,
            }
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct DefId {
    pub snapshot: SnapshotId,
    pub index: usize,
}

pub const MAX_ENUM_VARIANTS: usize = 4096;
pub const MAX_MATCH_ARMS: usize = 4096;
pub const MAX_ARRAY_LEN: usize = 4096;

#[derive(Clone)]
pub struct EnumTy(Arc<EnumDef>);

impl EnumTy {
    pub(super) fn new(def: EnumDef) -> Self {
        Self(Arc::new(def))
    }

    pub fn def(&self) -> &EnumDef {
        &self.0
    }

    pub fn has_payload(&self) -> bool {
        self.0
            .variants
            .iter()
            .any(|variant| variant.shape != VariantShape::Unit)
    }

    pub(super) fn validate(&self) -> Result<(), String> {
        if self.0.variants.is_empty() || self.0.variants.len() > MAX_ENUM_VARIANTS {
            return Err("enum variant count must be between 1 and 4096".into());
        }
        let mut fields = 0usize;
        for variant in self.0.variants.iter() {
            fields = fields
                .checked_add(variant.fields.len())
                .ok_or("enum payload field limit exceeded")?;
            if fields > MAX_PATTERN_BINDINGS {
                return Err("enum payload field limit exceeded".into());
            }
            if variant.shape == VariantShape::Unit && !variant.fields.is_empty() {
                return Err("unit enum variant cannot have payload fields".into());
            }
        }
        Ok(())
    }
}

impl std::fmt::Debug for EnumTy {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let name = &self.0.name;
        let end = name
            .char_indices()
            .nth(128)
            .map_or(name.len(), |(at, _)| at);
        f.debug_struct("EnumTy")
            .field("id", &self.0.id)
            .field("name", &&name[..end])
            .finish_non_exhaustive()
    }
}

impl PartialEq for EnumTy {
    fn eq(&self, other: &Self) -> bool {
        self.0.id == other.0.id
    }
}
impl Eq for EnumTy {}
impl Hash for EnumTy {
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.0.id.hash(state);
    }
}

#[derive(Clone, Debug)]
pub struct EnumDef {
    pub id: DefId,
    pub name: String,
    pub module: String,
    pub public: bool,
    pub variants: Arc<[VariantDef]>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum VariantShape {
    Unit,
    Tuple,
    Struct,
}

#[derive(Clone, Debug)]
pub struct VariantDef {
    pub name: String,
    pub shape: VariantShape,
    pub fields: Arc<[FieldDef]>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct VariantId {
    pub owner: DefId,
    pub index: usize,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum Projection {
    Field(usize),
    Downcast(VariantId),
    ConstantIndex(usize),
    Deref,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum BorrowKind {
    Shared,
    Mut,
}

/// Checks structural typing, not the runtime proof needed to read a payload.
pub fn projected_ty<'a>(root: &'a Ty, path: &[Projection]) -> Result<&'a Ty, String> {
    if path.len() > MAX_PATTERN_PROJECTION_DEPTH {
        return Err("projection depth exceeds limit 128".into());
    }
    if path.iter().any(|part| matches!(part, Projection::Deref))
        && (!matches!(path.first(), Some(Projection::Deref))
            || path
                .iter()
                .filter(|part| matches!(part, Projection::Deref))
                .count()
                != 1
            || path
                .iter()
                .any(|part| matches!(part, Projection::Downcast(_))))
    {
        return Err("unsupported: dereference must be first and cannot contain downcasts".into());
    }
    let mut ty = root;
    let mut path = path.iter();
    while let Some(projection) = path.next() {
        ty = match *projection {
            Projection::Deref => {
                let Ty::Ref { referent, .. } = ty else {
                    return Err("dereference requires reference type".into());
                };
                referent
            }
            Projection::ConstantIndex(index) => {
                let Ty::Array { element, len } = ty else {
                    return Err("constant index requires array type".into());
                };
                if *len > MAX_ARRAY_LEN || index >= *len {
                    return Err("array index out of bounds".into());
                }
                element
            }
            Projection::Field(index) => ty.field_ty(index).ok_or("invalid field projection")?,
            Projection::Downcast(variant) => {
                let Ty::Enum(owner) = ty else {
                    return Err("downcast requires enum type".into());
                };
                if owner.def().id != variant.owner {
                    return Err("downcast enum owner mismatch".into());
                }
                let def = owner
                    .def()
                    .variants
                    .get(variant.index)
                    .ok_or("downcast variant out of bounds")?;
                let Some(Projection::Field(index)) = path.next() else {
                    return Err("downcast must be immediately followed by field projection".into());
                };
                &def.fields
                    .get(*index)
                    .ok_or("invalid variant field projection")?
                    .ty
            }
        };
    }
    Ok(ty)
}

/// Immutable nominal handle. Equality and hashing use declaration identity,
/// never field shape, names or Arc's structural pointee equality.
#[derive(Clone)]
pub struct StructTy(Arc<StructDef>);

impl std::fmt::Debug for StructTy {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        // Types are a shared DAG, not a tree to expand into diagnostics. Names
        // themselves can be long, so bound their diagnostic rendering as well.
        let name = &self.0.name;
        let end = name
            .char_indices()
            .nth(128)
            .map_or(name.len(), |(at, _)| at);
        f.debug_struct("StructTy")
            .field("id", &self.0.id)
            .field("name", &&name[..end])
            .finish_non_exhaustive()
    }
}

impl StructTy {
    /// The declaration builder must validate the entire by-value dependency DAG
    /// before constructing definitions in dependency-first order.
    pub(super) fn new(def: StructDef) -> Self {
        Self(Arc::new(def))
    }

    pub fn def(&self) -> &StructDef {
        &self.0
    }

    pub fn field_index(&self, name: &str) -> Option<usize> {
        self.0.fields.iter().position(|field| field.name == name)
    }
}

impl PartialEq for StructTy {
    fn eq(&self, other: &Self) -> bool {
        self.0.id == other.0.id
    }
}

impl Eq for StructTy {}

impl Hash for StructTy {
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.0.id.hash(state);
    }
}

#[derive(Clone, Debug)]
pub struct StructDef {
    pub id: DefId,
    /// Canonical module-qualified name, used for diagnostics, not identity.
    pub name: String,
    /// Defining module, used for owner/descendant field visibility checks.
    pub module: String,
    pub public: bool,
    /// Declaration order. No layout offsets or implicit Copy permission.
    pub fields: Arc<[FieldDef]>,
}

#[derive(Clone, Debug)]
pub struct FieldDef {
    pub name: String,
    pub ty: Ty,
    pub public: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct LocalId(pub usize);
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct ExprId(pub usize);
/// Resolved control-flow target, allocated monotonically within one function.
/// Labels are not represented by this milestone's source language.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct LoopId(pub usize);

/// Resolved semantic types. Inference variables never cross this boundary.
/// Tuple structure is immutable and shared cheaply across checked facts.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub enum Ty {
    I32,
    Usize,
    Ref {
        kind: BorrowKind,
        referent: Arc<Ty>,
    },
    Bool,
    Unit,
    /// Nonempty structural tuple. The empty tuple is canonically Unit.
    /// Rust Copy semantics for admitted tuples do not make this host enum Copy.
    Tuple(Arc<[Ty]>),
    Array {
        element: Arc<Ty>,
        len: usize,
    },
    /// Nominal and always non-Copy in this slice, even with only scalar fields.
    Struct(StructTy),
    Enum(EnumTy),
    /// The semantic never type, distinct from runtime completion information.
    Never,
}

impl Ty {
    pub fn field_count(&self) -> Option<usize> {
        match self {
            Self::Tuple(fields) => Some(fields.len()),
            Self::Struct(ty) => Some(ty.def().fields.len()),
            _ => None,
        }
    }

    pub fn field_ty(&self, index: usize) -> Option<&Ty> {
        match self {
            Self::Tuple(fields) => fields.get(index),
            Self::Struct(ty) => ty.def().fields.get(index).map(|field| &field.ty),
            _ => None,
        }
    }

    pub fn is_copy(&self) -> bool {
        match self {
            Self::Ref { kind, .. } => *kind == BorrowKind::Shared,
            Self::I32 | Self::Usize | Self::Bool | Self::Unit | Self::Never => true,
            Self::Struct(_) | Self::Enum(_) => false,
            Self::Tuple(_) | Self::Array { .. } => {
                let mut pending = vec![self];
                let mut seen = std::collections::HashSet::new();
                let mut arrays = std::collections::HashSet::new();
                while let Some(ty) = pending.pop() {
                    match ty {
                        Self::Ref {
                            kind: BorrowKind::Mut,
                            ..
                        } => return false,
                        Self::Struct(_) | Self::Enum(_) => return false,
                        Self::Tuple(fields) if seen.insert(fields.as_ptr()) => {
                            pending.extend(fields.iter());
                        }
                        Self::Array { element, .. } if arrays.insert(Arc::as_ptr(element)) => {
                            // Zero length does not grant implicit Copy permission.
                            pending.push(element);
                        }
                        _ => {}
                    }
                }
                true
            }
        }
    }
}

/// Static normal-completion summary, independent of semantic value type.
/// Returns is conservative: it does not prove an exit is dynamically reachable
/// or that evaluation terminates. Never does not erase an explicit tail's type.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Completion {
    Returns,
    Never,
}

#[derive(Clone, Debug)]
pub struct LocalDecl {
    pub name: String,
    pub ty: Ty,
    pub mutable: bool,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Signature {
    pub ref_contract: Arc<RefContract>,
    pub params: Vec<Ty>,
    pub result: Ty,
}

#[derive(Clone, Debug)]
pub struct CheckedFunction {
    pub ref_contract: Arc<RefContract>,
    pub name: String,
    /// Ordered parameter bindings, distinct entries in `locals`.
    pub params: Vec<LocalId>,
    pub result: Ty,
    pub locals: Vec<LocalDecl>,
    pub exprs: Vec<TypedExpr>,
    pub body: Block,
}

pub const MAX_REGION_PARAMS: usize = 64;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct RegionParamId(pub u16);

#[derive(Clone, Debug, PartialEq, Eq, Default)]
pub enum ReturnContract {
    #[default]
    NoReference,
    SharedWholeParams(Arc<[usize]>),
}

#[derive(Clone, Debug, PartialEq, Eq, Default)]
pub struct RefContract {
    pub region_count: u16,
    pub param_regions: Arc<[Option<RegionParamId>]>,
    pub result_region: Option<RegionParamId>,
    pub outlives: Arc<[(RegionParamId, RegionParamId)]>,
    pub returns: ReturnContract,
}

impl RefContract {
    fn validate_shapes(params: &[Ty], result: &Ty) -> Result<usize, String> {
        if params.len() >= MAX_PATTERN_PROJECTION_WORK {
            return Err("reference contract type work limit exceeded".into());
        }
        let mut pending: Vec<_> = params
            .iter()
            .chain(std::iter::once(result))
            .map(|ty| (ty, 0usize))
            .collect();
        let mut work = 0;
        let mut seen = std::collections::HashSet::new();
        let mut structs = std::collections::HashSet::new();
        let mut enums = std::collections::HashSet::new();
        while let Some((ty, depth)) = pending.pop() {
            work += 1;
            if work > MAX_PATTERN_PROJECTION_WORK || depth > MAX_PATTERN_PROJECTION_DEPTH {
                return Err("reference contract type resource limit exceeded".into());
            }
            match ty {
                Ty::Ref { referent, .. } => {
                    if depth != 0 {
                        return Err("unsupported: nested or aggregate references".into());
                    }
                    pending.push((referent, depth + 1));
                }
                Ty::Tuple(fields) if seen.insert((0, fields.as_ptr() as usize)) => {
                    if fields.len()
                        > MAX_PATTERN_PROJECTION_WORK.saturating_sub(work + pending.len())
                    {
                        return Err("reference contract type work limit exceeded".into());
                    }
                    pending.extend(fields.iter().map(|ty| (ty, depth + 1)));
                }
                Ty::Array { element, .. } if seen.insert((1, Arc::as_ptr(element) as usize)) => {
                    pending.push((element, depth + 1))
                }
                Ty::Struct(ty) if structs.insert(ty.def().id) => {
                    if ty.def().fields.len()
                        > MAX_PATTERN_PROJECTION_WORK.saturating_sub(work + pending.len())
                    {
                        return Err("reference contract type work limit exceeded".into());
                    }
                    pending.extend(ty.def().fields.iter().map(|field| (&field.ty, depth + 1)));
                }
                Ty::Enum(ty) if enums.insert(ty.def().id) => {
                    if ty.def().variants.len() > MAX_ENUM_VARIANTS
                        || ty.def().variants.len() > MAX_PATTERN_PROJECTION_WORK - work
                    {
                        return Err("reference contract enum work limit exceeded".into());
                    }
                    work += ty.def().variants.len();
                    for variant in ty.def().variants.iter() {
                        if variant.fields.len()
                            > MAX_PATTERN_PROJECTION_WORK.saturating_sub(work + pending.len())
                        {
                            return Err("reference contract type work limit exceeded".into());
                        }
                        pending.extend(variant.fields.iter().map(|field| (&field.ty, depth + 1)));
                    }
                }
                _ => {}
            }
            if pending.len() > MAX_PATTERN_PROJECTION_WORK - work {
                return Err("reference contract type work limit exceeded".into());
            }
        }
        Ok(work)
    }
    /// Reflexive declaration closure only. Body universals never infer edges.
    pub fn closure(&self) -> Result<Vec<u64>, String> {
        let count = usize::from(self.region_count);
        if count > MAX_REGION_PARAMS || self.outlives.len() > MAX_REGION_PARAMS * MAX_REGION_PARAMS
        {
            return Err("reference contract region resource limit exceeded".into());
        }
        if self.param_regions.len() > MAX_PATTERN_PROJECTION_WORK {
            return Err("reference contract parameter resource limit exceeded".into());
        }
        for region in self
            .param_regions
            .iter()
            .flatten()
            .chain(self.result_region.iter())
        {
            if usize::from(region.0) >= count {
                return Err("invalid reference contract region ID".into());
            }
        }
        for (longer, shorter) in self.outlives.iter() {
            if usize::from(longer.0) >= count || usize::from(shorter.0) >= count {
                return Err("invalid reference contract outlives region ID".into());
            }
        }
        let mut rows: Vec<u64> = (0..count).map(|index| 1u64 << index).collect();
        for (longer, shorter) in self.outlives.iter() {
            rows[usize::from(longer.0)] |= 1u64 << shorter.0;
        }
        for middle in 0..count {
            let reachable = rows[middle];
            for row in &mut rows {
                if *row & (1u64 << middle) != 0 {
                    *row |= reachable;
                }
            }
        }
        Ok(rows)
    }

    pub fn outlives_region(
        &self,
        longer: RegionParamId,
        shorter: RegionParamId,
    ) -> Result<bool, String> {
        let rows = self.closure()?;
        if usize::from(longer.0) >= rows.len() || usize::from(shorter.0) >= rows.len() {
            return Err("invalid reference contract query region ID".into());
        }
        Ok(rows[usize::from(longer.0)] & (1u64 << shorter.0) != 0)
    }

    pub fn eligible_params(&self, params: &[Ty], result: &Ty) -> Result<Vec<usize>, String> {
        self.eligible_params_counted(params, result)
            .map(|(params, _)| params)
    }

    fn metadata_work(&self) -> usize {
        let count = usize::from(self.region_count);
        self.param_regions.len() * 2 + self.outlives.len() * 2 + count * count + count + 1
    }

    fn eligible_params_counted(
        &self,
        params: &[Ty],
        result: &Ty,
    ) -> Result<(Vec<usize>, usize), String> {
        let shape_work = Self::validate_shapes(params, result)?;
        let rows = self.closure()?;
        if self.param_regions.len() != params.len() {
            return Err("reference contract parameter arity mismatch".into());
        }
        let Ty::Ref {
            kind: BorrowKind::Shared,
            referent: output,
        } = result
        else {
            return Err("shared return contract requires shared reference result".into());
        };
        let region = self
            .result_region
            .ok_or("missing reference result region")?;
        let mut eligible = Vec::new();
        let mut work = 0;
        for (index, (ty, param_region)) in params.iter().zip(self.param_regions.iter()).enumerate()
        {
            match ty {
                Ty::Ref {
                    kind: BorrowKind::Shared,
                    referent,
                } => {
                    let input = param_region.ok_or("missing reference parameter region")?;
                    if rows[usize::from(input.0)] & (1u64 << region.0) != 0
                        && pattern_types_equal(referent, output, &mut work)?
                    {
                        if eligible.len() >= MAX_LOAN_SITES {
                            return Err("reference return candidate limit exceeded".into());
                        }
                        eligible.push(index);
                    }
                }
                Ty::Ref { .. } => {
                    return Err(
                        "unsupported: mutable input to shared reference-return function".into(),
                    );
                }
                _ if param_region.is_some() => {
                    return Err("nonreference parameter has reference region".into());
                }
                _ => {}
            }
        }
        Ok((
            eligible,
            shape_work + self.metadata_work() + params.len() + work,
        ))
    }

    pub fn validate(&self, params: &[Ty], result: &Ty) -> Result<(), String> {
        self.validation_work(params, result).map(|_| ())
    }

    /// Work for this exact structural validation, for the caller's shared
    /// preflight budget. Charge before repeating validation/publication.
    pub fn validation_work(&self, params: &[Ty], result: &Ty) -> Result<usize, String> {
        let mut work = Self::validate_shapes(params, result)?;
        self.closure()?;
        work += self.metadata_work();
        match &self.returns {
            ReturnContract::NoReference => {
                if matches!(result, Ty::Ref { .. })
                    || self.region_count != 0
                    || self.result_region.is_some()
                    || !self.outlives.is_empty()
                    || self.param_regions.iter().any(Option::is_some)
                    || (!self.param_regions.is_empty() && self.param_regions.len() != params.len())
                {
                    return Err("invalid noescape reference contract".into());
                }
            }
            ReturnContract::SharedWholeParams(candidates) => {
                if candidates.len() > MAX_LOAN_SITES {
                    return Err("reference return candidate limit exceeded".into());
                }
                let (eligible, candidate_work) = self.eligible_params_counted(params, result)?;
                work += candidate_work + candidates.len();
                if candidates.as_ref() != eligible.as_slice() {
                    return Err("reference return candidates must equal complete declaration-derived parameters".into());
                }
            }
        }
        if work > MAX_PATTERN_PROJECTION_WORK {
            return Err("reference contract validation work limit exceeded".into());
        }
        Ok(work)
    }
}

#[derive(Clone, Debug)]
pub struct TypedExpr {
    /// Actual semantic type, independent of whether evaluation completes.
    /// For example, `{ return; 0 }` has type I32 and completion Never, while
    /// `{ return; }` has type Never and completion Never. Never is not a
    /// placeholder for every expression that cannot complete normally.
    pub ty: Ty,
    pub completion: Completion,
    pub kind: ExprKind,
}
impl TypedExpr {
    /// Normal-completion summary only, not a semantic type query. In particular,
    /// None discards the actual type of an unreachable but explicitly typed tail.
    /// Operator selection, coercion and inference must inspect `ty` separately.
    pub fn flow(&self) -> Option<Ty> {
        match self.completion {
            Completion::Returns => Some(self.ty.clone()),
            Completion::Never => None,
        }
    }
}

#[derive(Clone, Debug)]
pub struct Block {
    pub statements: Vec<Stmt>,
    pub tail: Option<ExprId>,
}

/// A resolved local-rooted storage location. Projections retain source order
/// and distinguish fields, variant payloads and literal array indices.
/// An empty projection denotes the whole local. No byte offsets or target ABI.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct Place {
    pub local: LocalId,
    pub projection: Vec<Projection>,
}

pub const MAX_PATTERN_BINDINGS: usize = 65_536;
pub const MAX_PATTERN_PROJECTION_DEPTH: usize = 128;
pub const MAX_PATTERN_PROJECTION_WORK: usize = 1_000_000;

/// One binding from an irrefutable pattern. The projection is relative to the
/// initializer place, not an eagerly consumed whole-value snapshot. Ignored
/// wildcard/rest fields have no entry. Entries retain source pattern order.
#[derive(Clone, Debug)]
pub struct PatternBinding {
    pub local: LocalId,
    pub projection: Vec<Projection>,
}

fn pattern_types_equal(a: &Ty, b: &Ty, work: &mut usize) -> Result<bool, String> {
    let mut pending = vec![(a, b, 0)];
    while let Some((a, b, depth)) = pending.pop() {
        *work = work
            .checked_add(1)
            .ok_or("pattern projection work limit exceeded")?;
        if *work > MAX_PATTERN_PROJECTION_WORK {
            return Err("pattern projection work limit exceeded".into());
        }
        if depth > MAX_PATTERN_PROJECTION_DEPTH {
            return Err("pattern type comparison depth exceeds limit 128".into());
        }
        match (a, b) {
            (
                Ty::Ref {
                    kind: ak,
                    referent: a,
                },
                Ty::Ref {
                    kind: bk,
                    referent: b,
                },
            ) if ak == bk => {
                if !Arc::ptr_eq(a, b) {
                    pending.push((a, b, depth + 1));
                }
            }
            (Ty::I32, Ty::I32)
            | (Ty::Usize, Ty::Usize)
            | (Ty::Bool, Ty::Bool)
            | (Ty::Unit, Ty::Unit)
            | (Ty::Never, Ty::Never) => {}
            (Ty::Struct(a), Ty::Struct(b)) if a == b => {}
            (Ty::Enum(a), Ty::Enum(b)) if a == b => {}
            (
                Ty::Array {
                    element: a,
                    len: an,
                },
                Ty::Array {
                    element: b,
                    len: bn,
                },
            ) if an == bn => {
                if !Arc::ptr_eq(a, b) {
                    pending.push((a, b, depth + 1));
                }
            }
            (Ty::Tuple(a), Ty::Tuple(b)) if a.len() == b.len() => {
                if Arc::ptr_eq(a, b) {
                    continue;
                }
                let queued = pending
                    .len()
                    .checked_add(a.len())
                    .ok_or("pattern projection work limit exceeded")?;
                if queued > MAX_PATTERN_PROJECTION_WORK - *work {
                    return Err("pattern projection work limit exceeded".into());
                }
                pending.extend(a.iter().zip(b.iter()).map(|(a, b)| (a, b, depth + 1)));
            }
            _ => return Ok(false),
        }
    }
    Ok(true)
}

#[derive(Clone, Debug)]
pub enum Stmt {
    AssignIndex {
        array: Place,
        index: ExprId,
        value: ExprId,
    },
    /// Starts a fresh binding's lifetime without initializing its value.
    Declare {
        local: LocalId,
    },
    Let {
        local: LocalId,
        value: ExprId,
    },
    LetPattern {
        value: ExprId,
        /// Fully resolved effective pattern type, including an annotation or
        /// concrete pattern hint when the initializer itself has type Never.
        ty: Ty,
        bindings: Vec<PatternBinding>,
    },
    Assign {
        local: LocalId,
        value: ExprId,
    },
    AssignPlace {
        place: Place,
        value: ExprId,
    },
    Expr(ExprId),
    Return(Option<ExprId>),
    While {
        id: LoopId,
        condition: ExprId,
        body: Block,
    },
}

#[derive(Clone, Debug)]
pub enum MatchPattern {
    Wildcard,
    Binding(LocalId),
    Bool(bool),
    Int(i32),
    Unit,
    EnumVariant {
        ty: EnumTy,
        variant: usize,
        bindings: Vec<PatternBinding>,
    },
}

#[derive(Clone, Debug)]
pub struct MatchArm {
    pub pattern: MatchPattern,
    pub body: ExprId,
}

#[derive(Clone, Debug)]
pub enum ExprKind {
    Borrow {
        kind: BorrowKind,
        place: Place,
    },
    Deref {
        value: ExprId,
    },
    Usize(u64),
    DynamicIndex {
        base: ExprId,
        index: ExprId,
    },
    Array(Vec<ExprId>),
    Repeat {
        value: ExprId,
        len: usize,
    },
    Index {
        base: ExprId,
        index: usize,
    },
    EnumVariant {
        ty: EnumTy,
        variant: usize,
        fields: Vec<(usize, ExprId)>,
    },
    Match {
        scrutinee: ExprId,
        arms: Vec<MatchArm>,
    },
    Int(i32),
    Bool(bool),
    /// Read a resolved local into this expression's independent value snapshot.
    Local(LocalId),
    /// Elements are evaluated left to right into a new value snapshot.
    Tuple(Vec<ExprId>),
    /// Initializers retain source evaluation order. Each index names a field in
    /// declaration order; lowering permutes snapshots, never source evaluation.
    Struct {
        ty: StructTy,
        fields: Vec<(usize, ExprId)>,
    },
    /// Projects from an arbitrary expression snapshot, not a live local alias.
    Field {
        base: ExprId,
        index: usize,
    },
    Unary {
        op: UnaryOp,
        value: ExprId,
    },
    Binary {
        op: BinaryOp,
        left: ExprId,
        right: ExprId,
    },
    Call {
        name: String,
        args: Vec<ExprId>,
        /// Validated at typing time, shared by call lowering and ABI selection.
        signature: Signature,
    },
    If {
        condition: ExprId,
        then_block: Block,
        else_block: Block,
    },
    Block(Block),
    /// Repeats the body until an associated break produces a value.
    Loop {
        id: LoopId,
        body: Block,
    },
    /// Evaluates the operand before transferring to the resolved target's exit.
    /// This expression itself never completes normally, even for a unit break.
    Break {
        target: LoopId,
        value: Option<ExprId>,
    },
    /// Transfers to the target's head, including condition evaluation for while.
    Continue {
        target: LoopId,
    },
    Println(String),
}

impl CheckedFunction {
    pub(super) fn validate_match(
        &self,
        scrutinee: ExprId,
        arms: &[MatchArm],
        parameters: &std::collections::HashSet<LocalId>,
    ) -> Result<(), String> {
        let ty = &self
            .exprs
            .get(scrutinee.0)
            .ok_or("invalid match scrutinee")?
            .ty;
        if arms.len() > MAX_MATCH_ARMS || (arms.is_empty() && *ty != Ty::Never) {
            return Err(
                "match arm count must be between 1 and 4096 unless scrutinee is never".into(),
            );
        }
        if let Ty::Enum(ty) = ty {
            ty.validate()?;
        }
        let mut bindings = std::collections::HashSet::new();
        for arm in arms {
            self.exprs.get(arm.body.0).ok_or("invalid match arm body")?;
            let valid = match &arm.pattern {
                MatchPattern::Wildcard => true,
                MatchPattern::Binding(local) => {
                    let decl = self
                        .locals
                        .get(local.0)
                        .ok_or("invalid match binding local")?;
                    if parameters.contains(local) || !bindings.insert(*local) {
                        return Err("match bindings must be distinct fresh locals".into());
                    }
                    let mut work = 0;
                    *ty == Ty::Never || pattern_types_equal(ty, &decl.ty, &mut work)?
                }
                MatchPattern::Bool(_) => matches!(ty, Ty::Bool | Ty::Never),
                MatchPattern::Int(_) => matches!(ty, Ty::I32 | Ty::Never),
                MatchPattern::Unit => matches!(ty, Ty::Unit | Ty::Never),
                MatchPattern::EnumVariant {
                    ty: pattern_ty,
                    variant,
                    bindings: payload_bindings,
                } => {
                    pattern_ty.validate()?;
                    self.validate_pattern_bindings_with_params(
                        &Ty::Enum(pattern_ty.clone()),
                        payload_bindings,
                        parameters,
                    )?;
                    let expected = Projection::Downcast(VariantId {
                        owner: pattern_ty.def().id,
                        index: *variant,
                    });
                    for binding in payload_bindings {
                        if binding.projection.first() != Some(&expected)
                            || !bindings.insert(binding.local)
                        {
                            return Err("invalid or repeated enum payload binding".into());
                        }
                    }
                    *variant < pattern_ty.def().variants.len()
                        && (*ty == Ty::Never || *ty == Ty::Enum(pattern_ty.clone()))
                }
            };
            if !valid {
                return Err("match pattern type or variant mismatch".into());
            }
        }
        Ok(())
    }

    /// Bound and check the flattened plan without revisiting source patterns,
    /// visibility, names or inference. No overlapping bindings are admitted by
    /// this slice, which deliberately excludes `@` patterns.
    #[cfg(test)]
    fn validate_pattern_bindings(
        &self,
        ty: &Ty,
        bindings: &[PatternBinding],
    ) -> Result<(), String> {
        if bindings.is_empty() {
            return Ok(());
        }
        let parameters = self.params.iter().copied().collect();
        self.validate_pattern_bindings_with_params(ty, bindings, &parameters)
    }

    pub(super) fn validate_pattern_bindings_with_params(
        &self,
        ty: &Ty,
        bindings: &[PatternBinding],
        parameters: &std::collections::HashSet<LocalId>,
    ) -> Result<(), String> {
        if bindings.len() > MAX_PATTERN_BINDINGS {
            return Err("pattern binding limit exceeded".into());
        }
        let mut work = bindings.len();
        let mut locals = std::collections::HashSet::new();
        let mut paths = Vec::with_capacity(bindings.len());
        for binding in bindings {
            if binding.projection.len() > MAX_PATTERN_PROJECTION_DEPTH {
                return Err("pattern projection depth exceeds limit 128".into());
            }
            work = work
                .checked_add(binding.projection.len())
                .ok_or("pattern projection work limit exceeded")?;
            if work > MAX_PATTERN_PROJECTION_WORK {
                return Err("pattern projection work limit exceeded".into());
            }
            let local = self
                .locals
                .get(binding.local.0)
                .ok_or("invalid pattern binding local")?;
            if !locals.insert(binding.local) || parameters.contains(&binding.local) {
                return Err("pattern binding locals must be distinct fresh locals".into());
            }
            let projected = projected_ty(ty, &binding.projection)?;
            if !pattern_types_equal(&local.ty, projected, &mut work)? {
                return Err("pattern binding local type mismatch".into());
            }
            paths.push(binding.projection.as_slice());
        }
        paths.sort_unstable();
        if paths.windows(2).any(|pair| pair[1].starts_with(pair[0])) {
            return Err("overlapping pattern binding projections".into());
        }
        Ok(())
    }

    /// Optional structural debugging check. This is not source type checking,
    /// borrow checking, or a codegen prerequisite. Normal compilation consumes
    /// the immutable result produced by the checker without revalidating it.
    ///
    /// Checks reference bounds, distinct parameters, call arity and flattened
    /// pattern plan consistency. Deliberately
    /// does not establish lexical scope, reachability, expression flow, or loop
    /// target validity or projection typing. Those are established once by
    /// semantic analysis.
    pub fn validate(&self) -> Result<(), String> {
        let local = |id: LocalId| {
            if id.0 < self.locals.len() {
                Ok(())
            } else {
                Err(format!("invalid local {} in {}", id.0, self.name))
            }
        };
        let expr = |id: ExprId| {
            if id.0 < self.exprs.len() {
                Ok(())
            } else {
                Err(format!("invalid expression {} in {}", id.0, self.name))
            }
        };
        let mut parameters = std::collections::HashSet::new();
        for &id in &self.params {
            local(id)?;
            if !parameters.insert(id) {
                return Err(format!(
                    "duplicate parameter local {} in {}",
                    id.0, self.name
                ));
            }
        }
        if self.params.len() >= MAX_PATTERN_PROJECTION_WORK {
            return Err("reference contract parameter resource limit exceeded".into());
        }
        let parameter_types: Vec<_> = self
            .params
            .iter()
            .map(|id| self.locals[id.0].ty.clone())
            .collect();
        self.ref_contract.validate(&parameter_types, &self.result)?;
        // Traverse nested blocks iteratively so malformed/deep fixtures cannot
        // exhaust the host call stack inside this diagnostic helper.
        let mut blocks = vec![&self.body];
        for value in &self.exprs {
            match &value.kind {
                ExprKind::Borrow { kind, place } => {
                    let root = &self
                        .locals
                        .get(place.local.0)
                        .ok_or("invalid borrow local")?
                        .ty;
                    let referent = projected_ty(root, &place.projection)?;
                    let expected = Ty::Ref {
                        kind: *kind,
                        referent: Arc::new(referent.clone()),
                    };
                    if !pattern_types_equal(&value.ty, &expected, &mut 0)? {
                        return Err("borrow result type mismatch".into());
                    }
                }
                ExprKind::Deref { value: source } => {
                    expr(*source)?;
                    let Ty::Ref { referent, .. } = &self.exprs[source.0].ty else {
                        return Err("dereference requires reference type".into());
                    };
                    if !pattern_types_equal(&value.ty, referent, &mut 0)? {
                        return Err("dereference result type mismatch".into());
                    }
                }
                ExprKind::DynamicIndex { base, index } => {
                    expr(*base)?;
                    expr(*index)?;
                    if self.exprs[index.0].ty != Ty::Usize && self.exprs[index.0].ty != Ty::Never {
                        return Err("dynamic array index requires usize".into());
                    }
                    let Ty::Array { element, len } = &self.exprs[base.0].ty else {
                        return Err("dynamic indexing requires array type".into());
                    };
                    if *len > MAX_ARRAY_LEN {
                        return Err("array length exceeds limit 4096".into());
                    }
                    let mut work = 0;
                    if !pattern_types_equal(&value.ty, element, &mut work)? {
                        return Err("dynamic index result type mismatch".into());
                    }
                }
                ExprKind::Array(elements) => {
                    let Ty::Array { len, .. } = &value.ty else {
                        return Err("array expression requires array type".into());
                    };
                    if *len > MAX_ARRAY_LEN || elements.len() != *len {
                        return Err("array expression length mismatch".into());
                    }
                    for &id in elements {
                        expr(id)?;
                    }
                }
                ExprKind::Repeat {
                    value: operand,
                    len,
                } => {
                    expr(*operand)?;
                    let Ty::Array {
                        element,
                        len: count,
                    } = &value.ty
                    else {
                        return Err("array repeat requires array type".into());
                    };
                    if len != count || *len > MAX_ARRAY_LEN || (*len > 1 && !element.is_copy()) {
                        return Err("invalid array repeat type, length or Copy requirement".into());
                    }
                }
                ExprKind::Index { base, index } => {
                    expr(*base)?;
                    projected_ty(&self.exprs[base.0].ty, &[Projection::ConstantIndex(*index)])?;
                }
                ExprKind::EnumVariant {
                    ty,
                    variant,
                    fields,
                } => {
                    ty.validate()?;
                    if *variant >= ty.def().variants.len() || value.ty != Ty::Enum(ty.clone()) {
                        return Err("invalid enum variant expression".into());
                    }
                    let def = &ty.def().variants[*variant];
                    if fields.len() != def.fields.len() {
                        return Err("enum initializer field count mismatch".into());
                    }
                    let mut seen = std::collections::HashSet::new();
                    for &(index, value) in fields {
                        if index >= def.fields.len() || !seen.insert(index) {
                            return Err("invalid enum initializer field".into());
                        }
                        expr(value)?;
                    }
                }
                ExprKind::Match { scrutinee, arms } => {
                    expr(*scrutinee)?;
                    self.validate_match(*scrutinee, arms, &parameters)?;
                    for arm in arms {
                        expr(arm.body)?;
                    }
                }
                ExprKind::Local(id) => local(*id)?,
                ExprKind::Tuple(elements) => {
                    for &id in elements {
                        expr(id)?;
                    }
                }
                ExprKind::Struct { fields, .. } => {
                    for &(_, id) in fields {
                        expr(id)?;
                    }
                }
                ExprKind::Field { base, .. } => expr(*base)?,
                ExprKind::Unary { value, .. } => expr(*value)?,
                ExprKind::Binary { left, right, .. } => {
                    expr(*left)?;
                    expr(*right)?;
                }
                ExprKind::Call {
                    name,
                    args,
                    signature,
                } => {
                    signature
                        .ref_contract
                        .validate(&signature.params, &signature.result)?;
                    if args.len() != signature.params.len() {
                        return Err(format!("invalid call arity for {name} in {}", self.name));
                    }
                    for &id in args {
                        expr(id)?;
                    }
                }
                ExprKind::If {
                    condition,
                    then_block,
                    else_block,
                } => {
                    expr(*condition)?;
                    blocks.push(then_block);
                    blocks.push(else_block);
                }
                ExprKind::Block(block) | ExprKind::Loop { body: block, .. } => blocks.push(block),
                ExprKind::Break {
                    value: Some(value), ..
                } => expr(*value)?,
                ExprKind::Break { value: None, .. } | ExprKind::Continue { .. } => {}
                ExprKind::Int(_)
                | ExprKind::Usize(_)
                | ExprKind::Bool(_)
                | ExprKind::Println(_) => {}
            }
        }
        while let Some(block) = blocks.pop() {
            if let Some(id) = block.tail {
                expr(id)?;
            }
            for statement in &block.statements {
                match statement {
                    Stmt::AssignIndex {
                        array,
                        index,
                        value,
                    } => {
                        local(array.local)?;
                        expr(*index)?;
                        expr(*value)?;
                        let array_ty =
                            projected_ty(&self.locals[array.local.0].ty, &array.projection)?;
                        if !matches!(array_ty, Ty::Array { .. }) {
                            return Err("dynamic indexed assignment requires array type".into());
                        }
                        if self.exprs[index.0].ty != Ty::Usize
                            && self.exprs[index.0].ty != Ty::Never
                        {
                            return Err("dynamic array index requires usize".into());
                        }
                    }
                    Stmt::Declare { local: id } => {
                        local(*id)?;
                        if parameters.contains(id) {
                            return Err("declaration local aliases parameter".into());
                        }
                    }
                    Stmt::Let { local: id, value } | Stmt::Assign { local: id, value } => {
                        local(*id)?;
                        expr(*value)?;
                    }
                    Stmt::LetPattern {
                        value,
                        ty,
                        bindings,
                    } => {
                        expr(*value)?;
                        self.validate_pattern_bindings_with_params(ty, bindings, &parameters)?;
                    }
                    Stmt::AssignPlace { place, value } => {
                        local(place.local)?;
                        expr(*value)?;
                    }
                    Stmt::Expr(id) | Stmt::Return(Some(id)) => expr(*id)?,
                    Stmt::Return(None) => {}
                    Stmt::While {
                        condition, body, ..
                    } => {
                        expr(*condition)?;
                        blocks.push(body);
                    }
                }
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn reference_contract_closure_candidates_and_malformed_metadata() {
        use super::*;
        let shared = |ty| Ty::Ref {
            kind: BorrowKind::Shared,
            referent: Arc::new(ty),
        };
        let params = vec![Ty::Bool, shared(Ty::Bool), shared(Ty::I32), shared(Ty::I32)];
        let result = shared(Ty::I32);
        let contract = RefContract {
            region_count: 3,
            param_regions: Arc::from([
                None,
                Some(RegionParamId(0)),
                Some(RegionParamId(0)),
                Some(RegionParamId(1)),
            ]),
            result_region: Some(RegionParamId(2)),
            outlives: Arc::from([
                (RegionParamId(0), RegionParamId(1)),
                (RegionParamId(1), RegionParamId(2)),
            ]),
            returns: ReturnContract::SharedWholeParams(Arc::from([2, 3])),
        };
        contract.validate(&params, &result).unwrap();
        assert!(
            contract
                .outlives_region(RegionParamId(0), RegionParamId(2))
                .unwrap()
        );
        assert!(
            !contract
                .outlives_region(RegionParamId(2), RegionParamId(0))
                .unwrap()
        );
        assert_eq!(
            contract.eligible_params(&params, &result).unwrap(),
            vec![2, 3]
        );
        assert!(contract.validation_work(&params, &result).unwrap() >= params.len());
        let mut invalid = contract.clone();
        invalid.returns = ReturnContract::SharedWholeParams(Arc::from([2]));
        assert!(invalid.validate(&params, &result).is_err());
        invalid.returns = ReturnContract::SharedWholeParams(Arc::from([3, 2]));
        assert!(invalid.validate(&params, &result).is_err());
        invalid.returns = ReturnContract::SharedWholeParams(Arc::from([2, 2, 3]));
        assert!(invalid.validate(&params, &result).is_err());
        for region in [3, u16::MAX] {
            let mut invalid = contract.clone();
            invalid.result_region = Some(RegionParamId(region));
            assert!(invalid.validate(&params, &result).is_err());
            assert!(
                contract
                    .outlives_region(RegionParamId(region), RegionParamId(0))
                    .is_err()
            );
        }
        let mut invalid = contract.clone();
        invalid.region_count = 65;
        assert!(invalid.closure().is_err());
        invalid = contract.clone();
        invalid.outlives = Arc::from([(RegionParamId(0), RegionParamId(63))]);
        assert!(invalid.closure().is_err());
        invalid = contract.clone();
        invalid.param_regions = Arc::from([Some(RegionParamId(0))]);
        assert!(invalid.validate(&params, &result).is_err());
        let mut no_edges = contract.clone();
        no_edges.outlives = Arc::from([]);
        no_edges.returns = ReturnContract::SharedWholeParams(Arc::from([]));
        no_edges.validate(&params, &result).unwrap();
        assert!(
            !no_edges
                .outlives_region(RegionParamId(0), RegionParamId(2))
                .unwrap()
        );
        assert!(
            contract
                .validate(
                    &params,
                    &Ty::Ref {
                        kind: BorrowKind::Mut,
                        referent: Arc::new(Ty::I32)
                    }
                )
                .is_err()
        );
        assert!(RefContract::default().validate(&params, &result).is_err());
        RefContract::default().validate(&params, &Ty::Unit).unwrap();
        let canonical = RefContract {
            param_regions: vec![None; params.len()].into(),
            ..Default::default()
        };
        canonical.validate(&params, &Ty::Unit).unwrap();
        let mut nested = params.clone();
        nested[2] = shared(shared(Ty::I32));
        assert!(contract.validate(&nested, &result).is_err());
    }

    #[test]
    fn reference_copy_and_projection_contract() {
        use super::*;
        let shared = Ty::Ref {
            kind: BorrowKind::Shared,
            referent: Arc::new(Ty::Tuple(Arc::from([Ty::I32]))),
        };
        let mutable = Ty::Ref {
            kind: BorrowKind::Mut,
            referent: Arc::new(Ty::I32),
        };
        assert!(shared.is_copy());
        assert!(!mutable.is_copy());
        assert_eq!(
            projected_ty(&shared, &[Projection::Deref, Projection::Field(0)]).unwrap(),
            &Ty::I32
        );
        assert!(projected_ty(&shared, &[Projection::Deref, Projection::Deref]).is_err());
        assert!(projected_ty(&Ty::I32, &[Projection::Deref]).is_err());
        assert!(!Ty::Tuple(Arc::from([mutable])).is_copy());
    }
    use super::*;

    #[test]
    fn arrays_are_structural_indexed_and_copy_depends_on_element_even_when_empty() {
        let scalar = Ty::Array {
            element: Arc::new(Ty::I32),
            len: 2,
        };
        assert!(scalar.is_copy());
        assert_eq!(scalar.field_count(), None);
        assert_eq!(scalar.field_ty(0), None);
        assert_eq!(
            projected_ty(&scalar, &[Projection::ConstantIndex(1)]).unwrap(),
            &Ty::I32
        );
        assert!(projected_ty(&scalar, &[Projection::Field(0)]).is_err());
        assert!(projected_ty(&scalar, &[Projection::ConstantIndex(2)]).is_err());
        let empty = Ty::Array {
            element: Arc::new(Ty::I32),
            len: 0,
        };
        assert!(projected_ty(&empty, &[Projection::ConstantIndex(0)]).is_err());
        let noncopy = Ty::Struct(nominal(
            DefId {
                snapshot: SnapshotId::allocate().unwrap(),
                index: 0,
            },
            "S",
            vec![Ty::I32],
        ));
        for len in [0, 1, 2] {
            assert!(
                !Ty::Array {
                    element: Arc::new(noncopy.clone()),
                    len
                }
                .is_copy()
            );
        }
        let mut work = 0;
        assert!(
            pattern_types_equal(
                &scalar,
                &Ty::Array {
                    element: Arc::new(Ty::I32),
                    len: 2
                },
                &mut work
            )
            .unwrap()
        );
        assert!(!pattern_types_equal(&scalar, &empty, &mut work).unwrap());
        let mut function = fixture();
        function.locals[0].ty = noncopy.clone();
        function.exprs[0].ty = noncopy.clone();
        for len in [0, 1] {
            function.exprs.push(TypedExpr {
                ty: Ty::Array {
                    element: Arc::new(noncopy.clone()),
                    len,
                },
                completion: Completion::Returns,
                kind: ExprKind::Repeat {
                    value: ExprId(0),
                    len,
                },
            });
        }
        function.validate().unwrap();
        function.exprs.push(TypedExpr {
            ty: Ty::Array {
                element: Arc::new(noncopy),
                len: 2,
            },
            completion: Completion::Returns,
            kind: ExprKind::Repeat {
                value: ExprId(0),
                len: 2,
            },
        });
        assert!(function.validate().unwrap_err().contains("Copy"));
    }

    fn unit_variant(name: &str) -> VariantDef {
        VariantDef {
            name: name.into(),
            shape: VariantShape::Unit,
            fields: Arc::from([]),
        }
    }

    #[test]
    fn payload_projection_requires_owner_bound_downcast_field_pair() {
        let id = DefId {
            snapshot: SnapshotId::allocate().unwrap(),
            index: 0,
        };
        let ty = EnumTy::new(EnumDef {
            id,
            name: "E".into(),
            module: String::new(),
            public: false,
            variants: vec![
                VariantDef {
                    name: "A".into(),
                    shape: VariantShape::Tuple,
                    fields: vec![FieldDef {
                        name: "0".into(),
                        ty: Ty::Tuple(vec![Ty::I32].into()),
                        public: true,
                    }]
                    .into(),
                },
                unit_variant("B"),
            ]
            .into(),
        });
        assert!(ty.has_payload());
        let root = Ty::Enum(ty);
        let variant = VariantId {
            owner: id,
            index: 0,
        };
        assert_eq!(
            projected_ty(
                &root,
                &[
                    Projection::Downcast(variant),
                    Projection::Field(0),
                    Projection::Field(0)
                ]
            )
            .unwrap(),
            &Ty::I32
        );
        for projection in [
            vec![Projection::Field(0)],
            vec![Projection::Downcast(variant)],
            vec![Projection::Downcast(variant), Projection::Downcast(variant)],
            vec![
                Projection::Downcast(VariantId {
                    index: 1,
                    ..variant
                }),
                Projection::Field(0),
            ],
            vec![
                Projection::Downcast(VariantId {
                    owner: DefId { index: 1, ..id },
                    ..variant
                }),
                Projection::Field(0),
            ],
        ] {
            assert!(projected_ty(&root, &projection).is_err(), "{projection:?}");
        }
        let empty_payload = EnumTy::new(EnumDef {
            id,
            name: "Empty".into(),
            module: String::new(),
            public: false,
            variants: vec![VariantDef {
                name: "A".into(),
                shape: VariantShape::Struct,
                fields: Arc::from([]),
            }]
            .into(),
        });
        assert!(
            empty_payload.has_payload(),
            "zero-field non-unit shapes retain indirect ABI"
        );
    }

    #[test]
    fn deferred_declaration_references_require_nonparameter_locals() {
        let mut function = fixture();
        function
            .body
            .statements
            .push(Stmt::Declare { local: LocalId(1) });
        function.validate().unwrap();
        function
            .body
            .statements
            .push(Stmt::Declare { local: LocalId(0) });
        assert!(function.validate().unwrap_err().contains("parameter"));
        function.body.statements.pop();
        function.body.statements.push(Stmt::Declare {
            local: LocalId(usize::MAX),
        });
        assert!(function.validate().unwrap_err().contains("invalid local"));
    }

    #[test]
    fn enum_identity_copy_debug_and_variant_bounds() {
        let id = DefId {
            snapshot: SnapshotId::allocate().unwrap(),
            index: 0,
        };
        let handle = EnumTy::new(EnumDef {
            id,
            name: "é".repeat(1000),
            module: String::new(),
            public: false,
            variants: vec![unit_variant("A")].into(),
        });
        let same_id = EnumTy::new(EnumDef {
            id,
            name: "Other".into(),
            module: String::new(),
            public: true,
            variants: vec![unit_variant("B")].into(),
        });
        assert_eq!(handle, same_id);
        let mut set = std::collections::HashSet::new();
        assert!(set.insert(handle.clone()));
        assert!(!set.insert(same_id));
        let ty = Ty::Enum(handle.clone());
        assert!(!ty.is_copy());
        assert!(!Ty::Tuple(vec![ty.clone()].into()).is_copy());
        assert_eq!(ty.field_count(), None);
        assert_eq!(ty.field_ty(0), None);
        assert!(format!("{handle:?}").len() < 512);
        let mut function = fixture();
        function.exprs.push(TypedExpr {
            ty,
            completion: Completion::Returns,
            kind: ExprKind::EnumVariant {
                ty: handle,
                variant: 1,
                fields: vec![],
            },
        });
        assert!(function.validate().unwrap_err().contains("enum variant"));
        for count in [0, MAX_ENUM_VARIANTS + 1] {
            let invalid = EnumTy::new(EnumDef {
                id,
                name: "Invalid".into(),
                module: String::new(),
                public: false,
                variants: vec![unit_variant(""); count].into(),
            });
            assert!(invalid.validate().is_err());
        }
    }

    #[test]
    fn match_validation_bounds_and_checks_dead_patterns() {
        let function = fixture();
        let parameters = function.params.iter().copied().collect();
        let arm = MatchArm {
            pattern: MatchPattern::Wildcard,
            body: ExprId(0),
        };
        assert!(
            function
                .validate_match(ExprId(0), &[], &parameters)
                .is_err()
        );
        assert!(
            function
                .validate_match(
                    ExprId(0),
                    &vec![arm.clone(); MAX_MATCH_ARMS + 1],
                    &parameters
                )
                .is_err()
        );
        let invalid = MatchArm {
            pattern: MatchPattern::Binding(LocalId(usize::MAX)),
            body: ExprId(0),
        };
        assert!(
            function
                .validate_match(ExprId(0), &[arm.clone(), invalid], &parameters)
                .is_err()
        );
        let invalid = MatchArm {
            pattern: MatchPattern::Wildcard,
            body: ExprId(usize::MAX),
        };
        assert!(
            function
                .validate_match(ExprId(0), &[arm, invalid], &parameters)
                .is_err()
        );
    }

    #[test]
    fn flattened_pattern_plan_validates_types_and_disjoint_fresh_bindings() {
        let mut function = fixture();
        let ty = Ty::Tuple(vec![Ty::I32, Ty::Bool].into());
        function.locals[0].ty = ty.clone();
        function.exprs[0].ty = ty.clone();
        function.locals.push(LocalDecl {
            name: "first".into(),
            ty: Ty::I32,
            mutable: false,
        });
        let bindings = vec![
            PatternBinding {
                local: LocalId(2),
                projection: vec![0].into_iter().map(Projection::Field).collect(),
            },
            PatternBinding {
                local: LocalId(1),
                projection: vec![1].into_iter().map(Projection::Field).collect(),
            },
        ];
        function.body.statements = vec![Stmt::LetPattern {
            value: ExprId(0),
            ty: ty.clone(),
            bindings: bindings.clone(),
        }];
        function.validate().unwrap();
        function.validate_pattern_bindings(&ty, &[]).unwrap();
        let mut invalid = bindings.clone();
        invalid[1].local = LocalId(2);
        assert!(
            function
                .validate_pattern_bindings(&ty, &invalid)
                .unwrap_err()
                .contains("distinct")
        );
        invalid[1].local = LocalId(0);
        assert!(
            function
                .validate_pattern_bindings(&ty, &invalid)
                .unwrap_err()
                .contains("fresh")
        );
        invalid[1].local = LocalId(1);
        invalid[1].projection = vec![Projection::Field(2)];
        assert!(
            function
                .validate_pattern_bindings(&ty, &invalid)
                .unwrap_err()
                .contains("projection")
        );
        invalid[1].projection = vec![Projection::Field(0)];
        assert!(
            function
                .validate_pattern_bindings(&ty, &invalid)
                .unwrap_err()
                .contains("type mismatch")
        );
        function.locals.push(LocalDecl {
            name: "overlap".into(),
            ty: Ty::I32,
            mutable: false,
        });
        invalid[1].local = LocalId(3);
        assert!(
            function
                .validate_pattern_bindings(&ty, &invalid)
                .unwrap_err()
                .contains("overlapping")
        );
        function.locals[3].ty = ty.clone();
        invalid[1].projection.clear();
        assert!(
            function
                .validate_pattern_bindings(&ty, &invalid)
                .unwrap_err()
                .contains("overlapping")
        );
    }

    #[test]
    fn flattened_pattern_plan_enforces_binding_depth_and_work_bounds() {
        let mut function = fixture();
        let binding = PatternBinding {
            local: LocalId(1),
            projection: vec![],
        };
        let oversized = vec![binding.clone(); MAX_PATTERN_BINDINGS + 1];
        assert!(
            function
                .validate_pattern_bindings(&Ty::Bool, &oversized)
                .unwrap_err()
                .contains("binding limit")
        );
        let deep = PatternBinding {
            local: LocalId(1),
            projection: vec![Projection::Field(0); MAX_PATTERN_PROJECTION_DEPTH + 1],
        };
        assert!(
            function
                .validate_pattern_bindings(&Ty::Bool, &[deep])
                .unwrap_err()
                .contains("depth")
        );

        let mut root = Ty::I32;
        for _ in 0..MAX_PATTERN_PROJECTION_DEPTH {
            root = Ty::Tuple(vec![root].into());
        }
        let count = MAX_PATTERN_PROJECTION_WORK / (MAX_PATTERN_PROJECTION_DEPTH + 1) + 1;
        function.locals.truncate(1);
        let mut bindings = Vec::with_capacity(count);
        for index in 0..count {
            function.locals.push(LocalDecl {
                name: format!("b{index}"),
                ty: Ty::I32,
                mutable: false,
            });
            bindings.push(PatternBinding {
                local: LocalId(index + 1),
                projection: vec![Projection::Field(0); MAX_PATTERN_PROJECTION_DEPTH],
            });
        }
        assert!(
            function
                .validate_pattern_bindings(&root, &bindings)
                .unwrap_err()
                .contains("work limit")
        );
    }

    #[test]
    fn pattern_type_equality_fails_closed_on_independently_allocated_shared_dags() {
        let mut a = Ty::I32;
        let mut b = Ty::I32;
        for _ in 0..32 {
            a = Ty::Tuple(vec![a.clone(), a].into());
            b = Ty::Tuple(vec![b.clone(), b].into());
        }
        assert!(a.is_copy());
        assert!(b.is_copy());
        let mut function = fixture();
        function.locals[1].ty = a;
        let bindings = [PatternBinding {
            local: LocalId(1),
            projection: vec![],
        }];
        assert!(
            function
                .validate_pattern_bindings(&b, &bindings)
                .unwrap_err()
                .contains("work limit")
        );
    }

    fn nominal(id: DefId, name: &str, fields: Vec<Ty>) -> StructTy {
        StructTy::new(StructDef {
            id,
            name: name.into(),
            module: String::new(),
            public: false,
            fields: fields
                .into_iter()
                .enumerate()
                .map(|(index, ty)| FieldDef {
                    name: format!("f{index}"),
                    ty,
                    public: false,
                })
                .collect::<Vec<_>>()
                .into(),
        })
    }

    #[test]
    fn nominal_identity_is_not_name_shape_or_declaration_index_alone() {
        let snapshot = SnapshotId::allocate().unwrap();
        let first = nominal(DefId { snapshot, index: 0 }, "S", vec![Ty::I32]);
        let same_shape = nominal(DefId { snapshot, index: 1 }, "S", vec![Ty::I32]);
        let next_snapshot = nominal(
            DefId {
                snapshot: SnapshotId::allocate().unwrap(),
                index: 0,
            },
            "S",
            vec![Ty::I32],
        );
        assert_eq!(first, first.clone());
        assert_ne!(first, same_shape);
        assert_ne!(first, next_snapshot);
        let mut identities = std::collections::HashSet::new();
        assert!(identities.insert(first.clone()));
        assert!(!identities.insert(first));
        assert!(identities.insert(same_shape));
        assert!(identities.insert(next_snapshot));
    }

    #[test]
    fn aggregate_helpers_preserve_nominal_non_copy_and_projected_copy() {
        let handle = nominal(
            DefId {
                snapshot: SnapshotId::allocate().unwrap(),
                index: 0,
            },
            "S",
            vec![Ty::I32, Ty::Tuple(vec![Ty::Bool, Ty::Unit].into())],
        );
        assert_eq!(handle.field_index("f1"), Some(1));
        assert_eq!(handle.field_index("missing"), None);
        let ty = Ty::Struct(handle);
        assert_eq!(ty.field_count(), Some(2));
        assert_eq!(ty.field_ty(0), Some(&Ty::I32));
        assert_eq!(ty.field_ty(2), None);
        assert!(!ty.is_copy());
        assert!(ty.field_ty(1).unwrap().is_copy());
        assert!(Ty::Tuple(vec![Ty::Bool, Ty::Unit].into()).is_copy());
        assert!(!Ty::Tuple(vec![Ty::Bool, ty].into()).is_copy());
        assert_eq!(Ty::I32.field_count(), None);
        assert_eq!(Ty::I32.field_ty(0), None);
    }

    #[test]
    fn snapshot_allocation_fails_at_exhaustion_without_wrapping() {
        let next = AtomicU64::new(u64::MAX - 1);
        assert_eq!(SnapshotId::allocate_from(&next).unwrap().0, u64::MAX - 1);
        assert!(SnapshotId::allocate_from(&next).is_err());
        assert!(SnapshotId::allocate_from(&next).is_err());
        assert_eq!(next.load(Ordering::Relaxed), u64::MAX);
    }

    #[test]
    fn snapshot_allocation_is_unique_across_threads() {
        let workers: Vec<_> = (0..8)
            .map(|_| {
                std::thread::spawn(|| {
                    (0..32)
                        .map(|_| SnapshotId::allocate().unwrap())
                        .collect::<Vec<_>>()
                })
            })
            .collect();
        let ids: std::collections::HashSet<_> = workers
            .into_iter()
            .flat_map(|worker| worker.join().unwrap())
            .collect();
        assert_eq!(ids.len(), 256);
    }

    #[test]
    fn nominal_debug_bounds_unicode_names_and_does_not_expand_shared_fields() {
        let snapshot = SnapshotId::allocate().unwrap();
        let mut ty = Ty::I32;
        for index in 0..16 {
            ty = Ty::Struct(nominal(
                DefId { snapshot, index },
                &"é".repeat(1000),
                vec![ty.clone(), ty],
            ));
        }
        let diagnostic = format!("{ty:?}");
        assert!(diagnostic.len() < 512, "unbounded nominal diagnostic");
        assert!(!diagnostic.contains("fields"));
        assert!(diagnostic.contains("DefId"));
    }

    #[test]
    fn tuple_types_are_structural_and_cheaply_cloned() {
        let fields: Arc<[Ty]> = vec![Ty::I32, Ty::Tuple(vec![Ty::Bool, Ty::Unit].into())].into();
        let ty = Ty::Tuple(fields.clone());
        let expression = TypedExpr {
            ty: ty.clone(),
            completion: Completion::Returns,
            kind: ExprKind::Tuple(vec![]),
        };
        let Some(Ty::Tuple(cloned_fields)) = expression.flow() else {
            panic!("tuple flow missing")
        };
        assert!(Arc::ptr_eq(&fields, &cloned_fields));
        assert_eq!(
            ty,
            Ty::Tuple(vec![Ty::I32, Ty::Tuple(vec![Ty::Bool, Ty::Unit].into())].into())
        );
        assert_ne!(ty, Ty::Tuple(vec![Ty::Bool, Ty::I32].into()));
        assert_ne!(Ty::Unit, Ty::Tuple(vec![Ty::Unit].into()));
    }

    #[test]
    fn tuple_field_and_assignment_references_are_checked() {
        let mut value = fixture();
        value.exprs.push(TypedExpr {
            ty: Ty::Tuple(vec![Ty::I32].into()),
            completion: Completion::Returns,
            kind: ExprKind::Tuple(vec![ExprId(0)]),
        });
        value.exprs.push(TypedExpr {
            ty: Ty::I32,
            completion: Completion::Returns,
            kind: ExprKind::Field {
                base: ExprId(1),
                index: 0,
            },
        });
        value.body.statements.push(Stmt::AssignPlace {
            place: Place {
                local: LocalId(0),
                projection: vec![0, 1].into_iter().map(Projection::Field).collect(),
            },
            value: ExprId(2),
        });
        // Structural validation intentionally does not redo projection typing.
        value.validate().unwrap();
        value.exprs[2].kind = ExprKind::Field {
            base: ExprId(99),
            index: 0,
        };
        assert!(
            value
                .validate()
                .unwrap_err()
                .contains("invalid expression 99")
        );
        value.exprs[2].kind = ExprKind::Tuple(vec![ExprId(99)]);
        assert!(
            value
                .validate()
                .unwrap_err()
                .contains("invalid expression 99")
        );
        value.exprs[2].kind = ExprKind::Int(0);
        value.body.statements.push(Stmt::AssignPlace {
            place: Place {
                local: LocalId(99),
                projection: vec![0].into_iter().map(Projection::Field).collect(),
            },
            value: ExprId(2),
        });
        assert!(value.validate().unwrap_err().contains("invalid local 99"));
    }

    fn fixture() -> CheckedFunction {
        CheckedFunction {
            ref_contract: Default::default(),
            name: "f".into(),
            params: vec![LocalId(0)],
            result: Ty::Unit,
            locals: vec![
                LocalDecl {
                    name: "x".into(),
                    ty: Ty::I32,
                    mutable: false,
                },
                LocalDecl {
                    name: "x".into(),
                    ty: Ty::Bool,
                    mutable: true,
                },
            ],
            exprs: vec![TypedExpr {
                ty: Ty::I32,
                completion: Completion::Returns,
                kind: ExprKind::Local(LocalId(0)),
            }],
            body: Block {
                statements: vec![Stmt::Expr(ExprId(0))],
                tail: None,
            },
        }
    }

    #[test]
    fn divergence_is_not_unit_flow() {
        let value = TypedExpr {
            ty: Ty::Never,
            completion: Completion::Never,
            kind: ExprKind::Block(Block {
                statements: vec![Stmt::Return(None)],
                tail: None,
            }),
        };
        assert_eq!(value.flow(), None);
        assert_eq!(value.ty, Ty::Never);
        let value = TypedExpr {
            ty: Ty::Unit,
            completion: Completion::Returns,
            ..value
        };
        assert_eq!(value.flow(), Some(Ty::Unit));
    }

    #[test]
    fn unreachable_typed_tail_retains_its_semantic_type() {
        let mut value = fixture();
        value.exprs.push(TypedExpr {
            ty: Ty::I32,
            completion: Completion::Never,
            kind: ExprKind::Block(Block {
                statements: vec![Stmt::Return(None)],
                tail: Some(ExprId(0)),
            }),
        });
        assert_eq!(value.exprs[1].ty, Ty::I32);
        assert_eq!(value.exprs[1].flow(), None);
        assert_ne!(value.exprs[1].ty, Ty::Never);
        value.validate().unwrap();

        // A loop's break operand can have a concrete type without completing.
        // Its semantic result type must not be overwritten by its completion.
        value.exprs.push(TypedExpr {
            ty: Ty::Never,
            completion: Completion::Never,
            kind: ExprKind::Break {
                target: LoopId(0),
                value: Some(ExprId(1)),
            },
        });
        value.exprs.push(TypedExpr {
            ty: Ty::I32,
            completion: Completion::Never,
            kind: ExprKind::Loop {
                id: LoopId(0),
                body: Block {
                    statements: vec![Stmt::Expr(ExprId(2))],
                    tail: None,
                },
            },
        });
        assert_eq!(value.exprs[3].ty, Ty::I32);
        assert_eq!(value.exprs[3].flow(), None);
        value.validate().unwrap();
    }

    #[test]
    fn display_names_do_not_define_local_identity() {
        let value = fixture();
        assert_eq!(value.locals[0].name, value.locals[1].name);
        assert_ne!(LocalId(0), LocalId(1));
        value.validate().unwrap();
    }

    #[test]
    fn structural_validation_rejects_invalid_references_and_parameters() {
        let mut value = fixture();
        value.body.tail = Some(ExprId(1));
        assert!(value.validate().unwrap_err().contains("invalid expression"));
        value.body.tail = None;
        value.params.push(LocalId(0));
        assert!(
            value
                .validate()
                .unwrap_err()
                .contains("duplicate parameter")
        );
        value.params = vec![LocalId(2)];
        assert!(value.validate().unwrap_err().contains("invalid local"));
    }

    #[test]
    fn structural_validation_rejects_call_arity() {
        let mut value = fixture();
        value.exprs.push(TypedExpr {
            ty: Ty::Unit,
            completion: Completion::Returns,
            kind: ExprKind::Call {
                name: "g".into(),
                args: vec![],
                signature: Signature {
                    ref_contract: Default::default(),
                    params: vec![Ty::I32],
                    result: Ty::Unit,
                },
            },
        });
        assert!(value.validate().unwrap_err().contains("invalid call arity"));
    }

    #[test]
    fn loop_body_and_break_operand_references_are_checked() {
        let mut value = fixture();
        value.exprs.push(TypedExpr {
            ty: Ty::Never,
            completion: Completion::Never,
            kind: ExprKind::Loop {
                id: LoopId(0),
                body: Block {
                    statements: vec![],
                    tail: Some(ExprId(99)),
                },
            },
        });
        assert!(
            value
                .validate()
                .unwrap_err()
                .contains("invalid expression 99")
        );
        value.exprs[1].kind = ExprKind::Break {
            target: LoopId(0),
            value: Some(ExprId(99)),
        };
        assert!(
            value
                .validate()
                .unwrap_err()
                .contains("invalid expression 99")
        );
        value.exprs[1].kind = ExprKind::Break {
            target: LoopId(0),
            value: Some(ExprId(0)),
        };
        assert_eq!(value.exprs[1].flow(), None);
        value.validate().unwrap();
        value.exprs[1].kind = ExprKind::Continue { target: LoopId(0) };
        assert_eq!(value.exprs[1].flow(), None);
        value.validate().unwrap();
    }

    #[test]
    fn while_body_references_are_checked() {
        let mut value = fixture();
        value.body.statements.push(Stmt::While {
            id: LoopId(0),
            condition: ExprId(0),
            body: Block {
                statements: vec![Stmt::Expr(ExprId(99))],
                tail: None,
            },
        });
        assert!(
            value
                .validate()
                .unwrap_err()
                .contains("invalid expression 99")
        );
    }
}
