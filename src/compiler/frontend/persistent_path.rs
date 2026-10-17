//! Stable, checked structural declaration descriptors for cache encoding only.
//! Process-local handles and diagnostic display text are never persisted.
use super::super::expansion_budget::{Counter, Storage};
use super::super::module_identity::{CatalogModule, ModuleSegment};
use super::super::token_session::{BudgetVec, TokenError};
use super::*;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PersistentCrate {
    Source,
    Core,
}
#[derive(Clone, Copy, Debug)]
pub enum PersistentModule<'a> {
    Source(&'a str),
    Catalog(CatalogModule),
}
#[derive(Clone, Copy, Debug)]
pub enum PersistentDeclItem<'a> {
    Name(&'a str),
    AssociatedType { trait_name: &'a str, name: &'a str },
    Impl,
    Catalog(tr::CatalogItem),
}
pub struct PersistentDeclPath<'a> {
    root: PersistentCrate,
    reverse_modules: BudgetVec<PersistentModule<'a>>,
    item: PersistentDeclItem<'a>,
    kind: ir::DeclKind,
}
impl<'a> PersistentDeclPath<'a> {
    pub fn crate_root(&self) -> PersistentCrate {
        self.root
    }
    pub fn modules(&self) -> impl DoubleEndedIterator<Item = PersistentModule<'a>> + '_ {
        self.reverse_modules.as_slice().iter().rev().copied()
    }
    pub fn item(&self) -> PersistentDeclItem<'a> {
        self.item
    }
    pub fn kind(&self) -> ir::DeclKind {
        self.kind
    }
}
impl PlanCtx {
    pub fn persistent_decl_path(&self, id: ir::DeclId) -> Result<PersistentDeclPath<'_>, String> {
        self.check_budget(&self.budget)?;
        let info = self.declaration(id)?;
        let names = self.program.names.as_ref();
        let modules = self.program.modules.as_ref();
        modules.check_names(names).map_err(module_diagnostic)?;
        let root = modules
            .root_of(info.path.module)
            .map_err(module_diagnostic)?;
        let root_kind = if root == modules.source_root().map_err(module_diagnostic)? {
            PersistentCrate::Source
        } else if root == modules.core_root() {
            PersistentCrate::Core
        } else {
            return Err("contract error: unknown persistent declaration crate root".into());
        };
        let mut reverse_modules = BudgetVec::new(names.budget(), Storage::Temporary)
            .map_err(|error| super::super::source::SourceError::from_token(error).to_string())?;
        let mut current = info.path.module;
        while current != root {
            names.budget().charge(Counter::Work, 1).map_err(|error| {
                super::super::source::SourceError::from_token(TokenError::Budget(error)).to_string()
            })?;
            let segment = match modules.segment(current).map_err(module_diagnostic)? {
                ModuleSegment::Source(ident) => {
                    let data = names.data(ident).map_err(native_adapter::name_diagnostic)?;
                    PersistentModule::Source(
                        names
                            .atom_text(data.atom)
                            .map_err(native_adapter::name_diagnostic)?,
                    )
                }
                ModuleSegment::Catalog(module) => PersistentModule::Catalog(module),
                ModuleSegment::SourceRoot => {
                    return Err(
                        "contract error: premature source root in declaration ancestry".into(),
                    );
                }
            };
            reverse_modules.push(segment).map_err(|error| {
                super::super::source::SourceError::from_token(error).to_string()
            })?;
            current = modules
                .parent(current)
                .map_err(module_diagnostic)?
                .ok_or("contract error: truncated declaration module ancestry")?;
        }
        let item = match info.path.item {
            DeclItemName::Source(ident) => {
                if root_kind != PersistentCrate::Source {
                    return Err("contract error: source item in catalog crate".into());
                }
                let data = names.data(ident).map_err(native_adapter::name_diagnostic)?;
                let name = names
                    .atom_text(data.atom)
                    .map_err(native_adapter::name_diagnostic)?;
                if info.kind == ir::DeclKind::AssociatedType {
                    let owner = self.declaration(
                        info.owner
                            .ok_or("associated member has no persistent owner")?,
                    )?;
                    if owner.kind != ir::DeclKind::Trait || owner.path.module != info.path.module {
                        return Err("associated member persistent owner mismatch".into());
                    }
                    let DeclItemName::Source(trait_name) = owner.path.item else {
                        return Err("source associated member has non-source owner".into());
                    };
                    let owner_data = names
                        .data(trait_name)
                        .map_err(native_adapter::name_diagnostic)?;
                    let trait_name = names
                        .atom_text(owner_data.atom)
                        .map_err(native_adapter::name_diagnostic)?;
                    PersistentDeclItem::AssociatedType { trait_name, name }
                } else {
                    PersistentDeclItem::Name(name)
                }
            }
            DeclItemName::Impl => {
                if root_kind != PersistentCrate::Source
                    || !matches!(info.kind, ir::DeclKind::Impl | ir::DeclKind::TraitImpl)
                {
                    return Err("contract error: invalid implementation descriptor".into());
                }
                PersistentDeclItem::Impl
            }
            DeclItemName::Catalog(item) => {
                if root_kind != PersistentCrate::Core {
                    return Err("contract error: catalog item in source crate".into());
                }
                PersistentDeclItem::Catalog(item)
            }
        };
        Ok(PersistentDeclPath {
            root: root_kind,
            reverse_modules,
            item,
            kind: info.kind,
        })
    }
    pub fn is_entry_decl(&self, id: ir::DeclId) -> Result<bool, String> {
        self.check_budget(&self.budget)?;
        let info = self.declaration(id)?;
        self.program
            .modules
            .check_names(self.program.names.as_ref())
            .map_err(module_diagnostic)?;
        self.program
            .modules
            .check(info.path.module)
            .map_err(module_diagnostic)?;
        let root = self
            .program
            .modules
            .source_root()
            .map_err(module_diagnostic)?;
        let DeclItemName::Source(name) = info.path.item else {
            return Ok(false);
        };
        let is_main = self.program.symbols.matches(
            self.program.names.as_ref(),
            name,
            super::super::semantic_name::BuiltinAtom::EntryMain,
        )?;
        Ok(info.path.module == root
            && info.kind == ir::DeclKind::Function
            && info.owner.is_none()
            && is_main)
    }
}
