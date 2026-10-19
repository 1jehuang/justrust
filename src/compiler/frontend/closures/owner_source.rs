//! Exact source ownership for stored closures in free, implementation and
//! default trait bodies. A method spelling is never a declaration identity.
use super::*;

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
}

#[cfg(test)]
mod tests {
    use super::*;

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
