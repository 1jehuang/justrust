//! Checked lexical module ownership, independent of declaration snapshots.
//!
//! These process-local handles are not persistent cache identities, source
//! provenance, or authority to publish checked body metadata. Aliases reuse a
//! canonical module handle. An absent child is not a resolver completeness fact.
use std::collections::hash_map::RandomState;
use std::fmt;
use std::hash::BuildHasher;
use std::mem::size_of;
use std::num::NonZeroU64;
use std::sync::Arc;

use super::expansion_budget::{BudgetFailure, ByteLease, Counter, ExpansionBudget, Storage};
use super::owner_path::{Issued, OwnerDomain};
use super::semantic_name::NameStoreBuilder;
use super::semantic_name::{AstIdent, LookupClass, NameError, NameKey, Namespace};
use super::semantic_path::NameAccess;
use super::token_session::{BudgetVec, CrateOwners, SessionId, TokenError};

/// An opaque module in one arena. Equality alone never validates ownership.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct ModuleId {
    owner: NonZeroU64,
    index: u32,
}

/// Closed catalog module descriptors, not names parsed from diagnostic strings.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum CatalogModule {
    Root,
    Marker,
    Clone,
    Ops,
}
impl CatalogModule {
    fn index(self) -> usize {
        match self {
            Self::Root => 0,
            Self::Marker => 1,
            Self::Clone => 2,
            Self::Ops => 3,
        }
    }
}

/// Original lexical segment for checked diagnostic/structural traversal.
/// Rendering a source occurrence additionally requires its exact NameStore.
#[derive(Clone, Copy, Debug)]
pub enum ModuleSegment {
    SourceRoot,
    Catalog(CatalogModule),
    Source(AstIdent),
}

#[derive(Debug)]
pub enum ModuleError {
    Token(TokenError),
    Name(NameError),
    ForeignOwner,
    InvalidHandle,
    SourceUnavailable,
    ForeignNames,
    InvalidParent,
    InvalidRestriction,
}
impl From<TokenError> for ModuleError {
    fn from(error: TokenError) -> Self {
        Self::Token(error)
    }
}
impl From<NameError> for ModuleError {
    fn from(error: NameError) -> Self {
        Self::Name(error)
    }
}
impl From<BudgetFailure> for ModuleError {
    fn from(error: BudgetFailure) -> Self {
        Self::Token(TokenError::Budget(error))
    }
}
impl fmt::Display for ModuleError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Token(error) => write!(f, "module authority: {error}"),
            Self::Name(error) => write!(f, "module authority: {error}"),
            Self::ForeignOwner => f.write_str("foreign module arena"),
            Self::InvalidHandle => f.write_str("invalid module handle"),
            Self::SourceUnavailable => f.write_str("catalog-only arena has no source root"),
            Self::ForeignNames => f.write_str("module arena belongs to a different name store"),
            Self::InvalidParent => {
                f.write_str("source module cannot be attached to a catalog module")
            }
            Self::InvalidRestriction => {
                f.write_str("visibility scope is not an ancestor of its definition")
            }
        }
    }
}
impl std::error::Error for ModuleError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Token(error) => Some(error),
            Self::Name(error) => Some(error),
            _ => None,
        }
    }
}

#[derive(Clone, Copy, Debug)]
struct ModuleNode {
    segment: ModuleSegment,
    parent: Option<ModuleId>,
    root: ModuleId,
    depth: u32,
    name: Option<NameKey>,
    hash: u64,
}

// Backing is dropped before its accounting. Rebuilds retain both old and new
// allocations until replacement, including allocator-rounded capacity.
struct ChildIndex {
    slots: Vec<Option<u32>>,
    _lease: ByteLease,
}
impl ChildIndex {
    fn new(budget: &ExpansionBudget, capacity: usize) -> Result<Self, ModuleError> {
        let bytes = (capacity as u64)
            .checked_mul(size_of::<Option<u32>>() as u64)
            .ok_or(TokenError::IdExhausted)?;
        budget.charge(Counter::Work, capacity as u64)?;
        let mut lease = budget.reserve(Storage::Retained, bytes)?;
        let mut slots = Vec::new();
        if slots.try_reserve_exact(capacity).is_err() {
            return Err(budget.poison_allocation().into());
        }
        let actual = (slots.capacity() as u64)
            .checked_mul(size_of::<Option<u32>>() as u64)
            .ok_or(TokenError::IdExhausted)?;
        lease.resize(actual)?;
        slots.resize(capacity, None);
        Ok(Self {
            slots,
            _lease: lease,
        })
    }
}

