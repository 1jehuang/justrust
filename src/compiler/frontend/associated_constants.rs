//! Source declarations for associated constants. These are not callable bodies
//! and cannot be executed by the runtime method-selection path.
use super::*;
mod inherent;
mod initializer;
pub(in crate::compiler) use initializer::InitializerMemberInput;
mod metadata;
mod symbolic;
mod uses;
pub(in crate::compiler::frontend) use symbolic::{
    SourceSymbolicConstantUse, validate_symbolic_constants,
};
pub(in crate::compiler) use uses::AuthorizedAssociatedConstant;

#[cfg(test)]
mod tests;

#[derive(Clone, Debug)]
pub struct AssociatedConstDecl {
    pub name: AstIdent,
    pub ty: Type,
    pub initializer: Option<Expr>,
    pub visibility: ir::Visibility,
}

#[derive(Clone, Debug)]
pub struct AssociatedConstUse {
    pub owner: Type,
    pub trait_path: Option<TraitPath>,
    pub name: AstIdent,
}

#[derive(Debug, Default)]
pub(super) struct Registry {
    pub(super) traits: Vec<Vec<ir::DeclId>>,
    pub(super) impls: Vec<Vec<ir::DeclId>>,
    pub(super) inherent: HashMap<NameKey, Vec<ir::DeclId>>,
}

pub(super) fn seal_registry_owners(
    registry: &mut DeclRegistry,
    program: &Program,
    budget: &ir::PlanBudget,
) -> Result<(), String> {
    for (owner_index, constants) in registry.associated_constants.traits.iter().enumerate() {
        let owner = registry.trait_ids[owner_index];
        let env = registry.declaration(owner)?.env.clone();
        for (index, declaration) in constants.iter().enumerate() {
            budget.charge_work(1)?;
            let source = &program.traits[owner_index].associated_constants[index];
            clone_visibility(&source.visibility, budget)?;
            let info = decl_mut(&registry.slots, &mut registry.declarations, declaration)?;
            info.owner = Some(owner);
            info.env = env.clone();
            info.visibility = source.visibility.clone();
        }
    }
    for (owner_index, constants) in registry.associated_constants.impls.iter().enumerate() {
        let block = &program.impls[owner_index];
        for (index, member) in block.associated_constants.iter().enumerate() {
            for previous in &block.associated_constants[..index] {
                budget.charge_work(1)?;
                if name_scope::ident_eq(
                    registry.names.as_ref(),
                    previous.name,
                    member.name,
                    LookupClass::Member,
                )? {
                    return Err("E0201: duplicate associated constant declaration".into());
                }
            }
            for method in &block.items {
                budget.charge_work(1)?;
                if let Some(name) = method.function.name.single()
                    && name_scope::ident_eq(
                        registry.names.as_ref(),
                        name,
                        member.name,
                        LookupClass::Member,
                    )?
                {
                    return Err(
                        "E0201: associated constant conflicts with method declaration".into(),
                    );
                }
            }
        }
        let owner = registry.impl_ids[owner_index];
        let env = registry.declaration(owner)?.env.clone();
        for (index, declaration) in constants.iter().enumerate() {
            budget.charge_work(1)?;
            let source = &program.impls[owner_index].associated_constants[index];
            clone_visibility(&source.visibility, budget)?;
            let info = decl_mut(&registry.slots, &mut registry.declarations, declaration)?;
            info.owner = Some(owner);
            info.env = env.clone();
            info.visibility = source.visibility.clone();
        }
    }
    let mut indexed = HashMap::<NameKey, Vec<ir::DeclId>>::new();
    for (owner, rows) in registry.associated_constants.impls.iter().enumerate() {
        if program.impls[owner].trait_path.is_some() {
            continue;
        }
        for (index, declaration) in rows.iter().enumerate() {
            budget.charge_work(1)?;
            let name = program.impls[owner].associated_constants[index].name;
            let key = registry
                .names
                .key(name, LookupClass::Member)
                .map_err(native_adapter::name_diagnostic)?;
            sr::push(indexed.entry(key).or_default(), *declaration, budget)
                .map_err(|e| e.into_legacy_diagnostic(budget))?;
        }
    }
    registry.associated_constants.inherent = indexed;
    Ok(())
}

