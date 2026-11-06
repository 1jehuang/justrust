//! Glob trait scope is produced by module resolution, never guessed by method lookup.
use super::*;
use std::collections::HashSet;

pub(super) fn collect(
    namespace: &ResolutionNamespace,
    ordinal: usize,
    resolver: &mut Resolver<'_>,
    imports: &mut Vec<(usize, ModuleId, TraitPath)>,
) -> Result<(), String> {
    let module = *namespace.modules.get(ordinal).ok_or("foreign glob scope")?;
    let scope = resolver.module_path(module)?;
    resolver.charge_entries::<usize>(1)?;
    let mut pending = vec![ordinal];
    let mut visited = HashSet::new();
    let mut names = HashMap::new();
    while let Some(current) = pending.pop() {
        resolver.alias_budget.charge_work(1)?;
        if visited.contains(&current) {
            continue;
        }
        resolver.charge_entries::<usize>(1)?;
        visited.insert(current);
        if current != ordinal {
            for &order in &namespace.declared[current] {
                resolver.alias_budget.charge_work(1)?;
                let path = &namespace.paths[order];
                let Some(ident) = path.segments().last().copied() else {
                    continue;
                };
                // Only trait bindings can contribute method scope. Resolve at
                // their declaring scope first, retaining canonical import authority.
                let owner = namespace.modules[current];
                let Some(item) = resolver.item(path, owner, true, true)? else {
                    continue;
                };
                if item.kind != ResolvedKind::Trait {
                    continue;
                }
                let key = resolver
                    .names()
                    .key(ident, LookupClass::Item(Namespace::Type))
                    .map_err(native_adapter::name_diagnostic)?;
                resolver.charge_entries::<(NameKey, (usize, AstIdent))>(1)?;
                names.entry(key).or_insert((order, ident));
            }
        }
        for &index in &namespace.globs[current] {
            resolver.alias_budget.charge_work(1)?;
            let glob = &namespace.parsed.glob_imports[index];
            let Some(target) = resolver.path(&glob.target, glob.module, true, true)? else {
                continue;
            };
            let ResolvedPath::Source(path) = &target.path else {
                continue;
            };
            let target_key = resolver.key(path)?;
            for (next, candidate) in namespace.modules.iter().enumerate() {
                resolver.alias_budget.charge_work(1)?;
                if resolver.key(&resolver.module_path(*candidate)?)? == target_key {
                    resolver.charge_entries::<usize>(1)?;
                    pending.push(next);
                    break;
                }
            }
        }
    }
    resolver.charge_entries::<(usize, AstIdent)>(names.len())?;
    let mut names = names.into_values().collect::<Vec<_>>();
    names.sort_by_key(|(order, _)| *order);
    for (order, ident) in names {
        let path = resolver.semantic_append(&scope, ident)?;
        // Re-resolve from the importing module. This enforces private glob
        // edges, explicit-binding precedence, ambiguity and cycle cuts.
        let Some(item) = resolver.item(&path, module, true, true)? else {
            continue;
        };
        if item.kind != ResolvedKind::Trait {
            continue;
        }
        let path = match item.path {
            ResolvedPath::Source(path) => TraitPath::Source(resolver.alias_source_path(&path)?),
            ResolvedPath::Builtin(id) => TraitPath::Builtin(id),
            _ => return Err("trait glob resolved to a non-trait authority".into()),
        };
        resolver.charge_entries::<(usize, ModuleId, TraitPath)>(1)?;
        imports.push((order, module, path));
    }
    Ok(())
}