pub struct ModuleArena {
    owner: NonZeroU64,
    /// Explicit crate-scoped owner set; exactly `owner` today.
    owners: CrateOwners<NonZeroU64>,
    /// Frozen sibling file arenas of the same crate (PARSE-SPLIT.md step 1),
    /// sorted by owner. Their modules are served by them; others are foreign.
    linked: Arc<[(NonZeroU64, Arc<ModuleArena>)]>,
    /// Whether this file arena adopted a linked crate root (`join_crate`).
    joined: bool,
    source_names: Option<SessionId>,
    source_root: Option<ModuleId>,
    catalog: [ModuleId; 4],
    nodes: BudgetVec<ModuleNode>,
    children: ChildIndex,
    child_count: usize,
    hasher: RandomState,
    budget: ExpansionBudget,
    _lease: ByteLease,
}
impl fmt::Debug for ModuleArena {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ModuleArena")
            .field("owner", &self.owner)
            .field("source_root", &self.source_root)
            .field("modules", &self.nodes.len())
            .finish()
    }
}
#[derive(Debug)]
pub struct ModuleArenaBuilder {
    arena: ModuleArena,
}

impl ModuleArenaBuilder {
    /// A source arena owned by a child issued from the session's own issuer
    /// (the graph route: PARSE's issuer reaches here through the builder).
    pub fn for_builder(names: &mut NameStoreBuilder) -> Result<Self, ModuleError> {
        let owner = names.issue_owner(OwnerDomain::ModuleArena)?;
        Self::new(names.budget().clone(), Some(names.session().id()), owner)
    }
    /// A source arena for direct tooling over any name view: its owner is
    /// issued under a fresh standalone root, never a compiler counter.
    /// Direct tooling passes an owner it was issued (for example by
    /// `owner_path::standalone_issued`); this never mints one itself.
    pub fn for_source(names: &impl NameAccess, owner: Issued) -> Result<Self, ModuleError> {
        Self::new(names.budget().clone(), Some(names.session().id()), owner)
    }
    pub fn for_catalog(budget: ExpansionBudget, owner: Issued) -> Result<Self, ModuleError> {
        Self::new(budget, None, owner)
    }
    fn new(
        budget: ExpansionBudget,
        source_names: Option<SessionId>,
        owner: Issued,
    ) -> Result<Self, ModuleError> {
        budget.check()?;
        let owner = owner.tag().bits();
        // Stable std cannot fallibly allocate this fixed-size Arc control block.
        let bytes = size_of::<ModuleArena>()
            .checked_add(2 * size_of::<usize>())
            .ok_or(TokenError::IdExhausted)?;
        let lease = budget.reserve(Storage::Retained, bytes as u64)?;
        let mut nodes = BudgetVec::new(&budget, Storage::Retained)?;
        let core = ModuleId { owner, index: 0 };
        budget.charge(Counter::Work, 15)?;
        nodes.push(ModuleNode {
            segment: ModuleSegment::Catalog(CatalogModule::Root),
            parent: None,
            root: core,
            depth: 0,
            name: None,
            hash: 0,
        })?;
        for (index, module) in [
            CatalogModule::Marker,
            CatalogModule::Clone,
            CatalogModule::Ops,
        ]
        .into_iter()
        .enumerate()
        {
            nodes.push(ModuleNode {
                segment: ModuleSegment::Catalog(module),
                parent: Some(core),
                root: core,
                depth: 1,
                name: None,
                hash: 0,
            })?;
            debug_assert_eq!(nodes.len(), index + 2);
        }
        let source_root = if source_names.is_some() {
            let root = ModuleId { owner, index: 4 };
            nodes.push(ModuleNode {
                segment: ModuleSegment::SourceRoot,
                parent: None,
                root,
                depth: 0,
                name: None,
                hash: 0,
            })?;
            Some(root)
        } else {
            None
        };
        let children = ChildIndex::new(&budget, 0)?;
        Ok(Self {
            arena: ModuleArena {
                owner,
                owners: CrateOwners::single(owner),
                linked: Arc::from([]),
                joined: false,
                source_names,
                source_root,
                catalog: [
                    core,
                    ModuleId { owner, index: 1 },
                    ModuleId { owner, index: 2 },
                    ModuleId { owner, index: 3 },
                ],
                nodes,
                children,
                child_count: 0,
                hasher: RandomState::new(),
                budget,
                _lease: lease,
            },
        })
    }
    pub fn intern_child(
        &mut self,
        names: &impl NameAccess,
        parent: ModuleId,
        ident: AstIdent,
    ) -> Result<ModuleId, ModuleError> {
        let key = self.arena.checked_child_key(names, parent, ident)?;
        let hash = self.arena.hasher.hash_one((parent, key));
        self.intern_key(parent, ident, key, hash)
    }
    fn intern_key(
        &mut self,
        parent: ModuleId,
        ident: AstIdent,
        key: NameKey,
        hash: u64,
    ) -> Result<ModuleId, ModuleError> {
        if let Some(id) = self.arena.find_child(parent, key, hash)? {
            return Ok(id);
        }
        let count = self
            .arena
            .child_count
            .checked_add(1)
            .ok_or(TokenError::IdExhausted)?;
        if count.checked_mul(2).ok_or(TokenError::IdExhausted)? > self.arena.children.slots.len() {
            let capacity = self
                .arena
                .children
                .slots
                .len()
                .max(4)
                .checked_mul(2)
                .ok_or(TokenError::IdExhausted)?;
            let mut next = ChildIndex::new(&self.arena.budget, capacity)?;
            for (index, node) in self.arena.nodes.as_slice().iter().enumerate() {
                self.arena.budget.charge(Counter::Work, 1)?;
                if node.name.is_some() {
                    Self::insert_index(&self.arena.budget, &mut next, node.hash, index as u32)?;
                }
            }
            self.arena.children = next;
        }
        let parent_node = *self.arena.node(parent)?;
        let depth = parent_node
            .depth
            .checked_add(1)
            .ok_or(TokenError::IdExhausted)?;
        let index = self
            .arena
            .budget
            .checked_u32(self.arena.nodes.len() as u64)?;
        self.arena.budget.charge(Counter::Work, 3)?;
        self.arena.nodes.push(ModuleNode {
            segment: ModuleSegment::Source(ident),
            parent: Some(parent),
            root: parent_node.root,
            depth,
            name: Some(key),
            hash,
        })?;
        Self::insert_index(&self.arena.budget, &mut self.arena.children, hash, index)?;
        self.arena.child_count = count;
        Ok(ModuleId {
            owner: self.arena.owner,
            index,
        })
    }
    fn insert_index(
        budget: &ExpansionBudget,
        index: &mut ChildIndex,
        hash: u64,
        value: u32,
    ) -> Result<(), ModuleError> {
        let mask = index
            .slots
            .len()
            .checked_sub(1)
            .ok_or(ModuleError::InvalidHandle)?;
        let mut slot = hash as usize & mask;
        for _ in 0..index.slots.len() {
            budget.charge(Counter::Work, 1)?;
            if index.slots[slot].is_none() {
                index.slots[slot] = Some(value);
                return Ok(());
            }
            slot = (slot + 1) & mask;
        }
        Err(ModuleError::InvalidHandle)
    }
    /// Link the crate's frozen sibling file arenas. Explicit and fixed once
    /// linked: only these owners join this arena's own.
    pub fn link_crate_files(&mut self, arenas: Vec<Arc<ModuleArena>>) -> Result<(), ModuleError> {
        if !self.arena.linked.is_empty() {
            return Err(TokenError::InvalidInput("crate sibling arenas already linked").into());
        }
        self.arena
            .budget
            .charge(Counter::Work, arenas.len() as u64)?;
        let mut linked: Vec<(NonZeroU64, Arc<ModuleArena>)> = Vec::new();
        linked
            .try_reserve_exact(arenas.len())
            .map_err(|_| self.arena.budget.poison_allocation())?;
        for arena in arenas {
            if arena.owner == self.arena.owner {
                return Err(TokenError::InvalidInput("an arena cannot link itself").into());
            }
            linked.push((arena.owner, arena));
        }
        linked.sort_unstable_by_key(|(owner, _)| *owner);
        if linked.windows(2).any(|pair| pair[0].0 == pair[1].0) {
            return Err(TokenError::InvalidInput("duplicate linked arena owner").into());
        }
        self.arena.linked = linked.into();
        Ok(())
    }
    /// Place this file arena into the crate whose root file arena `root` is
    /// already linked: the crate source root and catalog modules become the
    /// root's, so every file of one crate shares one module tree.
    pub fn join_crate(&mut self, root: &ModuleArena) -> Result<(), ModuleError> {
        self.arena.budget.charge(Counter::Work, 1)?;
        if self.arena.joined {
            return Err(TokenError::InvalidInput("arena already joined a crate").into());
        }
        if root.joined
            || self
                .arena
                .linked
                .binary_search_by_key(&root.owner, |(owner, _)| *owner)
                .is_err()
        {
            return Err(ModuleError::ForeignOwner);
        }
        self.arena.source_root = Some(root.source_root()?);
        self.arena.catalog = root.catalog;
        self.arena.joined = true;
        Ok(())
    }
    pub fn freeze(self) -> Result<Arc<ModuleArena>, ModuleError> {
        self.arena.budget.check()?;
        Ok(Arc::new(self.arena))
    }
    pub fn source_root(&self) -> Result<ModuleId, ModuleError> {
        self.arena.source_root()
    }
    pub fn core_root(&self) -> ModuleId {
        self.arena.core_root()
    }
    pub fn catalog_module(&self, module: CatalogModule) -> ModuleId {
        self.arena.catalog_module(module)
    }
    pub fn budget(&self) -> &ExpansionBudget {
        self.arena.budget()
    }
    pub fn check(&self, id: ModuleId) -> Result<(), ModuleError> {
        self.arena.check(id)
    }
    pub fn check_names(&self, names: &impl NameAccess) -> Result<(), ModuleError> {
        self.arena.check_names(names)
    }
    pub fn segment(&self, id: ModuleId) -> Result<ModuleSegment, ModuleError> {
        self.arena.segment(id)
    }
    pub fn name(&self, id: ModuleId) -> Result<Option<AstIdent>, ModuleError> {
        self.arena.name(id)
    }
    pub fn parent(&self, id: ModuleId) -> Result<Option<ModuleId>, ModuleError> {
        self.arena.parent(id)
    }
    pub fn root_of(&self, id: ModuleId) -> Result<ModuleId, ModuleError> {
        self.arena.root_of(id)
    }
    pub fn is_descendant(&self, module: ModuleId, ancestor: ModuleId) -> Result<bool, ModuleError> {
        self.arena.is_descendant(module, ancestor)
    }
    pub fn lookup_child(
        &self,
        names: &impl NameAccess,
        parent: ModuleId,
        ident: AstIdent,
    ) -> Result<Option<ModuleId>, ModuleError> {
        self.arena.lookup_child(names, parent, ident)
    }
}
impl ModuleArena {
    pub fn source_root(&self) -> Result<ModuleId, ModuleError> {
        self.budget.check()?;
        self.source_root.ok_or(ModuleError::SourceUnavailable)
    }
    pub fn core_root(&self) -> ModuleId {
        self.catalog[0]
    }
    pub fn catalog_module(&self, module: CatalogModule) -> ModuleId {
        self.catalog[module.index()]
    }
    pub fn budget(&self) -> &ExpansionBudget {
        &self.budget
    }
    fn node(&self, id: ModuleId) -> Result<&ModuleNode, ModuleError> {
        self.budget.charge(Counter::Work, 1)?;
        if !self.owners.admits(id.owner) {
            return match self
                .linked
                .binary_search_by_key(&id.owner, |(owner, _)| *owner)
            {
                Ok(index) => self.linked[index].1.node(id),
                Err(_) => Err(ModuleError::ForeignOwner),
            };
        }
        self.nodes
            .as_slice()
            .get(id.index as usize)
            .ok_or(ModuleError::InvalidHandle)
    }
    pub fn check(&self, id: ModuleId) -> Result<(), ModuleError> {
        self.node(id).map(|_| ())
    }
    pub fn segment(&self, id: ModuleId) -> Result<ModuleSegment, ModuleError> {
        Ok(self.node(id)?.segment)
    }
    pub fn name(&self, id: ModuleId) -> Result<Option<AstIdent>, ModuleError> {
        Ok(match self.segment(id)? {
            ModuleSegment::Source(ident) => Some(ident),
            _ => None,
        })
    }
    /// Validate the exact name authority even when no source child exists.
    pub fn check_names(&self, names: &impl NameAccess) -> Result<(), ModuleError> {
        self.budget.charge(Counter::Work, 1)?;
        let expected = self.source_names.ok_or(ModuleError::SourceUnavailable)?;
        if expected != names.session().id() {
            return Err(ModuleError::ForeignNames);
        }
        names.budget().check()?;
        Ok(())
    }
    pub fn parent(&self, id: ModuleId) -> Result<Option<ModuleId>, ModuleError> {
        Ok(self.node(id)?.parent)
    }
    pub fn root_of(&self, id: ModuleId) -> Result<ModuleId, ModuleError> {
        Ok(self.node(id)?.root)
    }
    pub fn is_descendant(&self, module: ModuleId, ancestor: ModuleId) -> Result<bool, ModuleError> {
        let mut module_node = *self.node(module)?;
        let ancestor_node = *self.node(ancestor)?;
        if module_node.root != ancestor_node.root || module_node.depth < ancestor_node.depth {
            return Ok(false);
        }
        let mut current = module;
        while module_node.depth > ancestor_node.depth {
            current = module_node.parent.ok_or(ModuleError::InvalidHandle)?;
            module_node = *self.node(current)?;
        }
        Ok(current == ancestor)
    }
    fn checked_child_key(
        &self,
        names: &impl NameAccess,
        parent: ModuleId,
        ident: AstIdent,
    ) -> Result<NameKey, ModuleError> {
        self.check(parent)?;
        self.check_names(names)?;
        // The sealed NameAccess implementations expose the same issuing ledger.
        names.check(ident)?;
        if self.root_of(parent)? != self.source_root()? {
            return Err(ModuleError::InvalidParent);
        }
        Ok(names.key(ident, LookupClass::Item(Namespace::Type))?)
    }
    fn find_child(
        &self,
        parent: ModuleId,
        key: NameKey,
        hash: u64,
    ) -> Result<Option<ModuleId>, ModuleError> {
        if self.children.slots.is_empty() {
            return Ok(None);
        }
        let mask = self.children.slots.len() - 1;
        let mut slot = hash as usize & mask;
        for _ in 0..self.children.slots.len() {
            self.budget.charge(Counter::Work, 1)?;
            let Some(index) = self.children.slots[slot] else {
                return Ok(None);
            };
            let node = &self.nodes.as_slice()[index as usize];
            if node.hash == hash && node.parent == Some(parent) && node.name == Some(key) {
                return Ok(Some(ModuleId {
                    owner: self.owner,
                    index,
                }));
            }
            slot = (slot + 1) & mask;
        }
        Err(ModuleError::InvalidHandle)
    }
    pub fn lookup_child(
        &self,
        names: &impl NameAccess,
        parent: ModuleId,
        ident: AstIdent,
    ) -> Result<Option<ModuleId>, ModuleError> {
        let key = self.checked_child_key(names, parent, ident)?;
        if let Some(found) = self.find_child(parent, key, self.hasher.hash_one((parent, key)))? {
            return Ok(Some(found));
        }
        // A child interned by a linked sibling file (for example the file
        // that declares `mod child;` under this parent).
        for (_, arena) in self.linked.iter() {
            self.budget.charge(Counter::Work, 1)?;
            if let Some(found) =
                arena.find_child(parent, key, arena.hasher.hash_one((parent, key)))?
            {
                return Ok(Some(found));
            }
        }
        Ok(None)
    }
}

