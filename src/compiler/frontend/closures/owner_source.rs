//! Exact source ownership for stored closures in free, implementation and
//! default trait bodies. A method spelling is never a declaration identity.
use super::*;

impl PlanCtx {
    /// Stored nested bodies are published with their outermost source callable.
    /// Keep the immediate owner/site separate: they still authenticate selection
    /// from the ancestor's immutable closure publication.
    pub(in crate::compiler) fn closure_publication_owner(
        &self,
        declaration: ir::DeclId,
        budget: &ir::PlanBudget,
    ) -> Result<ir::DeclId, String> {
        self.check_budget(budget)?;
        let mut owner = declaration;
        for depth in 0..=ir::MAX_PATTERN_PROJECTION_DEPTH {
            budget.check_depth(depth)?;
            budget.charge_work(1)?;
            match self.stored_closure_owner(owner)? {
                Some((parent, _)) => owner = parent,
                None => return Ok(owner),
            }
        }
        Err("closure publication owner chain exceeds depth limit".into())
    }
}

impl DeclRegistry {
    pub(in crate::compiler::frontend) fn closure_source_owner(
        &self,
        source: &ClosureSource,
    ) -> Result<ir::DeclId, String> {
        let key = name_scope::path_key(self.names.as_ref(), &source.name, Namespace::Type)?;
        let original = self
            .program
            .closure_sources
            .get(&key)
            .ok_or("stored closure source is absent from its defining program")?;
        if !std::ptr::eq(original, source) {
            return Err("stored closure source belongs to another publication".into());
        }
        let owner = if let Some(ordinal) = source.owner_source {
            if source.parent.is_some() {
                return Err("stored closure has conflicting canonical owners".into());
            }
            let owner = match ordinal {
                CallableSource::Associated {
                    implementation,
                    item,
                } => self
                    .associated_ids
                    .get(implementation)
                    .and_then(|items| items.get(item))
                    .copied(),
                CallableSource::TraitDefault { trait_index, item } => self
                    .trait_default_callables
                    .get(&(trait_index, item))
                    .copied(),
                CallableSource::Free(_) => {
                    return Err(
                        "stored closure free owner requires its retained parent path".into(),
                    );
                }
            }
            .ok_or("stored closure owner ordinal is outside its defining program")?;
            let registered = self
                .function_position(owner)
                .and_then(|index| self.callable_sources.get(index));
            if registered != Some(&ordinal) {
                return Err(
                    "stored closure owner ordinal does not identify a callable body".into(),
                );
            }
            owner
        } else {
            let parent = source
                .parent
                .as_ref()
                .ok_or("stored closure lacks canonical parent")?;
            let parent_key =
                name_scope::path_key(self.names.as_ref(), &parent.name, Namespace::Value)?;
            *self
                .functions
                .get(&parent_key)
                .ok_or("stored closure parent is not a callable")?
        };
        // Ordinals alone cannot authorize moving a closure to a different body.
        // Inspect only the selected owner's retained syntax, never all callables.
        let function = self
            .source_callable(owner)
            .ok_or("stored closure owner has no source body")?;
        let owner_key = name_scope::path_key(self.names.as_ref(), &function.name, Namespace::Type)?;
        let body = self
            .program
            .closure_sources
            .get(&owner_key)
            .and_then(|source| source.stored.as_ref())
            .map_or(&function.body, |syntax| &syntax.body);
        let mut found = false;
        let mut inspect = |node: &DepthNode<'_>, _: usize| -> Result<(), String> {
            if let DepthNode::Expr(Expr::Var(path)) = node {
                found |= name_scope::path_key(self.names.as_ref(), path, Namespace::Type)? == key;
            }
            Ok(())
        };
        walk_depth_inspecting(
            DepthNode::Block(body),
            None,
            Some(self.names.as_ref()),
            Some(&mut inspect),
        )?;
        if !found {
            return Err("stored closure source site is not retained by its canonical owner".into());
        }
        Ok(owner)
    }

    pub(in crate::compiler::frontend) fn retained_stored_closure_projection(
        &self,
        owner: ir::DeclId,
        body: &Block,
        projection: &SourceProjection,
        budget: &ir::PlanBudget,
        depth: usize,
    ) -> Result<bool, String> {
        budget.check_depth(depth)?;
        budget.charge_work(1)?;
        let names = self.names.as_ref();
        let mut found = false;
        let mut inspect = |node: &DepthNode<'_>, node_depth: usize| -> Result<(), String> {
            budget.check_depth(depth.saturating_add(node_depth))?;
            budget.charge_work(1)?;
            let DepthNode::Expr(Expr::Var(path)) = node else {
                return Ok(());
            };
            let key = name_scope::path_key(names, path, Namespace::Type)?;
            let Some(source) = self.program.closure_sources.get(&key) else {
                return Ok(());
            };
            let Some(syntax) = &source.stored else {
                return Ok(());
            };
            // Pointer-original source, exact canonical owner and the authored
            // constructor occurrence must all replay before inspecting syntax.
            if self.closure_source_owner(source)? != owner {
                return Err("stored projection source changed canonical closure owner".into());
            }
            for (_, ty) in &syntax.params {
                found |= retained_type_projection(names, ty, projection, budget)?;
            }
            found |= retained_type_projection(names, &syntax.result, projection, budget)?;
            found |= retained_node_projection(
                names,
                DepthNode::Block(&syntax.body),
                projection,
                budget,
            )?;
            let callable_key = name_scope::path_key(names, &source.name, Namespace::Value)?;
            let callable = self
                .functions
                .get(&callable_key)
                .copied()
                .ok_or("stored projection source has no callable declaration")?;
            found |= self.retained_stored_closure_projection(
                callable,
                &syntax.body,
                projection,
                budget,
                depth + 1,
            )?;
            Ok(())
        };
        walk_depth_inspecting(
            DepthNode::Block(body),
            None,
            Some(names),
            Some(&mut inspect),
        )?;
        Ok(found)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn nested_closure_publication_owner_preserves_exact_lexical_chain() {
        let program =
            parse("fn main(){let f=||{let g=||1i32;1i32};}fn other(){let h=||2i32;}").unwrap();
        let budget = ir::PlanBudget::default();
        let plan = PlanCtx::from_program(&program, &budget).unwrap();
        let mut nested = None;
        for source in plan.registry.program.closure_sources.values() {
            let key = name_scope::path_key(program.names.as_ref(), &source.name, Namespace::Value)
                .unwrap();
            let provider = *plan.registry.functions.get(&key).unwrap();
            let Some((parent, _)) = plan.stored_closure_owner(provider).unwrap() else {
                continue;
            };
            if let Some((root, _)) = plan.stored_closure_owner(parent).unwrap() {
                assert_eq!(
                    plan.closure_publication_owner(provider, &budget).unwrap(),
                    root
                );
                assert_ne!(parent, root);
                nested = Some(source.name.clone());
            }
        }
        let nested = nested.expect("real nested closure source");
        let key = name_scope::path_key(program.names.as_ref(), &nested, Namespace::Type).unwrap();
        let mut forged = program.clone();
        let other = forged
            .functions
            .iter()
            .find(|f| forged.names.spelling(f.name.segments()[0]).unwrap() == "other")
            .unwrap()
            .name
            .clone();
        Arc::make_mut(&mut forged.closure_sources)
            .get_mut(&key)
            .unwrap()
            .parent
            .as_mut()
            .unwrap()
            .name = other;
        if let Ok(forged_plan) = PlanCtx::from_program(&forged, &budget) {
            let provider_key =
                name_scope::path_key(forged.names.as_ref(), &nested, Namespace::Value).unwrap();
            let provider = *forged_plan.registry.functions.get(&provider_key).unwrap();
            assert!(
                forged_plan
                    .closure_publication_owner(provider, &budget)
                    .is_err()
            );
        }
    }

    #[test]
    fn generic_method_and_trait_default_keep_source_owner_ordinals() {
        for source in [
            "struct A<T>{value:T}impl<T>A<T>{fn run()->i32{let f=||1i32;1}}fn main(){}",
            "trait Read{fn run()->i32{let f=||3i32;3}}struct A;impl Read for A{}fn main(){}",
        ] {
            let program = parse(source).unwrap();
            let budget = ir::PlanBudget::default();
            let plan = PlanCtx::from_program(&program, &budget).unwrap();
            let mut count = 0;
            for closure in plan.registry.program.closure_sources.values() {
                if closure.stored.is_some() && closure.owner_source.is_some() {
                    plan.registry.closure_source_owner(closure).unwrap();
                    count += 1;
                }
            }
            assert_eq!(count, 1);
        }
    }

    #[test]
    fn associated_closure_owner_requires_exact_ordinal_site_and_publication() {
        let program = parse("struct A;impl A{fn one()->i32{let f=||1i32;1}fn two()->i32{let f=||2i32;2}}fn main(){}").unwrap();
        let budget = ir::PlanBudget::default();
        let plan = PlanCtx::from_program(&program, &budget).unwrap();
        let source = plan
            .registry
            .program
            .closure_sources
            .values()
            .find(|source| {
                source.owner_source
                    == Some(CallableSource::Associated {
                        implementation: 0,
                        item: 0,
                    })
            })
            .expect("method closure owner was not published after implementation resolution");
        let owner = plan.registry.closure_source_owner(source).unwrap();
        assert_eq!(owner, plan.registry.associated_ids[0][0]);
        assert!(plan.registry.closure_source_owner(&source.clone()).is_err());
        let key =
            name_scope::path_key(program.names.as_ref(), &source.name, Namespace::Type).unwrap();
        for wrong in [
            CallableSource::Associated {
                implementation: 0,
                item: 1,
            },
            CallableSource::Associated {
                implementation: usize::MAX,
                item: 0,
            },
            CallableSource::TraitDefault {
                trait_index: 0,
                item: 0,
            },
        ] {
            let mut forged = program.clone();
            Arc::make_mut(&mut forged.closure_sources)
                .get_mut(&key)
                .unwrap()
                .owner_source = Some(wrong);
            if let Ok(forged_plan) = PlanCtx::from_program(&forged, &budget) {
                let foreign = forged_plan
                    .registry
                    .program
                    .closure_sources
                    .get(&key)
                    .unwrap();
                assert!(forged_plan.registry.closure_source_owner(foreign).is_err());
            }
        }
    }
}
