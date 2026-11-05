//! Inline module headers, kept off the recursive module-body parser frame.
use super::*;

/// Two original declaration occurrences, not two requests to intern a child.
/// Kept private with the parsed program so public AST mutation cannot erase
/// the obligation by removing an unused function or changing its module.
#[derive(Clone, Debug)]
pub(super) struct DuplicateModuleSyntax {
    pub(super) parent: ModuleId,
    child: ModuleId,
    pub(super) first: AstPath,
    pub(super) second: AstPath,
    issued: ModuleDeclarationIssuance,
}

/// Only the header parser below mints this immutable authority, after consuming
/// `mod`, its identifier and `{`, and finding the first module declaration.
/// Transported paths cannot replace these exact occurrences with same-owner
/// function names, references, or structurally constructed identifiers.
#[derive(Clone, Debug)]
struct ModuleDeclarationIssuance {
    parent: ModuleId,
    child: ModuleId,
    first: AstPath,
    second: AstPath,
}

impl DuplicateModuleSyntax {
    pub(super) fn validate(&self, program: &Program) -> Result<(), String> {
        if self.parent != self.issued.parent || self.child != self.issued.child {
            return Err("duplicate module declaration issuance mismatch".into());
        }
        self.issued
            .first
            .check_target(program.names.as_ref(), &self.first)
            .and_then(|()| {
                self.issued
                    .second
                    .check_target(program.names.as_ref(), &self.second)
            })
            .map_err(native_adapter::name_diagnostic)?;
        program
            .modules
            .check(self.parent)
            .map_err(module_diagnostic)?;
        program
            .modules
            .check(self.child)
            .map_err(module_diagnostic)?;
        self.first
            .check(program.names.as_ref())
            .map_err(native_adapter::name_diagnostic)?;
        self.second
            .check(program.names.as_ref())
            .map_err(native_adapter::name_diagnostic)?;
        let first = name_scope::path_key(program.names.as_ref(), &self.first, Namespace::Type)?;
        let second = name_scope::path_key(program.names.as_ref(), &self.second, Namespace::Type)?;
        let terminal = self
            .second
            .segments()
            .last()
            .ok_or("empty duplicate module path")?;
        if first != second
            || program
                .modules
                .parent(self.child)
                .map_err(module_diagnostic)?
                != Some(self.parent)
            || program
                .modules
                .lookup_child(program.names.as_ref(), self.parent, *terminal)
                .map_err(module_diagnostic)?
                != Some(self.child)
        {
            return Err("invalid duplicate module source authority".into());
        }
        let first_name = self
            .first
            .segments()
            .last()
            .ok_or("empty duplicate module path")?;
        let first_span = program
            .names
            .data(*first_name)
            .map_err(native_adapter::name_diagnostic)?
            .span;
        let second_span = program
            .names
            .data(*terminal)
            .map_err(native_adapter::name_diagnostic)?
            .span;
        if first_span == second_span {
            return Err("duplicate module evidence needs distinct declaration occurrences".into());
        }
        Ok(())
    }
}