#[cfg(test)]
mod tests {
    use super::super::expansion_budget::ExpansionLimits;
    use super::super::semantic_name::NameStoreBuilder;
    use super::super::token::Edition;
    use super::super::token_session::{SourceSnapshot, SpanId, TokenSession};
    use super::*;
    use std::path::Path;

    fn names(budget: ExpansionBudget) -> (NameStoreBuilder, SpanId) {
        let source = SourceSnapshot::from_text(
            Path::new("modules.rs"),
            "",
            &budget,
            crate::compiler::owner_path::standalone_issued(
                &budget,
                crate::compiler::owner_path::OwnerDomain::SourceSnapshot,
            )
            .unwrap(),
        )
        .unwrap();
        let original = source.span(source.file(0).unwrap(), 0, 0).unwrap();
        let (mut session, session_cap) = TokenSession::standalone(source, budget).unwrap();
        let origin = session.original_span(original).unwrap();
        (
            NameStoreBuilder::new(session, session_cap, Edition::E2024).unwrap(),
            origin,
        )
    }

    #[test]
    fn empty_source_owner_is_checked_before_and_after_name_freeze() {
        let (names, _) = names(ExpansionBudget::default());
        let (foreign, _) = self::names(ExpansionBudget::default());
        let builder = ModuleArenaBuilder::for_source(
            &names,
            crate::compiler::owner_path::standalone_issued(
                &names.budget(),
                crate::compiler::owner_path::OwnerDomain::ModuleArena,
            )
            .unwrap(),
        )
        .unwrap();
        builder.check_names(&names).unwrap();
        assert!(matches!(
            builder.check_names(&foreign),
            Err(ModuleError::ForeignNames)
        ));
        let modules = builder.freeze().unwrap();
        let frozen = names.freeze().unwrap();
        modules.check_names(frozen.as_ref()).unwrap();
        assert!(matches!(
            modules.check_names(&foreign),
            Err(ModuleError::ForeignNames)
        ));
        let catalog = ModuleArenaBuilder::for_catalog(
            ExpansionBudget::default(),
            crate::compiler::owner_path::standalone_issued(
                &ExpansionBudget::default(),
                crate::compiler::owner_path::OwnerDomain::ModuleArena,
            )
            .unwrap(),
        )
        .unwrap();
        assert!(matches!(
            catalog.check_names(&foreign),
            Err(ModuleError::SourceUnavailable)
        ));
    }

