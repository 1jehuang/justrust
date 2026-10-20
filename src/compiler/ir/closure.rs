//! Immutable closure types. The source owner and ordered owner arguments are
//! part of the identity, not just the anonymous declaration's display name.
use super::*;
use crate::compiler::frontend::ClosureShapeSeal;
#[cfg(test)]
#[path = "closure_tests.rs"]
mod tests;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum ClosureKind {
    Fn,
    FnMut,
    FnOnce,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum CaptureKind {
    Value,
    Shared,
    Mut,
}

/// Source authorization for a physical environment field. The place belongs to
/// the canonical parent's checked body, never an alias spelled in the child.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct CaptureSource {
    pub place: Place,
    pub kind: CaptureKind,
}

/// Identity of one anonymous provider's retained source recipe. Unlike a
/// declaration number this cannot be transplanted between independent worlds.
#[derive(Clone, Debug)]
pub struct ClosureRecipeId(Arc<ClosureRecipeIdentity>);
#[derive(Debug)]
struct ClosureRecipeIdentity;
impl ClosureRecipeId {
    pub(in crate::compiler) fn new() -> Self {
        Self(Arc::new(ClosureRecipeIdentity))
    }
}
impl PartialEq for ClosureRecipeId {
    fn eq(&self, other: &Self) -> bool {
        Arc::ptr_eq(&self.0, &other.0)
    }
}
impl Eq for ClosureRecipeId {}
impl std::hash::Hash for ClosureRecipeId {
    fn hash<H: std::hash::Hasher>(&self, state: &mut H) {
        // Identity equality is stronger than this intentionally colliding hash.
        // No raw address is persisted or used as an identity representation.
        std::hash::Hash::hash(&0u8, state);
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct ClosureTy {
    owner: DeclId,
    owner_args: Arc<[Ty]>,
    site: u32,
    provider: DeclId,
    params: Arc<[Ty]>,
    result: Arc<Ty>,
    captures: Arc<[Ty]>,
    sources: Arc<[CaptureSource]>,
    kind: ClosureKind,
    seal: Option<Arc<ClosureShapeSeal>>,
    result_receipt: Option<Arc<ClosureResultReceipt>>,
}

impl ClosureTy {
    #[allow(clippy::too_many_arguments)]
    pub(in crate::compiler) fn new(
        owner: DeclId,
        owner_args: Arc<[Ty]>,
        site: u32,
        provider: DeclId,
        params: Arc<[Ty]>,
        result: Ty,
        captures: Arc<[Ty]>,
        kind: ClosureKind,
    ) -> Self {
        Self {
            owner,
            owner_args,
            site,
            provider,
            params,
            result: Arc::new(result),
            captures,
            sources: Arc::from([]),
            kind,
            seal: None,
            result_receipt: None,
        }
    }
    pub(in crate::compiler) fn with_result_receipt(
        mut self,
        receipt: Option<Arc<ClosureResultReceipt>>,
    ) -> Self {
        self.result_receipt = receipt;
        self
    }
    pub fn result_receipt(&self) -> Option<&Arc<ClosureResultReceipt>> {
        self.result_receipt.as_ref()
    }
    pub(in crate::compiler) fn with_sources(mut self, sources: Arc<[CaptureSource]>) -> Self {
        self.sources = sources;
        self
    }
    pub fn sources(&self) -> &[CaptureSource] {
        &self.sources
    }
    pub(in crate::compiler) fn with_shape_seal(
        mut self,
        seal: Option<Arc<ClosureShapeSeal>>,
    ) -> Self {
        self.seal = seal;
        self
    }
    pub(in crate::compiler) fn shape_seal(&self) -> Option<&Arc<ClosureShapeSeal>> {
        self.seal.as_ref()
    }
    pub(in crate::compiler) fn try_map<E>(
        &self,
        mut map: impl FnMut(&Ty) -> Result<Ty, E>,
    ) -> Result<Self, E> {
        Ok(Self::new(
            self.owner,
            self.owner_args
                .iter()
                .map(&mut map)
                .collect::<Result<Vec<_>, _>>()?
                .into(),
            self.site,
            self.provider,
            self.params
                .iter()
                .map(&mut map)
                .collect::<Result<Vec<_>, _>>()?
                .into(),
            map(&self.result)?,
            self.captures
                .iter()
                .map(&mut map)
                .collect::<Result<Vec<_>, _>>()?
                .into(),
            self.kind,
        )
        .with_sources(self.sources.clone())
        .with_result_receipt(self.result_receipt.clone())
        .with_shape_seal(self.seal.clone()))
    }
    pub(in crate::compiler) fn try_eq<'a, E>(
        &'a self,
        other: &'a Self,
        mut equal: impl FnMut(&'a Ty, &'a Ty) -> Result<bool, E>,
    ) -> Result<bool, E> {
        if self.owner != other.owner
            || self.site != other.site
            || self.provider != other.provider
            || self.kind != other.kind
            || self.sources != other.sources
            || self.seal != other.seal
            || !self.same_result_receipt_shape(other)
            || self.owner_args.len() != other.owner_args.len()
            || self.params.len() != other.params.len()
            || self.captures.len() != other.captures.len()
        {
            return Ok(false);
        }
        for (left, right) in self.types().zip(other.types()) {
            if !equal(left, right)? {
                return Ok(false);
            }
        }
        Ok(true)
    }
    pub fn owner(&self) -> DeclId {
        self.owner
    }
    pub fn owner_args(&self) -> &[Ty] {
        &self.owner_args
    }
    pub fn site(&self) -> u32 {
        self.site
    }
    pub fn provider(&self) -> DeclId {
        self.provider
    }
    pub fn params(&self) -> &[Ty] {
        &self.params
    }
    pub fn result(&self) -> &Ty {
        &self.result
    }
    pub fn captures(&self) -> &[Ty] {
        &self.captures
    }
    pub fn kind(&self) -> ClosureKind {
        self.kind
    }
    pub fn supports(&self, kind: ClosureKind) -> bool {
        matches!(
            (self.kind, kind),
            (ClosureKind::Fn, _)
                | (ClosureKind::FnMut, ClosureKind::FnMut | ClosureKind::FnOnce)
                | (ClosureKind::FnOnce, ClosureKind::FnOnce)
        )
    }
    pub fn same_result_receipt_shape(&self, other: &Self) -> bool {
        match (&self.result_receipt, &other.result_receipt) {
            (None, None) => true,
            (Some(left), Some(right)) => left.same_source_shape(right),
            _ => false,
        }
    }
    /// All semantic children, including the erased runtime signature. Runtime
    /// carrier/layout walks must visit captures only instead.
    pub fn types(&self) -> impl Iterator<Item = &Ty> {
        self.owner_args
            .iter()
            .chain(self.params.iter())
            .chain(std::iter::once(self.result.as_ref()))
            .chain(self.captures.iter())
    }
}
