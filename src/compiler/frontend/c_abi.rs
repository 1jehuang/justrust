//! Parser-owned external C declaration authority. The foreign marker never
//! comes from a public Function field or from a display-name convention.
use super::*;
use crate::compiler::semantic_name::NameKey;
#[derive(Clone, Debug, Default)]
pub(in crate::compiler) struct Declarations {
    pub functions: HashMap<(ModuleId, NameKey), FunctionAbi>,
    pub repr_c: std::collections::HashSet<(ModuleId, NameKey)>,
}
#[derive(Clone, Debug)]
pub(in crate::compiler) struct FunctionAbi {
    module: ModuleId,
    name: AstIdent,
    pub imported: bool,
    pub exported: bool,
    signature: String,
}
fn foreign_carrier(function: &Function) -> bool {
    function.body.statements.is_empty()
        && matches!(function.body.tail.as_deref(),
        Some(Expr::Loop { label: None, body }) if body.statements.is_empty() && body.tail.is_none())
}
fn fingerprint(function: &Function) -> String {
    format!(
        "{:?}:{:?}:{:?}:{:?}:{:?}",
        function.params,
        function.result,
        function.type_params,
        function.constness,
        function.unsafety
    )
}
impl Program {
    pub(in crate::compiler) fn validate_c_source(&self) -> Result<(), String> {
        if self.c_abi.functions.is_empty() {
            return Ok(());
        }
        let mut matched = 0usize;
        for function in &self.functions {
            let Some(name) = function.name.segments().last() else {
                continue;
            };
            let key = self
                .names
                .key(*name, LookupClass::Item(Namespace::Value))
                .map_err(native_adapter::name_diagnostic)?;
            if let Some(entry) = self.c_abi.functions.get(&(function.module, key)) {
                matched += 1;
                if entry.imported && !foreign_carrier(function) {
                    return Err("contract: changed foreign declaration body authority".into());
                }
                if entry.signature != fingerprint(function) {
                    return Err("contract: changed source C ABI signature authority".into());
                }
            }
        }
        if matched != self.c_abi.functions.len() {
            return Err(
                "contract: missing or duplicated source C ABI declaration authority".into(),
            );
        }
        Ok(())
    }
    pub(in crate::compiler) fn seal_resolved_c_source(&mut self) -> Result<(), String> {
        if self.c_abi.functions.is_empty() {
            return Ok(());
        }
        let mut declarations = self.c_abi.as_ref().clone();
        for function in &self.functions {
            let Some(name) = function.name.segments().last() else {
                continue;
            };
            let key = self
                .names
                .key(*name, LookupClass::Item(Namespace::Value))
                .map_err(native_adapter::name_diagnostic)?;
            if let Some(entry) = declarations.functions.get_mut(&(function.module, key)) {
                entry.signature = fingerprint(function);
            }
        }
        self.c_abi = Arc::new(declarations);
        Ok(())
    }
}
impl Parser<'_, '_> {
    pub(in crate::compiler) fn c_attributes(&mut self) -> Result<(), ParseError> {
        while self.eat("#") {
            if self.item_expansion.is_some() {
                return Err(self.unsupported("generated ABI attributes are unsupported"));
            }
            self.expect("[")?;
            if self.eat("repr") {
                self.expect("(")?;
                self.expect("C")?;
                self.expect(")")?;
                if self.pending_repr_c {
                    return Err(self.error("duplicate repr(C)"));
                }
                self.pending_repr_c = true;
            } else if self.eat("unsafe") {
                self.expect("(")?;
                self.expect("no_mangle")?;
                self.expect(")")?;
                if self.pending_c_export {
                    return Err(self.error("duplicate no_mangle"));
                }
                self.pending_c_export = true;
            } else {
                return Err(self.unsupported("unsupported ABI attribute"));
            }
            self.expect("]")?;
        }
        Ok(())
    }
    pub(in crate::compiler) fn c_foreign_block(
        &mut self,
        module: ModuleId,
        path: &AstPath,
        parsed: &mut ParsedDeclarations,
    ) -> Result<(), ParseError> {
        if self.foreign_c || self.item_expansion.is_some() {
            return Err(self.unsupported("nested or generated extern blocks are unsupported"));
        }
        self.expect("unsafe")?;
        self.expect("extern")?;
        if self.token().string {
            if self.token().text != "C" {
                return Err(self.unsupported("only extern \"C\" is supported"));
            }
            self.cursor.bump();
        }
        self.expect("{")?;
        self.foreign_c = true;
        let result = (|| {
            while !self.is("}") {
                let (visibility, explicit) = self.visibility(module)?;
                if !self.starts_qualified_function() {
                    return Err(self.unsupported(
                        "extern C blocks currently support function declarations only",
                    ));
                }
                self.reserve_item_publication(parsed)?;
                self.nonmodule_item(module, path, parsed, visibility, explicit)?;
            }
            self.expect("}")
        })();
        self.foreign_c = false;
        result
    }
    pub(in crate::compiler) fn record_c_function(
        &mut self,
        function: &Function,
        external: bool,
        exported: bool,
    ) -> Result<(), ParseError> {
        if !external {
            if exported {
                return Err(self.unsupported("no_mangle currently requires extern C"));
            }
            return Ok(());
        }
        if self.item_expansion.is_some() {
            return Err(self.unsupported("generated C ABI declarations are unsupported"));
        }
        if function.constness != Constness::Runtime
            || !function.type_params.is_empty()
            || !function.regions.params.is_empty()
        {
            return Err(self.unsupported(
                "generic, lifetime-generic and const C ABI functions are unsupported",
            ));
        }
        let spelling = self
            .names
            .spelling(
                *function
                    .name
                    .segments()
                    .last()
                    .ok_or_else(|| self.error("C function name"))?,
            )
            .map_err(native_adapter::name_error)?;
        if spelling == "main" || spelling == "_start" || spelling.starts_with("__jr_") {
            return Err(self.unsupported("reserved native C symbol is unsupported"));
        }
        let signature = fingerprint(function);
        self.names
            .retain_parser_text(signature.len())
            .map_err(native_adapter::name_error)?;
        self.names
            .budget()
            .charge(
                super::super::expansion_budget::Counter::Work,
                signature.len() as u64,
            )
            .map_err(|e| native_adapter::transport_error(e.into()))?;
        let entry = FunctionAbi {
            module: function.module,
            name: function
                .name
                .single()
                .ok_or_else(|| self.error("C function name"))?,
            imported: self.foreign_c,
            exported,
            signature,
        };
        self.names
            .retain_parser_text(std::mem::size_of::<FunctionAbi>() * 4)
            .map_err(native_adapter::name_error)?;
        self.c_abi
            .functions
            .try_reserve(1)
            .map_err(|_| self.limit("C ABI declaration allocation"))?;
        let key = self
            .names
            .key(entry.name, LookupClass::Item(Namespace::Value))
            .map_err(native_adapter::name_error)?;
        if self
            .c_abi
            .functions
            .insert((entry.module, key), entry)
            .is_some()
        {
            return Err(self.error("duplicate C ABI function"));
        }
        Ok(())
    }
}
impl PlanCtx {
    pub(in crate::compiler) fn has_c_abi(&self) -> bool {
        !self.program.c_abi.functions.is_empty()
    }
    pub(in crate::compiler) fn c_abi_for(
        &self,
        decl: ir::DeclId,
    ) -> Result<Option<(&FunctionAbi, crate::compiler::c_abi::Signature)>, String> {
        let Some(function) = self.registry.source_callable(decl) else {
            return Ok(None);
        };
        let Some(name) = function.name.segments().last() else {
            return Ok(None);
        };
        let key = self
            .program
            .names
            .key(*name, LookupClass::Item(Namespace::Value))
            .map_err(native_adapter::name_diagnostic)?;
        let Some(entry) = self.program.c_abi.functions.get(&(function.module, key)) else {
            return Ok(None);
        };
        self.budget.charge_work(function.params.len() + 1)?;
        if entry.signature != fingerprint(function) {
            return Err("contract: changed source C ABI signature authority".into());
        }
        let signature = self.signature(decl)?;
        let repr = |id| self.c_repr_decl(id);
        let mut params = Vec::with_capacity(signature.signature.params.len());
        for param in &signature.signature.params {
            params.push(crate::compiler::c_abi::layout::of(
                param,
                &repr,
                &self.budget,
            )?);
        }
        let result =
            crate::compiler::c_abi::layout::of(&signature.signature.result, &repr, &self.budget)?;
        Ok(Some((
            entry,
            crate::compiler::c_abi::Signature::new(params, result)?,
        )))
    }
    pub(in crate::compiler) fn c_abi_symbol(&self, decl: ir::DeclId) -> Result<String, String> {
        let function = self
            .registry
            .source_callable(decl)
            .ok_or("C ABI declaration has no source")?;
        self.program
            .names
            .spelling(*function.name.segments().last().ok_or("C ABI name")?)
            .map(str::to_owned)
            .map_err(native_adapter::name_diagnostic)
    }
    fn c_repr_decl(&self, id: ir::DeclId) -> bool {
        let Ok(info) = self.declaration(id) else {
            return false;
        };
        if info.kind != ir::DeclKind::Struct {
            return false;
        }
        self.program
            .structs
            .get(info.source_index)
            .is_some_and(|s| {
                s.name.segments().last().is_some_and(|name| {
                    self.program
                        .names
                        .key(*name, LookupClass::Item(Namespace::Type))
                        .is_ok_and(|key| self.program.c_abi.repr_c.contains(&(s.module, key)))
                })
            })
    }
    pub(in crate::compiler) fn validate_c_abi(&self) -> Result<(), String> {
        if self.program.c_abi.functions.is_empty() {
            return Ok(());
        }
        for index in 0..self.callable_count() {
            let decl = self.callable_decl(index)?;
            self.c_abi_for(decl)?;
        }
        Ok(())
    }
}