pub(super) fn source<'a>(
    registry: &'a DeclRegistry,
    info: &DeclInfo,
) -> Result<&'a AssociatedConstDecl, String> {
    if !std::ptr::eq(registry.declaration(info.id)?, info) {
        return Err(
            "associated constant source metadata is not its retained registry occurrence".into(),
        );
    }
    if info.kind != ir::DeclKind::AssociatedConst {
        return Err("associated constant identity has a different declaration kind".into());
    }
    let owner = registry.declaration(info.owner.ok_or("associated constant has no owner")?)?;
    let rows = match owner.kind {
        ir::DeclKind::Trait => registry
            .program
            .traits
            .get(owner.source_index)
            .map(|item| &item.associated_constants),
        ir::DeclKind::Impl | ir::DeclKind::TraitImpl => registry
            .program
            .impls
            .get(owner.source_index)
            .map(|item| &item.associated_constants),
        _ => None,
    }
    .ok_or("associated constant has an invalid retained owner")?;
    rows.get(info.source_index)
        .ok_or_else(|| "associated constant source occurrence is missing".into())
}

pub(super) fn retains_projection(
    registry: &DeclRegistry,
    info: &DeclInfo,
    projection: &SourceProjection,
    budget: &ir::PlanBudget,
) -> Result<bool, String> {
    let source = source(registry, info)?;
    let mut found =
        retained_type_projection(registry.names.as_ref(), &source.ty, projection, budget)?;
    if let Some(value) = &source.initializer {
        found |= retained_node_projection(
            registry.names.as_ref(),
            DepthNode::Expr(value),
            projection,
            budget,
        )?;
    }
    Ok(found)
}

