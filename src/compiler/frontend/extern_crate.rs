//! `extern crate` items. The item is an ordinary type-namespace import of the
//! crate root (native `core`/`std` or a mounted dependency, which already
//! enters through the extern prelude), so `extern crate core as std;` makes
//! `std::...` name core paths and the import resolver keeps deciding privacy,
//! shadowing and collisions. An unknown dependency name fails resolution.
use super::*;

impl Parser<'_, '_> {
    #[inline(never)]
    pub(super) fn extern_crate_item(
        &mut self,
        module: ModuleId,
        module_path: &AstPath,
        parsed: &mut ParsedDeclarations,
        visibility: ir::Visibility,
    ) -> Result<(), ParseError> {
        self.expect("extern")?;
        self.expect("crate")?;
        let crate_token = self.token();
        if crate_token.string || crate_token.text == "self" {
            return Err(self.unsupported("extern crate self is unsupported"));
        }
        let library = matches!(crate_token.text, "core" | "std");
        let crate_name = self.ident()?;
        let local = if self.eat("as") {
            if self.is("_") {
                return Err(self.unsupported("extern crate ... as _ is unsupported"));
            }
            self.ident()?
        } else {
            crate_name
        };
        self.expect(";")?;
        if crate_token.text == "std" && !self.no_std_roots.is_empty() {
            let root = super::super::source_macro_binding::invocation_root(
                self.names.session(),
                crate_token.span,
            )
            .map_err(native_adapter::transport_error)?;
            if self.no_std_roots.contains(&root) {
                return Err(self.unsupported(
                    "explicit extern crate std opt-in in a no_std crate is unsupported",
                ));
            }
        }
        let target = AstPath::new(self.names, PathAnchor::Relative, &[crate_name])
            .map_err(native_adapter::name_error)?;
        // A dependency crate must exist even when the item is otherwise a
        // no-op: validate it like an empty use group naming its root.
        if !library {
            parsed.empty_use_prefixes.push((module, target.clone()));
        }
        // An unaliased `extern crate std;` names itself; the extern prelude
        // already provides that name, so recording a self-import would only
        // create a resolver cycle.
        if self.name_key(local, LookupClass::Item(Namespace::Type))?
            == self.name_key(crate_name, LookupClass::Item(Namespace::Type))?
        {
            return Ok(());
        }
        let path = module_path
            .push(self.names, local)
            .map_err(native_adapter::name_error)?;
        let crate_root = module == self.modules.source_root().map_err(module_parse_error)?
            || parsed.program.extern_mounts.contains(&module);
        if crate_root {
            parsed
                .program
                .extern_crate_aliases
                .push((module, path.clone()));
        }
        self.insert_item(
            parsed,
            module,
            &path,
            visibility,
            ItemKind::Import(target, true),
        )
    }
}
