//! `#[macro_use] mod name;` on an external module file. The child's textual
//! macro frontier comes from its declaration-only SOURCE output (reached
//! through the crate's export completeness tree, never the child's parse),
//! and is re-defined at the parent's frontier for the `mod` item, so the
//! rules stay in scope after the module ends, like stock textual scope.
use super::*;
use crate::compiler::source_item_macro::exports::{ExportSubtree, MacroUseEntry};

impl Parser<'_, '_> {
    /// Re-define child `index`'s `#[macro_use]` frontier at `point`.
    pub(super) fn adopt_external_macro_use(
        &mut self,
        index: usize,
        point: crate::compiler::semantic_scope::ModuleFrontierId,
    ) -> Result<(), ParseError> {
        let exports = self.lexical.exports.clone().ok_or_else(|| {
            self.unsupported("macro_use on an external module file requires a graph file parse")
        })?;
        let tree = exports
            .child_subtree(index)
            .map_err(native_adapter::source_error)?
            .ok_or_else(|| {
                self.unsupported("macro_use on an external module file requires a graph file parse")
            })?
            .clone();
        // Explicit work stack of (subtree, next entry): nested `#[macro_use]`
        // children are spliced in source order without recursion.
        let mut pending: Vec<(Arc<ExportSubtree>, usize)> = vec![(tree, 0)];
        while let Some((tree, at)) = pending.pop() {
            self.names
                .budget()
                .charge(crate::compiler::expansion_budget::Counter::Work, 1)
                .map_err(|error| native_adapter::transport_error(error.into()))?;
            let Some(entry) = tree.macro_use().get(at) else {
                continue;
            };
            pending.push((tree.clone(), at + 1));
            match entry {
                MacroUseEntry::Child(child) => {
                    if pending.len() > 128 {
                        return Err(self.limit("module nesting exceeds limit 128"));
                    }
                    let child = tree.child(*child).cloned().ok_or_else(|| {
                        native_adapter::transport_error(
                            crate::compiler::token_session::TokenError::InvalidHandle,
                        )
                    })?;
                    pending.push((child, 0));
                }
                MacroUseEntry::Unsupported(_) => {
                    return Err(self.unsupported(
                        "this macro definition form in a macro_use module file is unsupported",
                    ));
                }
                MacroUseEntry::Rules { name, body, site } => {
                    let Some(driver) = self.lexical.item_macros.as_mut() else {
                        return Err(self.unsupported("macro_use requires a module macro scope"));
                    };
                    let defined = driver.define(*name, body.clone(), *site, point, self.names);
                    defined.map_err(|error| self.driver_error(error))?;
                }
            }
        }
        Ok(())
    }
}
