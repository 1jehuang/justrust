//! Immutable per-owner/per-site capture graph data. No context/budget handles
//! are retained. Keys contain declaration indices and structural source paths.
use super::*;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(in crate::compiler) enum CaptureScalar {
    Unit,
    Bool,
    Char,
    Int(ir::IntTy),
    Float(ir::FloatTy),
}
impl CaptureScalar {
    fn source(ty: &Type) -> Result<Self, String> {
        Ok(match ty {
            Type::Unit => Self::Unit,
            Type::Bool => Self::Bool,
            Type::Char => Self::Char,
            Type::Int(ty) => Self::Int(*ty),
            Type::Float(ty) => Self::Float(*ty),
            _ => return Err("unsupported: capture graph requires a scalar parameter".into()),
        })
    }
    fn checked(ty: &ir::Ty) -> Result<Self, String> {
        Ok(match ty {
            ir::Ty::Unit => Self::Unit,
            ir::Ty::Bool => Self::Bool,
            ir::Ty::Char => Self::Char,
            ir::Ty::Int(ty) => Self::Int(*ty),
            ir::Ty::Float(ty) => Self::Float(*ty),
            _ => return Err("capture signature is not the original scalar parameter".into()),
        })
    }
}
#[derive(Clone, Debug)]
pub(in crate::compiler) struct LexicalCapture {
    pub binding: sr::SourceNode,
    pub parameter: usize,
    pub scalar: CaptureScalar,
}
#[derive(Clone, Debug)]
pub(in crate::compiler) struct LexicalClosureSite {
    pub node: sr::SourceNode,
    pub declaration: usize,
    pub captures: Arc<[LexicalCapture]>,
}
#[derive(Clone, Debug)]
pub(in crate::compiler) struct LexicalBody {
    pub subject: ir::DeclId,
    pub owner: sr::OriginalBodyId,
    pub declaration: usize,
    pub sites: Arc<[LexicalClosureSite]>,
}
#[derive(Clone, Debug)]
pub(in crate::compiler) struct CaptureEnvironment {
    pub site: sr::SourceNode,
    pub parent: usize,
    pub declaration: usize,
    pub fields: Arc<[LexicalCapture]>,
}

impl PlanCtx {
    fn capture_declaration_index(
        &self,
        name: &AstPath,
        budget: &ir::PlanBudget,
    ) -> Result<usize, String> {
        budget.charge_work(1)?;
        let key = name_scope::path_key(self.program.names.as_ref(), name, Namespace::Value)?;
        let declaration = self
            .registry
            .functions
            .get(&key)
            .ok_or("capture owner is not a source function")?;
        self.callable_index(*declaration)
    }

    /// At most own and immediate parent lexical facts. Programs without captured
    /// sites do not acquire extra graph dependencies or source-inventory work.
    pub(in crate::compiler) fn closure_graph_owners(
        &self,
        index: usize,
        budget: &ir::PlanBudget,
    ) -> Result<Vec<usize>, String> {
        if self.program.closure_owners.is_empty() {
            return Ok(Vec::new());
        }
        let declaration = self.callable_decl(index)?;
        if !matches!(
            self.callable_provider(declaration)?,
            CallableProvider::Source { .. }
        ) {
            return Ok(Vec::new());
        }
        let function = self.callable_body(&self.program, declaration)?;
        let key =
            name_scope::path_key(self.program.names.as_ref(), &function.name, Namespace::Type)?;
        budget.charge_work(2)?;
        let mut owners = Vec::new();
        if self.program.closure_owners.contains_key(&key) {
            owners.push(index);
        }
        if let Some(source) = self.program.closure_sources.get(&key)
            && !source.captures.is_empty()
        {
            let parent = source
                .parent
                .as_ref()
                .ok_or("captured source lacks parent")?;
            let parent = self.capture_declaration_index(&parent.name, budget)?;
            if !owners.contains(&parent) {
                owners.push(parent);
            }
        }
        Ok(owners)
    }