    #[test]
    fn private_forged_indices_and_foreign_owners_are_not_public_roots() {
        let builder = ModuleArenaBuilder::for_catalog(
            ExpansionBudget::default(),
            crate::compiler::owner_path::standalone_issued(
                &ExpansionBudget::default(),
                crate::compiler::owner_path::OwnerDomain::ModuleArena,
            )
            .unwrap(),
        )
        .unwrap();
        let core = builder.core_root();
        assert!(matches!(
            builder.check(ModuleId {
                index: u32::MAX,
                ..core
            }),
            Err(ModuleError::InvalidHandle)
        ));
        let foreign = ModuleArenaBuilder::for_catalog(
            ExpansionBudget::default(),
            crate::compiler::owner_path::standalone_issued(
                &ExpansionBudget::default(),
                crate::compiler::owner_path::OwnerDomain::ModuleArena,
            )
            .unwrap(),
        )
        .unwrap();
        assert!(matches!(
            builder.is_descendant(core, foreign.core_root()),
            Err(ModuleError::ForeignOwner)
        ));
        assert_eq!(builder.parent(core).unwrap(), None);
        assert!(matches!(
            builder.source_root(),
            Err(ModuleError::SourceUnavailable)
        ));
    }

    #[test]
    fn linked_sibling_arena_serves_its_modules_and_rejects_outsiders() {
        let arena = |names: &NameStoreBuilder| {
            ModuleArenaBuilder::for_source(
                names,
                crate::compiler::owner_path::standalone_issued(
                    &names.budget(),
                    crate::compiler::owner_path::OwnerDomain::ModuleArena,
                )
                .unwrap(),
            )
            .unwrap()
        };
        let (mut b_names, origin) = names(ExpansionBudget::default());
        let mut b = arena(&b_names);
        let b_root = b.source_root().unwrap();
        let child = b_names.intern_root("child", origin).unwrap();
        let b_child = b.intern_child(&b_names, b_root, child).unwrap();
        let b = b.freeze().unwrap();
        let (a_names, _) = names(ExpansionBudget::default());
        let outside = arena(&a_names).freeze().unwrap();
        let mut a = arena(&a_names);
        a.link_crate_files(vec![b.clone()]).unwrap();
        assert!(a.link_crate_files(vec![b.clone()]).is_err());
        assert_eq!(a.parent(b_child).unwrap(), Some(b_root));
        assert!(matches!(
            a.check(outside.source_root().unwrap()),
            Err(ModuleError::ForeignOwner)
        ));
    }