impl PlanCtx {
    pub(in crate::compiler) fn associated_constant_declarations(
        &self,
    ) -> impl Iterator<Item = ir::DeclId> + '_ {
        self.registry
            .associated_constants
            .traits
            .iter()
            .chain(&self.registry.associated_constants.impls)
            .flatten()
            .copied()
    }
    pub(in crate::compiler) fn associated_constant_count(&self) -> usize {
        self.registry
            .associated_constants
            .traits
            .iter()
            .chain(&self.registry.associated_constants.impls)
            .map(Vec::len)
            .sum()
    }
    pub(in crate::compiler) fn associated_constant_source(
        &self,
        declaration: ir::DeclId,
    ) -> Result<&AssociatedConstDecl, String> {
        source(&self.registry, self.registry.declaration(declaration)?)
    }
    pub(in crate::compiler) fn associated_constant_header(
        &self,
        declaration: ir::DeclId,
        budget: &ir::PlanBudget,
    ) -> Result<tr::AssociatedConstHeader, String> {
        self.associated_constant_schema(declaration, budget)
    }
    pub(super) fn validate_constant_completeness(
        &self,
        implementations: &[tr::ImplHeader],
        budget: &ir::PlanBudget,
    ) -> Result<(), String> {
        for implementation in implementations {
            let owner = self.registry.declaration(implementation.declaration)?;
            let trait_owner = self
                .registry
                .declaration(implementation.trait_ref.declaration)?;
            if self.catalog.declaration(trait_owner.id).is_some() {
                continue;
            }
            let required = &self.program.traits[trait_owner.source_index].associated_constants;
            let supplied = &self.program.impls[owner.source_index].associated_constants;
            for member in required {
                if member.initializer.is_some() {
                    continue;
                }
                let mut found = false;
                for candidate in supplied {
                    budget.charge_work(1)?;
                    found |= name_scope::ident_eq(
                        self.registry.names.as_ref(),
                        member.name,
                        candidate.name,
                        LookupClass::Member,
                    )?;
                }
                if !found {
                    return Err("E0046: missing required associated constant implementation".into());
                }
            }
        }
        Ok(())
    }
    pub(super) fn associated_constant_schema(
        &self,
        declaration: ir::DeclId,
        budget: &ir::PlanBudget,
    ) -> Result<tr::AssociatedConstHeader, String> {
        self.check_budget(budget)?;
        let info = self.registry.declaration(declaration)?;
        let source = source(&self.registry, info)?;
        let owner_id = info.owner.ok_or("associated constant has no owner")?;
        let owner = self.registry.declaration(owner_id)?;
        let mut bindings = generic_bindings(&self.registry, &info.env, budget)?;
        let caller = ir::FactsCtx::Universal(&info.env);
        let mut state = self.scratch();
        let previous = state.source_context;
        let result = (|| {
            state.source_context = Some(self.registry.source_type_context(owner_id)?);
            let (bounds, implementation) = match owner.kind {
                ir::DeclKind::Trait => (&self.program.traits[owner.source_index].bounds, None),
                ir::DeclKind::Impl | ir::DeclKind::TraitImpl => {
                    let block = &self.program.impls[owner.source_index];
                    let self_ty = state.source_ty(
                        &self.registry,
                        &block.self_ty,
                        &bindings,
                        caller,
                        budget,
                        0,
                        true,
                    )?;
                    bindings.insert(GenericBindingKey::SelfType, self_ty.clone());
                    let application = block
                        .trait_path
                        .as_ref()
                        .map(|path| {
                            state
                                .source_trait_application(
                                    &self.registry,
                                    path,
                                    &self_ty,
                                    &bindings,
                                    caller,
                                    budget,
                                )?
                                .reject_equalities()
                        })
                        .transpose()?;
                    (&block.bounds, application)
                }
                _ => {
                    return Err("associated constant owner is not a trait or implementation".into());
                }
            };
            let implemented_member = implementation
                .as_ref()
                .map(|application| {
                    let trait_info = self.registry.declaration(application.declaration)?;
                    let members =
                        &self.program.traits[trait_info.source_index].associated_constants;
                    for (index, member) in members.iter().enumerate() {
                        budget.charge_work(1)?;
                        if name_scope::ident_eq(
                            self.registry.names.as_ref(),
                            member.name,
                            source.name,
                            LookupClass::Member,
                        )? {
                            return Ok(self.registry.associated_constants.traits
                                [trait_info.source_index][index]);
                        }
                    }
                    Err(
                        "E0438: associated constant is not a member of the implemented trait"
                            .to_owned(),
                    )
                })
                .transpose()?;
            let mut predicates = Vec::new();
            append_source_bounds(
                &mut state,
                &self.registry,
                &bindings,
                bounds,
                &mut predicates,
                caller,
                budget,
            )?;
            if let Some(application) = implementation {
                budget.retain_bytes(std::mem::size_of::<ir::Predicate>())?;
                predicates
                    .try_reserve_exact(1)
                    .map_err(|_| "associated constant predicate allocation")?;
                predicates.push(ir::Predicate::Implements(application));
            }
            state.source_context = Some(self.registry.source_type_context(declaration)?);
            let ty = state.source_ty(
                &self.registry,
                &source.ty,
                &bindings,
                caller,
                budget,
                0,
                true,
            )?;
            Ok(tr::AssociatedConstHeader {
                declaration,
                owner: owner_id,
                owner_ty: if owner.kind == ir::DeclKind::Trait {
                    None
                } else {
                    bindings.get(&GenericBindingKey::SelfType).cloned()
                },
                implemented_member,
                env: info.env.clone(),
                ty,
                predicates: predicates.into(),
                has_initializer: source.initializer.is_some(),
            })
        })();
        state.source_context = previous;
        result
    }
}

impl Parser<'_, '_> {
    pub(super) fn associated_constant(
        &mut self,
        required_allowed: bool,
        visibility: ir::Visibility,
        parameters: &[AstIdent],
    ) -> Result<AssociatedConstDecl, ParseError> {
        self.expect("const")?;
        let name = self.ident()?;
        if self.is("<") {
            return Err(self.unsupported("generic associated constants are unsupported"));
        }
        let mut visible = std::mem::take(&mut self.type_params);
        visible.clear();
        self.reserve_parser_items(&mut visible, parameters.len())?;
        visible.extend_from_slice(parameters);
        self.type_params = visible;
        self.expect(":")?;
        let ty = self.ty()?;
        let initializer = if self.eat("=") {
            Some(self.nested_expr()?)
        } else if required_allowed {
            None
        } else {
            return Err(self.error("associated constant implementation requires an initializer"));
        };
        self.expect(";")?;
        Ok(AssociatedConstDecl {
            name,
            ty,
            initializer,
            visibility,
        })
    }
}
