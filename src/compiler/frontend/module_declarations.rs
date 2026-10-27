//! Per-module source declaration identities (SIGNATURES split, step a).
//!
//! A source `DeclId` is `SOURCE_BIT | module-path key | kind | ordinal`, where
//! the ordinal counts same-kind items of one module in source order. This
//! module recomputes those identities for a single module from the resolved
//! program alone, without the whole-program registry, so a per-module graph
//! node can publish them. `registry_ids_match_module_enumeration` proves the
//! two derivations agree.
use super::*;

/// Canonical source declaration identities owned by one module, in the
/// registry's enumeration order. Indices only: the snapshot is issued by the
/// consumer, so the fact is independent of catalog ownership.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ModuleDeclarations {
    pub module_key: u64,
    pub declarations: Vec<(ir::DeclKind, u64)>,
    /// `(trait impl index, implemented trait index)` for every trait impl of
    /// this module. Builtin traits use their core catalog index.
    pub impl_traits: Vec<(u64, u64)>,
}

#[allow(dead_code)] // traits() consumers land with per-trait facts
impl ModuleDeclarations {
    /// Trait declarations of this module (inputs of per-trait completeness).
    pub fn traits(&self) -> impl Iterator<Item = u64> + '_ {
        self.of(ir::DeclKind::Trait)
    }
    /// Trait implementation blocks of this module.
    pub fn trait_impls(&self) -> impl Iterator<Item = u64> + '_ {
        self.of(ir::DeclKind::TraitImpl)
    }
    fn of(&self, kind: ir::DeclKind) -> impl Iterator<Item = u64> + '_ {
        self.declarations
            .iter()
            .filter(move |(k, _)| *k == kind)
            .map(|(_, index)| *index)
    }
}

/// Bits of the module-path key carried inside a source `DeclId`.
pub const MODULE_KEY_MASK: u64 = (1 << 43) - 1;

/// Canonical module-path key of `module`.
pub fn module_key(program: &Program, module: ModuleId) -> Result<u64, String> {
    canonical_module_key(&program.modules, &program.names, module)
}

/// Enumerates the declarations owned by the module whose canonical key is
/// `key`, mirroring `PlanCtx` registration order kind by kind. Work is
/// charged per inspected item.
pub fn module_declarations(
    program: &Program,
    key: u64,
    budget: &ir::PlanBudget,
) -> Result<ModuleDeclarations, String> {
    let mut out = ModuleDeclarations {
        module_key: key & MODULE_KEY_MASK,
        declarations: Vec::new(),
        impl_traits: Vec::new(),
    };
    let mut ordinals: HashMap<u8, u64> = HashMap::new();
    let mut push = |kind: ir::DeclKind, out: &mut ModuleDeclarations| -> Result<(), String> {
        let ordinal = ordinals.entry(kind as u8).or_insert(0);
        out.declarations
            .push((kind, ir::DeclId::source_index(key, kind, *ordinal)?));
        *ordinal += 1;
        Ok(())
    };
    let owned = |module: ModuleId| -> Result<bool, String> {
        budget.charge_work(1)?;
        Ok(module_key(program, module)? & MODULE_KEY_MASK == key & MODULE_KEY_MASK)
    };
    for d in &program.structs {
        if owned(d.module)? {
            push(ir::DeclKind::Struct, &mut out)?;
        }
    }
    for d in &program.enums {
        if owned(d.module)? {
            push(ir::DeclKind::Enum, &mut out)?;
        }
    }
    for f in &program.functions {
        if owned(f.module)? {
            push(ir::DeclKind::Function, &mut out)?;
        }
    }
    for t in &program.traits {
        if !owned(t.module)? {
            continue;
        }
        push(ir::DeclKind::Trait, &mut out)?;
        for item in &t.items {
            push(ir::DeclKind::TraitItem, &mut out)?;
            if item.default.is_some() {
                push(ir::DeclKind::AssociatedFunction, &mut out)?;
            }
        }
        for _ in &t.associated_types {
            push(ir::DeclKind::AssociatedType, &mut out)?;
        }
        for _ in &t.associated_constants {
            push(ir::DeclKind::AssociatedConst, &mut out)?;
        }
    }
    for b in &program.impls {
        if !owned(b.module)? {
            continue;
        }
        let kind = if b.trait_path.is_some() {
            ir::DeclKind::TraitImpl
        } else {
            ir::DeclKind::Impl
        };
        push(kind, &mut out)?;
        if let Some(path) = &b.trait_path {
            let (_, implementation) = *out.declarations.last().ok_or("missing impl id")?;
            out.impl_traits
                .push((implementation, implemented_trait(program, path, budget)?));
        }
        for _ in &b.items {
            push(ir::DeclKind::AssociatedFunction, &mut out)?;
        }
        for _ in &b.associated_constants {
            push(ir::DeclKind::AssociatedConst, &mut out)?;
        }
    }
    for c in &program.constants {
        if owned(c.module)? {
            push(ir::DeclKind::Const, &mut out)?;
        }
    }
    Ok(out)
}

