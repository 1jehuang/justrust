//! Inline module headers, kept off the recursive module-body parser frame.
use super::*;

impl Parser<'_, '_> {
    #[inline(never)]
    pub(super) fn inline_module_header(
        &mut self,
        module: ModuleId,
        module_path: &AstPath,
        parsed: &mut ParsedDeclarations,
        visibility: ir::Visibility,
    ) -> Result<(ModuleId, AstPath), ParseError> {
        let name = self.ident()?;
        let full = module_path
            .push(self.names, name)
            .map_err(native_adapter::name_error)?;
        let child = self
            .modules
            .intern_child(self.names, module, name)
            .map_err(module_parse_error)?;
        let root = self.modules.source_root().map_err(module_parse_error)?;
        let spelling = self
            .names
            .spelling(name)
            .map_err(native_adapter::name_error)?;
        if module == root && spelling == super::super::source::source_prelude::PRELUDE_MODULE {
            if parsed.program.prelude_module.is_some() {
                return Err(self.error("duplicate native prelude module"));
            }
            parsed.program.prelude_module = Some(child);
        }
        if module == root && spelling.starts_with(super::super::source::EXTERN_PREFIX) {
            parsed.program.extern_mounts.push(child);
        }
        if let Some(prelude) = parsed.program.prelude_module {
            if module == prelude && spelling == "core_surface" {
                parsed.program.core_library_module = Some(child);
            }
            let parent = self.modules.parent(module).map_err(module_parse_error)?;
            // The crate's edition selects its implicit prelude module.
            let legacy = self.names.edition().pre_2021();
            let (v1, core_v1) = if legacy {
                ("v1_2018", "core_v1_2018")
            } else {
                ("v1", "core_v1")
            };
            if spelling == v1
                && parent == Some(prelude)
                && self
                    .modules
                    .name(module)
                    .map_err(module_parse_error)?
                    .map(|ident| self.names.spelling(ident).map(|s| s == "prelude"))
                    .transpose()
                    .map_err(native_adapter::name_error)?
                    == Some(true)
            {
                parsed.program.prelude_scope = Some(child);
            }
            if spelling == core_v1
                && parent == Some(prelude)
                && self
                    .modules
                    .name(module)
                    .map_err(module_parse_error)?
                    .map(|ident| self.names.spelling(ident).map(|s| s == "prelude"))
                    .transpose()
                    .map_err(native_adapter::name_error)?
                    == Some(true)
            {
                parsed.program.core_prelude_scope = Some(child);
            }
        }
        if self.eat(";") {
            return Err(self.unsupported("external module files are unsupported; use inline mod"));
        }
        let open = self.token().bytes;
        self.expect("{")?;
        self.insert_item(parsed, module, &full, visibility, ItemKind::Module(child))?;
        self.record_child_entry(open, child, &full)?;
        Ok((child, full))
    }

    /// Place this file's parser into its parent's crate at `mount`
    /// (PARSE-SPLIT.md step C2). The module arena and scope arena link the
    /// parent's frozen arenas and join its crate; the module frontier and
    /// textual macro scope resume from the parent's declaration site.
    pub(super) fn mount_child(&mut self, mount: &ChildMount<'_>) -> Result<(), ParseError> {
        let parent = mount.parent;
        let mut arenas = parent.modules.linked_arenas();
        arenas.push(parent.modules.clone());
        let root = arenas
            .iter()
            .find(|a| !a.is_joined())
            .cloned()
            .ok_or_else(|| self.error("no crate root module arena"))?;
        self.modules
            .link_crate_files(arenas)
            .and_then(|()| self.modules.join_crate(&root))
            .map_err(module_parse_error)?;
        let Some(parent_scopes) = parent.scopes.clone() else {
            return Ok(());
        };
        let mut scopes = super::super::semantic_scope::ScopeArenaBuilder::for_builder(self.names)
            .map_err(scope_parse_error)?;
        let mut linked = parent_scopes.linked_arenas();
        linked.push(parent_scopes);
        scopes.link_crate_files(linked).map_err(scope_parse_error)?;
        self.module_frontier = match mount.entry.frontier {
            Some(at) => Some(
                scopes
                    .module_resume(self.names, &*self.modules, at)
                    .map_err(scope_parse_error)?,
            ),
            None => None,
        };
        self.lexical.scopes = Some(scopes);
        if let (Some(checkpoint), Some(source)) =
            (&mount.entry.macros, self.input.macro_source().cloned())
        {
            self.lexical.item_macros = Some(
                super::super::source_item_macro::SourceItemMacroDriver::from_checkpoint(
                    source, checkpoint, self.names,
                )
                .map_err(|error| self.driver_error(error))?,
            );
        }
        Ok(())
    }

    /// Records the entry of an external `mod child;` group in a single-file
    /// view parse: the source preparer rewrote it to `{\n` with the child's
    /// site just inside (PARSE-SPLIT.md step C1).
    fn record_child_entry(
        &mut self,
        open: std::ops::Range<usize>,
        child: ModuleId,
        full: &AstPath,
    ) -> Result<(), ParseError> {
        let next = self.child_entries.len();
        let Some(&site) = self.child_sites.get(next) else {
            return Ok(());
        };
        if open.is_empty() || open.start + 2 != site {
            return Ok(());
        }
        let macros = match self.lexical.item_macros.as_mut() {
            Some(driver) => Some(
                driver
                    .checkpoint(self.names)
                    .map_err(|error| self.driver_error(error))?,
            ),
            None => None,
        };
        self.child_entries.push(ChildEntry {
            module: child,
            path: full.clone(),
            macros,
            frontier: self.module_frontier,
            no_std_roots: self.no_std_roots.clone(),
            forbid_unsafe_roots: self.forbid_unsafe_roots.clone(),
        });
        Ok(())
    }
}
