//! Transport authored owner context only after associated declarations are final.
use super::*;

pub(super) fn install_closure_source_contexts(
    contexts: &mut HashMap<ir::DeclId, CallableContext>,
    registry: &DeclRegistry,
    budget: &ir::PlanBudget,
) -> Result<(), String> {
    for declaration in &registry.declarations {
        let Some(function) = registry.source_callable(declaration.id) else {
            continue;
        };
        let key = name_scope::path_key(registry.names.as_ref(), &function.name, Namespace::Type)?;
        let Some(source) = registry
            .program
            .closure_sources
            .get(&key)
            .filter(|source| source.stored.is_some())
        else {
            continue;
        };
        let mut owner = registry.closure_source_owner(source)?;
        let mut visited = Vec::new();
        loop {
            budget.check_depth(visited.len())?;
            budget.charge_work(1)?;
            if visited.contains(&owner) {
                return Err("closure source context owner cycle".into());
            }
            budget.retain_bytes(std::mem::size_of::<ir::DeclId>())?;
            visited.push(owner);
            let parent = registry
                .source_callable(owner)
                .ok_or("closure source context has no authored owner")?;
            let parent_key =
                name_scope::path_key(registry.names.as_ref(), &parent.name, Namespace::Type)?;
            if let Some(parent_source) = registry
                .program
                .closure_sources
                .get(&parent_key)
                .filter(|source| source.stored.is_some())
            {
                owner = registry.closure_source_owner(parent_source)?;
            } else {
                break;
            }
        }
        if let Some(mut context) = contexts.get(&owner).cloned() {
            if context.module != declaration.path.module {
                return Err("closure source context changed its authored module".into());
            }
            // The adapter has explicit [receiver, tuple] parameters, not
            // a method receiver. Retain Self and associated-owner context
            // solely for checking the actual child source body.
            context.receiver = ReceiverKind::None;
            budget.retain_bytes(std::mem::size_of::<CallableContext>())?;
            contexts.insert(declaration.id, context);
        }
    }
    Ok(())
}
