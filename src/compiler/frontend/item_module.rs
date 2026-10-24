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
            if spelling == "v1"
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
            if spelling == "core_v1"
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
        });
        Ok(())
    }
}
