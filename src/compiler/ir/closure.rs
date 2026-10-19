//! Immutable closure types. The source owner and ordered owner arguments are
//! part of the identity, not just the anonymous declaration's display name.
use super::*;
#[cfg(test)]
#[path = "closure_tests.rs"]
mod tests;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum ClosureKind {
    Fn,
    FnMut,
    FnOnce,
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
    kind: ClosureKind,
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
            kind,
        }
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
        ))
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
