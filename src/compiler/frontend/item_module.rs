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
        }
        if self.eat(";") {
            return Err(self.unsupported("external module files are unsupported; use inline mod"));
        }
        self.expect("{")?;
        self.insert_item(parsed, module, &full, visibility, ItemKind::Module(child))?;
        Ok((child, full))
    }
}
