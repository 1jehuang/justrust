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
}

#[allow(dead_code)] // TRAIT_IMPLS/MODULE_DECLARATIONS consumers land next
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
        module_key: key,
        declarations: Vec::new(),
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
        Ok(module_key(program, module)? == key)
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn registry_ids_match_module_enumeration() {
        let source = "mod a { pub struct S; pub trait T { fn f(&self); fn g(&self) {} type X; } \
                      impl T for S { fn f(&self) {} type X = i32; } impl S { fn h() {} } \
                      pub const C: i32 = 1; } struct R; enum E { V } fn main() {}";
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
    }
}