/// Canonical index of the trait `path` names, resolved exactly as the
/// registry does (by checked path key), without the registry.
fn implemented_trait(
    program: &Program,
    path: &TraitPath,
    budget: &ir::PlanBudget,
) -> Result<u64, String> {
    let path = match path {
        TraitPath::Builtin(kind) => {
            return Ok(match kind {
                BuiltinTrait::Sized => 0,
                BuiltinTrait::Clone => 1,
                BuiltinTrait::Copy => 5,
                BuiltinTrait::Drop => 6,
            });
        }
        TraitPath::Source(path) => path,
    };
    let names = program.names.as_ref();
    let want = name_scope::path_key(names, &path.path(names)?, Namespace::Type)?;
    let mut ordinals: HashMap<u64, u64> = HashMap::new();
    let mut found = None;
    for t in &program.traits {
        budget.charge_work(1)?;
        let key = module_key(program, t.module)? & MODULE_KEY_MASK;
        let ordinal = ordinals.entry(key).or_insert(0);
        if name_scope::path_key(names, &t.name, Namespace::Type)? == want {
            if found.is_some() {
                return Err("duplicate trait declaration".into());
            }
            found = Some(ir::DeclId::source_index(
                key,
                ir::DeclKind::Trait,
                *ordinal,
            )?);
        }
        *ordinal += 1;
    }
    found.ok_or_else(|| "unknown trait".into())
}

/// Canonical keys of every module owning at least one declaration, in first
/// declaration order. Input of explicit per-trait completeness facts.
pub fn declaring_modules(program: &Program, budget: &ir::PlanBudget) -> Result<Vec<u64>, String> {
    let mut keys = Vec::new();
    let mut seen = std::collections::HashSet::new();
    let modules = program
        .structs
        .iter()
        .map(|d| d.module)
        .chain(program.enums.iter().map(|d| d.module))
        .chain(program.functions.iter().map(|d| d.module))
        .chain(program.traits.iter().map(|d| d.module))
        .chain(program.impls.iter().map(|d| d.module))
        .chain(program.constants.iter().map(|d| d.module));
    for module in modules {
        budget.charge_work(1)?;
        let key = module_key(program, module)? & MODULE_KEY_MASK;
        if seen.insert(key) {
            keys.push(key);
        }
    }
    Ok(keys)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn registry_ids_match_module_enumeration() {
        let source = "mod a { pub struct S; pub trait T { fn f(&self); fn g(&self) {} type X; } \
                      impl T for S { fn f(&self) {} type X = i32; } impl S { fn h() {} } \
                      pub const C: i32 = 1; } struct R; impl a::T for R { fn f(&self) {} type X = u8; } \
                      #[derive(Clone)] struct Q; enum E { V } fn main() {}";
        let program = resolve(&parse_unresolved(source).unwrap()).unwrap();
        let budget = ir::PlanBudget::default();
        let plan = PlanCtx::from_program(&program, &budget).unwrap();
        let mut keys: Vec<u64> = Vec::new();
        let mut expected: HashMap<u64, Vec<(ir::DeclKind, u64)>> = HashMap::new();
        for info in &plan.registry.declarations {
            if !info.id.is_source() {
                continue;
            }
            let key = module_key(&program, info.path.module).unwrap();
            if !keys.contains(&key) {
                keys.push(key);
            }
            expected
                .entry(key)
                .or_default()
                .push((info.kind, info.id.index));
        }
        assert!(keys.len() >= 2, "fixture must span modules");
        for key in keys {
            let mut got = module_declarations(&program, key, &budget)
                .unwrap()
                .declarations;
            let mut want = expected.remove(&key).unwrap();
            got.sort_by_key(|(_, i)| *i);
            want.sort_by_key(|(_, i)| *i);
            assert_eq!(got, want, "module {key:#x}");
        }
        // Per-module impl->trait edges reassemble exactly the world's
        // per-trait completeness.
        let world = plan.trait_world().unwrap();
        let mut by_trait: HashMap<u64, Vec<u64>> = HashMap::new();
        for key in declaring_modules(&program, &budget).unwrap() {
            for (implementation, implemented) in module_declarations(&program, key, &budget)
                .unwrap()
                .impl_traits
            {
                by_trait
                    .entry(implemented)
                    .or_default()
                    .push(implementation);
            }
        }
        assert!(!by_trait.is_empty());
        for (implemented, mut got) in by_trait {
            let trait_decl = ir::DeclId {
                snapshot: plan.registry.trait_ids[0].snapshot,
                index: implemented,
            };
            let mut want: Vec<u64> = world
                .trait_impls(trait_decl)
                .into_iter()
                .map(|d| d.index)
                .collect();
            got.sort();
            want.sort();
            assert_eq!(got, want, "trait {implemented:#x}");
        }
    }
}