    pub(in crate::compiler) fn publish_closure_lexical(
        &self,
        index: usize,
        budget: &ir::PlanBudget,
    ) -> Result<LexicalBody, String> {
        self.check_budget(budget)?;
        let declaration = self.callable_decl(index)?;
        let function = self.callable_body(&self.program, declaration)?;
        let owner_key =
            name_scope::path_key(self.program.names.as_ref(), &function.name, Namespace::Type)?;
        let expected = self
            .program
            .closure_owners
            .get(&owner_key)
            .ok_or("body has no captured source sites")?;
        let owner =
            sr::OriginalBodyId::new(budget).map_err(|e| e.into_legacy_diagnostic(budget))?;
        let inventory = SourceInventory::build(&self.program, function, owner.clone(), budget)
            .map_err(|e| e.into_legacy_diagnostic(budget))?;
        let offset = self
            .program
            .closure_sources
            .get(&owner_key)
            .map_or(0, |source| source.captures.len());
        let mut sites = Vec::new();
        for call in &inventory.calls {
            budget.charge_work(1)?;
            let Some(name) = &call.ordinary else { continue };
            let key = name_scope::path_key(self.program.names.as_ref(), name, Namespace::Type)?;
            let Some(source) = self.program.closure_sources.get(&key) else {
                continue;
            };
            if source.captures.is_empty() {
                continue;
            }
            let parent = source
                .parent
                .as_ref()
                .ok_or("captured source lacks parent")?;
            if name_scope::path_key(self.program.names.as_ref(), &parent.name, Namespace::Type)?
                != owner_key
            {
                return Err("captured call belongs to another original body".into());
            }
            let mut captures = Vec::new();
            for capture in &source.captures {
                budget.charge_work(1)?;
                let parameter = capture
                    .parameter
                    .checked_add(offset)
                    .ok_or("capture parameter index overflow")?;
                let (_, source_ty) = function
                    .params
                    .get(parameter)
                    .ok_or("capture parameter outside original owner")?;
                let scalar = CaptureScalar::source(source_ty)?;
                if scalar != CaptureScalar::source(&capture.ty)? {
                    return Err("capture scalar differs from source parameter".into());
                }
                let parameter_u32 =
                    u32::try_from(parameter).map_err(|_| "capture parameter path overflow")?;
                let binding = inventory
                    .bindings
                    .get(parameter)
                    .ok_or("capture binding outside source inventory")?;
                if binding.parameter != Some(parameter)
                    || binding.node.path.as_ref() != [0, parameter_u32]
                {
                    return Err("capture binding is not the original parameter occurrence".into());
                }
                let binding = binding.node.clone();
                sr::push(
                    &mut captures,
                    LexicalCapture {
                        binding,
                        parameter,
                        scalar,
                    },
                    budget,
                )
                .map_err(|e| e.into_legacy_diagnostic(budget))?;
            }
            let declaration = self.capture_declaration_index(&source.name, budget)?;
            sr::push(
                &mut sites,
                LexicalClosureSite {
                    node: call.node.clone(),
                    declaration,
                    captures: captures.into(),
                },
                budget,
            )
            .map_err(|e| e.into_legacy_diagnostic(budget))?;
        }
        if sites.len() != expected.len() {
            return Err("captured source-site inventory changed".into());
        }
        Ok(LexicalBody {
            subject: declaration,
            owner,
            declaration: index,
            sites: sites.into(),
        })
    }

    pub(in crate::compiler) fn publish_capture_environment(
        &self,
        lexical: &LexicalBody,
        path: &[u32],
        budget: &ir::PlanBudget,
    ) -> Result<CaptureEnvironment, String> {
        self.check_budget(budget)?;
        budget.check_depth(path.len() / 2)?;
        let mut selected = None;
        for site in lexical.sites.iter() {
            budget.charge_work(
                site.node
                    .path
                    .len()
                    .max(path.len())
                    .checked_add(1)
                    .ok_or("capture path work overflow")?,
            )?;
            if site.node.path.as_ref() == path && selected.replace(site).is_some() {
                return Err("duplicate captured source path".into());
            }
        }
        let site = selected.ok_or("capture graph path is not an original closure site")?;
        if site.node.owner != lexical.owner {
            return Err("capture graph source owner changed".into());
        }
        let parent = self.callable_decl(lexical.declaration)?;
        if parent != lexical.subject {
            return Err("capture lexical fact belongs to another source snapshot".into());
        }
        let callee = self.callable_decl(site.declaration)?;
        let parent = self.signature(parent)?;
        let callee = self.signature(callee)?;
        let fields = callee
            .signature
            .params
            .get(..site.captures.len())
            .ok_or("capture callee lacks shared environment parameters")?;
        for (index, capture) in site.captures.iter().enumerate() {
            budget.charge_work(1)?;
            if capture.binding.owner != lexical.owner {
                return Err("captured binding belongs to another source owner".into());
            }
            let parameter_u32 =
                u32::try_from(capture.parameter).map_err(|_| "capture binding path overflow")?;
            if capture.binding.path.as_ref() != [0, parameter_u32] {
                return Err("captured binding is not the exact original parameter".into());
            }
            let actual = parent
                .signature
                .params
                .get(capture.parameter)
                .ok_or("capture outside typed parent signature")?;
            if CaptureScalar::checked(actual)? != capture.scalar {
                return Err("typed capture differs from original parameter".into());
            }
            let ir::Ty::Ref {
                kind: ir::BorrowKind::Shared,
                referent,
            } = &fields[index]
            else {
                return Err("capture field is not a real shared loan".into());
            };
            if CaptureScalar::checked(referent)? != capture.scalar {
                return Err("capture environment field type changed".into());
            }
        }
        Ok(CaptureEnvironment {
            site: site.node.clone(),
            parent: lexical.declaration,
            declaration: site.declaration,
            fields: site.captures.clone(),
        })
    }
}