    #[test]
    fn forced_hash_collisions_keep_exact_parent_and_name_identity() {
        let (mut names, origin) = names(ExpansionBudget::default());
        let mut builder = ModuleArenaBuilder::for_source(
            &names,
            crate::compiler::owner_path::standalone_issued(
                &names.budget(),
                crate::compiler::owner_path::OwnerDomain::ModuleArena,
            )
            .unwrap(),
        )
        .unwrap();
        let root = builder.source_root().unwrap();
        let mut expected = Vec::new();
        for index in 0..64 {
            let ident = names.intern_root(&format!("m{index}"), origin).unwrap();
            let key = builder
                .arena
                .checked_child_key(&names, root, ident)
                .unwrap();
            let module = builder.intern_key(root, ident, key, 7).unwrap();
            expected.push((ident, key, module));
        }
        for (ident, key, module) in expected {
            assert_eq!(
                builder.arena.find_child(root, key, 7).unwrap(),
                Some(module)
            );
            assert_eq!(builder.intern_key(root, ident, key, 7).unwrap(), module);
            let occurrence = builder.name(module).unwrap().unwrap();
            assert_eq!(
                names
                    .key(occurrence, LookupClass::Item(Namespace::Type))
                    .unwrap(),
                key
            );
        }
    }

    #[test]
    fn allocation_failure_and_last_arc_release_preserve_live_accounting() {
        let zero = ExpansionBudget::new(ExpansionLimits {
            bytes: 0,
            ..ExpansionLimits::default()
        });
        assert!(matches!(
            ModuleArenaBuilder::for_catalog(
                zero.clone(),
                crate::compiler::owner_path::standalone_issued(
                    &ExpansionBudget::default(),
                    crate::compiler::owner_path::OwnerDomain::ModuleArena
                )
                .unwrap()
            ),
            Err(ModuleError::Token(TokenError::Budget(_)))
        ));
        assert_eq!(zero.snapshot().retained_bytes, 0);
        let budget = ExpansionBudget::default();
        let modules = ModuleArenaBuilder::for_catalog(
            budget.clone(),
            crate::compiler::owner_path::standalone_issued(
                &budget.clone(),
                crate::compiler::owner_path::OwnerDomain::ModuleArena,
            )
            .unwrap(),
        )
        .unwrap()
        .freeze()
        .unwrap();
        let clone = Arc::clone(&modules);
        let retained = budget.snapshot().retained_bytes;
        assert!(retained > 0);
        drop(modules);
        assert_eq!(budget.snapshot().retained_bytes, retained);
        drop(clone);
        assert_eq!(budget.snapshot().retained_bytes, 0);
    }

    #[test]
    fn poisoned_owner_association_preserves_first_failure() {
        let (names, _) = names(ExpansionBudget::default());
        let modules = ModuleArenaBuilder::for_source(
            &names,
            crate::compiler::owner_path::standalone_issued(
                &names.budget(),
                crate::compiler::owner_path::OwnerDomain::ModuleArena,
            )
            .unwrap(),
        )
        .unwrap()
        .freeze()
        .unwrap();
        let error = names
            .budget()
            .charge(
                Counter::Work,
                ExpansionLimits::default().counters[Counter::Work as usize],
            )
            .unwrap_err();
        let Err(ModuleError::Token(TokenError::Budget(actual))) = modules.check_names(&names)
        else {
            panic!("lost budget failure")
        };
        assert_eq!(actual, error);
    }
}