impl Parser<'_, '_> {
    #[inline(never)]
    pub(super) fn inline_module_header(
        &mut self,
        module: ModuleId,
        module_path: &AstPath,
        parsed: &mut ParsedDeclarations,
        visibility: ir::Visibility,
    ) -> Result<(ModuleId, AstPath, bool), ParseError> {
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
        let first = parsed
            .items
            .get(&full.key(Namespace::Type))
            .and_then(|items| {
                items
                    .iter()
                    .find(|item| matches!(item.kind, ItemKind::Module(_)))
            });
        let duplicate = if let Some(first) = first {
            let duplicates = &mut parsed.program.duplicate_modules;
            if duplicates.len() == duplicates.capacity() {
                let planned = duplicates
                    .len()
                    .checked_add(1)
                    .and_then(|n| n.checked_mul(std::mem::size_of::<DuplicateModuleSyntax>()))
                    .ok_or_else(|| {
                        native_adapter::transport_error(
                            super::super::token_session::TokenError::IdExhausted,
                        )
                    })?;
                self.names
                    .retain_parser_storage(planned)
                    .map_err(native_adapter::name_error)?;
                duplicates.try_reserve_exact(1).map_err(|_| {
                    native_adapter::transport_error(self.names.budget().poison_allocation().into())
                })?;
                let actual = duplicates
                    .capacity()
                    .checked_mul(std::mem::size_of::<DuplicateModuleSyntax>())
                    .ok_or_else(|| {
                        native_adapter::transport_error(
                            super::super::token_session::TokenError::IdExhausted,
                        )
                    })?;
                self.names
                    .retain_parser_storage(actual.saturating_sub(planned))
                    .map_err(native_adapter::name_error)?;
            }
            duplicates.push(DuplicateModuleSyntax {
                parent: module,
                child,
                first: first.path.clone(),
                second: full.clone(),
                issued: ModuleDeclarationIssuance {
                    parent: module,
                    child,
                    first: first.path.clone(),
                    second: full.clone(),
                },
            });
            true
        } else {
            self.insert_item(parsed, module, &full, visibility, ItemKind::Module(child))?;
            false
        };
        self.record_child_entry(open, child, &full)?;
        Ok((child, full, duplicate))
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

#[cfg(test)]
mod tests {
    use super::*;

    const SOURCE: &str = "mod owner { mod duplicate { pub fn value()->i32 {7} } mod duplicate { pub fn value()->i32 {11} } } fn main() {}";

    #[test]
    fn duplicate_module_obligation_survives_unused_public_ast_erasure() {
        let parsed = parse_unresolved(SOURCE).unwrap();
        assert_eq!(parsed.program.duplicate_modules.len(), 1);
        let mut program = resolve(&parsed).unwrap();
        program.functions.retain(|function| {
            program
                .names
                .spelling(*function.name.segments().last().unwrap())
                .unwrap()
                == "main"
        });
        program.validate_name_handles().unwrap();
        let plan = PlanCtx::from_program(&program, &ir::PlanBudget::default()).unwrap();
        let errors: Vec<_> = (0..plan.module_inventory_count())
            .filter_map(|ordinal| plan.module_inventory(ordinal).err())
            .collect();
        assert_eq!(errors.len(), 1, "{errors:?}");
        assert!(errors[0].contains("E0428"), "{errors:?}");
    }

    #[test]
    fn duplicate_module_evidence_rejects_foreign_owners_and_forged_occurrences() {
        let program = parse_unresolved(SOURCE).unwrap().program;
        let foreign = parse_unresolved(SOURCE).unwrap().program;
        let original = program.duplicate_modules[0].clone();
        original.validate(&program).unwrap();
        for mode in 0..5 {
            let mut forged = original.clone();
            match mode {
                0 => forged.child = foreign.duplicate_modules[0].child,
                1 => forged.parent = foreign.duplicate_modules[0].parent,
                2 => forged.second = foreign.duplicate_modules[0].second.clone(),
                3 => forged.second = forged.first.clone(),
                4 => forged.parent = program.modules.source_root().unwrap(),
                _ => unreachable!(),
            }
            assert!(forged.validate(&program).is_err(), "forgery {mode}");
            let mut changed = program.clone();
            changed.duplicate_modules = vec![forged].into();
            assert!(changed.validate_name_handles().is_err(), "forgery {mode}");
        }
    }

    fn header_fixture_with_structural_occurrences() -> (Program, AstPath, AstPath, ModuleId) {
        use crate::compiler::expansion_span::{ExpansionRecord, SyntheticReason};
        use crate::compiler::hygiene::Transparency;
        use crate::compiler::token::Edition;

        let (tokens, session, cap) = lex("mod owner { mod duplicate {} mod duplicate {} fn duplicate() {} mod healthy {} } fn main() {}")
            .unwrap()
            .into_parts();
        let mut names = NameStoreBuilder::new(session, cap, Edition::E2024).unwrap();
        let symbols = ProgramSymbols::new(&mut names).unwrap();
        let mut parser = Parser::new(&tokens, &mut names).unwrap();
        let root = parser.modules.source_root().unwrap();
        let root_path = AstPath::new(parser.names, PathAnchor::Crate, &[]).unwrap();
        let mut parsed = ParsedDeclarations {
            program: ProgramDeclarations::default(),
            items: HashMap::new(),
            empty_use_prefixes: Vec::new(),
            glob_imports: Vec::new(),
        };
        parser.items(root, &root_path, &mut parsed).unwrap();
        let original = parsed.program.duplicate_modules[0].clone();
        // Repeated arena interning is valid, and must never mint a source
        // declaration obligation or change an existing declaration's child.
        for path in [&original.first, &original.second] {
            assert_eq!(
                parser
                    .modules
                    .intern_child(
                        parser.names,
                        original.parent,
                        *path.segments().last().unwrap()
                    )
                    .unwrap(),
                original.child
            );
        }
        assert_eq!(parsed.program.duplicate_modules.len(), 1);
        let healthy = parsed
            .items
            .values()
            .flatten()
            .find_map(|item| {
                if let ItemKind::Module(child) = item.kind {
                    (parser.modules.parent(child).unwrap() == Some(original.parent)
                        && child != original.child)
                        .then_some(child)
                } else {
                    None
                }
            })
            .unwrap();
        let terminal = *original.second.segments().last().unwrap();
        let anchor = parser.names.span(terminal).unwrap();
        let synthetic_span = parser
            .names
            .session_mut()
            .synthetic_span(anchor, SyntheticReason::ExpansionToken)
            .unwrap();
        let synthetic = parser
            .names
            .intern_root("duplicate", synthetic_span)
            .unwrap();
        let context = parser.names.session().root_context();
        let contextual_span = parser
            .names
            .session_mut()
            .with_context(anchor, context, None)
            .unwrap();
        let contextual = parser
            .names
            .intern_root("duplicate", contextual_span)
            .unwrap();
        let file = parser.names.session().sources().file(0).unwrap();
        let krate = parser
            .names
            .session_mut()
            .register_crate_root(file)
            .unwrap();
        let expansion = parser
            .names
            .session_mut()
            .add_expansion(ExpansionRecord {
                definition: anchor,
                call: anchor,
                parent: None,
                defining_crate: krate,
                edition: Edition::E2024,
            })
            .unwrap();
        let marked = parser
            .names
            .session_mut()
            .apply_mark(context, expansion, Transparency::SemiTransparent)
            .unwrap();
        let marked_span = parser
            .names
            .session_mut()
            .with_context(anchor, marked, Some(expansion))
            .unwrap();
        assert_ne!(marked, context);
        assert!(parser.names.intern_root("duplicate", marked_span).is_err());
        let mut segments = original.second.segments().to_vec();
        *segments.last_mut().unwrap() = synthetic;
        let synthetic = AstPath::new(parser.names, original.second.anchor(), &segments).unwrap();
        *segments.last_mut().unwrap() = contextual;
        let contextual = AstPath::new(parser.names, original.second.anchor(), &segments).unwrap();
        let modules = parser.modules.into_owned().unwrap().freeze().unwrap();
        let program = parsed
            .program
            .publish(names.freeze().unwrap(), modules, symbols, None)
            .unwrap();
        (program, synthetic, contextual, healthy)
    }

    #[test]
    fn duplicate_module_issuance_rejects_same_owner_non_module_and_structural_paths() {
        let (program, synthetic, contextual, healthy) =
            header_fixture_with_structural_occurrences();
        let original = program.duplicate_modules[0].clone();
        original.validate(&program).unwrap();
        let function = program
            .functions
            .iter()
            .find(|function| {
                program
                    .names
                    .spelling(*function.name.segments().last().unwrap())
                    .unwrap()
                    == "duplicate"
            })
            .unwrap();
        for replacement in [&function.name, &synthetic, &contextual] {
            // These are valid, actual-changing same-owner, same-spelling
            // handles. Semantic path equality alone cannot prove a header.
            replacement.check(program.names.as_ref()).unwrap();
            assert_eq!(
                replacement.key(Namespace::Type),
                original.second.key(Namespace::Type)
            );
            assert!(
                !exact_source_ident(
                    &program.names,
                    *replacement.segments().last().unwrap(),
                    *original.second.segments().last().unwrap()
                )
                .unwrap()
            );
            for replace_first in [false, true] {
                let mut forged = original.clone();
                if replace_first {
                    forged.first = replacement.clone();
                } else {
                    forged.second = replacement.clone();
                }
                assert!(forged.validate(&program).is_err());
                let mut transported = program.clone();
                transported.duplicate_modules = vec![forged].into();
                assert!(transported.validate_name_handles().is_err());
            }
        }
        program.modules.check(healthy).unwrap();
        assert_eq!(
            program.modules.parent(healthy).unwrap(),
            Some(original.parent)
        );
        assert_ne!(healthy, original.child);
        let mut forged = original;
        forged.child = healthy;
        assert!(forged.validate(&program).is_err());
        let mut transported = program.clone();
        transported.duplicate_modules = vec![forged].into();
        assert!(transported.validate_name_handles().is_err());
    }

    #[test]
    fn unused_empty_duplicate_module_has_its_own_required_inventory() {
        let parsed = parse_unresolved("mod owner { mod duplicate {} mod duplicate {} } mod healthy { pub fn value()->i32 {7} } fn main() {} ").unwrap();
        let program = resolve(&parsed).unwrap();
        let plan = PlanCtx::from_program(&program, &ir::PlanBudget::default()).unwrap();
        let healthy = resolve(&parse_unresolved("mod owner { mod duplicate {} mod distinct {} } mod healthy { pub fn value()->i32 {7} } fn main() {} ").unwrap()).unwrap();
        let baseline = PlanCtx::from_program(&healthy, &ir::PlanBudget::default()).unwrap();
        assert_eq!(
            plan.module_inventory_count(),
            baseline.module_inventory_count() + 1
        );
        assert_eq!(
            (0..plan.module_inventory_count())
                .filter(|&ordinal| plan.module_inventory(ordinal).is_err())
                .count(),
            1
        );
    }

    #[test]
    fn duplicate_module_body_is_not_a_syntax_skip_boundary() {
        for source in [
            "mod duplicate {} mod duplicate { fn broken( } fn main() {}",
            "mod owner { mod duplicate {} mod duplicate { let =; } } fn main() {}",
        ] {
            assert!(parse_unresolved(source).is_err(), "{source}");
        }
    }
}
