//! Complete declaration namespaces linked without parsing dependency bodies.
use super::*;
use crate::compiler::semantic_path::{AstPath, PathAnchor};

pub(in crate::compiler) struct Namespace {
    local: Arc<LocalExports>,
    trees: BudgetVec<Arc<ExportSubtree>>,
    externs: Arc<[crate::compiler::source::ExternCrate]>,
}
impl Namespace {
    pub(in crate::compiler) fn new(
        local: Arc<LocalExports>,
        trees: BudgetVec<Arc<ExportSubtree>>,
        externs: Arc<[crate::compiler::source::ExternCrate]>,
    ) -> Result<Self, SourceError> {
        if !trees
            .as_slice()
            .iter()
            .any(|tree| tree.local.root == local.root)
        {
            return Err(contract(
                "current macro namespace lacks source completeness",
            ));
        }
        for (index, tree) in trees.as_slice().iter().enumerate() {
            if tree.local.file.parent().is_some()
                || trees.as_slice()[..index]
                    .iter()
                    .any(|prior| prior.local.file == tree.local.file)
            {
                return Err(contract(
                    "duplicate or non-root macro namespace completeness",
                ));
            }
        }
        Ok(Self {
            local,
            trees,
            externs,
        })
    }
    pub(in crate::compiler) fn stores(
        &self,
        budget: &crate::compiler::expansion_budget::ExpansionBudget,
    ) -> Result<BudgetVec<Arc<NameStore>>, SourceError> {
        let mut stores = BudgetVec::new(budget, Storage::Temporary).map_err(token_error)?;
        for tree in self.trees.as_slice() {
            let next = tree.stores(budget)?;
            for store in next.as_slice() {
                stores.push(store.clone()).map_err(token_error)?;
            }
        }
        Ok(stores)
    }
    pub(in crate::compiler) fn snapshot(
        &self,
        budget: &crate::compiler::expansion_budget::ExpansionBudget,
    ) -> Result<crate::compiler::token_session::SourceSnapshot, SourceError> {
        let local = self.local.names.session().sources();
        let file = local.file(0).map_err(token_error)?;
        let root = local.source_crate_root(file).map_err(token_error)?;
        let stores = self.stores(budget)?;
        let mut sources = Vec::new();
        let mut lease = budget
            .reserve(Storage::Temporary, 0)
            .map_err(|error| token_error(error.into()))?;
        for store in stores.as_slice() {
            let source = store.session().sources();
            if source.same_snapshot(local)
                || sources
                    .iter()
                    .any(|old: &crate::compiler::token_session::SourceSnapshot| {
                        old.same_snapshot(source)
                    })
            {
                continue;
            }
            let bytes = (sources.len() + 1)
                .checked_mul(std::mem::size_of::<
                    crate::compiler::token_session::SourceSnapshot,
                >())
                .ok_or_else(|| token_error(TokenError::IdExhausted))?;
            lease
                .resize(bytes as u64)
                .map_err(|error| token_error(error.into()))?;
            sources
                .try_reserve_exact(1)
                .map_err(|_| token_error(budget.poison_allocation().into()))?;
            lease
                .resize(
                    (sources.capacity()
                        * std::mem::size_of::<crate::compiler::token_session::SourceSnapshot>())
                        as u64,
                )
                .map_err(|error| token_error(error.into()))?;
            // Keep the sibling's own joined root. Dropping it would turn a
            // child-file definition into a different apparent source crate.
            sources.push(source.clone());
        }
        let snapshot = local
            .unlinked()
            .linked_with(sources, budget)
            .map_err(token_error)?;
        if root != file {
            snapshot.join_crate_root(root).map_err(token_error)
        } else {
            Ok(snapshot)
        }
    }
    pub(in crate::compiler) fn select<'a>(
        &'a self,
        path: &AstPath,
        names: &NameStoreBuilder,
    ) -> Result<Option<ExportSelection<'a>>, DriverError> {
        path.check(names)?;
        let (root, name) = match (path.anchor(), path.segments()) {
            (PathAnchor::Crate, [name]) => (self.local.root, *name),
            (PathAnchor::DefiningCrate(anchor), [name]) => (anchor.root(names.session())?, *name),
            (PathAnchor::Relative, [krate, name]) => {
                let alias = names.spelling(*krate)?;
                let Some(index) = self.externs.iter().position(|entry| entry.name == alias) else {
                    return Err(DriverError::Unsupported {
                        span: names.span(*name)?,
                        message: "exported macro module/import path requires configured namespace authority",
                    });
                };
                let file = crate::compiler::keys::SourceFile::from_words(&[index as u64 + 2])
                    .map_err(|_| TokenError::InvalidHandle)?;
                let Some(tree) = self
                    .trees
                    .as_slice()
                    .iter()
                    .find(|tree| tree.local.file == file)
                else {
                    return Err(TokenError::InvalidHandle.into());
                };
                (tree.local.root, *name)
            }
            _ => {
                let Some(name) = path.segments().last() else {
                    return Ok(None);
                };
                return Err(DriverError::Unsupported {
                    span: names.span(*name)?,
                    message: "this exported macro module/import path requires declaration import resolution",
                });
            }
        };
        let Some(tree) = self
            .trees
            .as_slice()
            .iter()
            .find(|tree| tree.local.root == root)
        else {
            return Err(TokenError::InvalidHandle.into());
        };
        tree.select(root, name, names)
    }
}
