//! Frontend adapter: validated declarations to the semantic core.
//!
//! `WorldDb` maps one validated trait world and declaration registry onto
//! `sema` types. It is built per check request from immutable published
//! facts (PlanCtx/ValidatedTraitWorld) and owns only immutable tables.
//! Query shapes match the `TraitDb` contract: per trait, per impl, per
//! callable and per simplified self type.
//!
//! Coverage today: declarations, impls, traits, callables, nominal fields.
//! Types the core cannot represent yet (opaque, dyn, closures from the old
//! pipeline) map to `Error` and are reported as unsupported by callers.

use super::check::{AdtData, CheckLang, FnSig, ItemDb, Owner, VariantData};
use super::hir::BinOp;
use super::solve::{ImplData, LangItems, TraitData, TraitDb};
use super::ty::{self, DefId, Mutbl, SimplifiedType, Ty, TyKind};
use crate::compiler::frontend::PlanCtx;
use crate::compiler::ir;
use std::collections::HashMap;
use std::sync::Arc;

#[derive(Debug, Default)]
pub struct WorldDb {
    traits: HashMap<DefId, TraitData>,
    impls: HashMap<DefId, ImplData>,
    impls_of: HashMap<DefId, Vec<DefId>>,
    inherent: HashMap<SimplifiedType, Vec<DefId>>,
    impl_items: HashMap<DefId, Vec<(String, DefId)>>,
    impl_consts: HashMap<DefId, Vec<(String, Ty)>>,
    fns: HashMap<DefId, FnSig>,
    adts: HashMap<DefId, AdtData>,
    lang: LangItems,
    check_lang: CheckLang,
    /// Unsupported constructs met while mapping, by declaration.
    pub unsupported: Vec<(DefId, &'static str)>,
    /// Trait name to id, for lang-item discovery and diagnostics.
    pub trait_names: HashMap<DefId, String>,
    /// Struct/enum name for diagnostics.
    pub adt_names: HashMap<DefId, String>,
    empty_impls: Vec<DefId>,
    empty_items: Vec<(String, DefId)>,
}

impl TraitDb for WorldDb {
    fn trait_data(&self, t: DefId) -> &TraitData {
        &self.traits[&t]
    }
    fn impls_of(&self, t: DefId) -> &[DefId] {
        self.impls_of.get(&t).map_or(&self.empty_impls, |v| v)
    }
    fn impl_data(&self, i: DefId) -> &ImplData {
        &self.impls[&i]
    }
    fn inherent_impls(&self, k: SimplifiedType) -> &[DefId] {
        self.inherent.get(&k).map_or(&self.empty_impls, |v| v)
    }
    fn lang(&self) -> &LangItems {
        &self.lang
    }
}

impl ItemDb for WorldDb {
    fn fn_sig(&self, f: DefId) -> &FnSig {
        &self.fns[&f]
    }
    fn adt(&self, a: DefId) -> &AdtData {
        &self.adts[&a]
    }
    fn impl_items(&self, i: DefId) -> &[(String, DefId)] {
        self.impl_items.get(&i).map_or(&self.empty_items, |v| v)
    }
    fn impl_consts(&self, i: DefId) -> &[(String, Ty)] {
        self.impl_consts.get(&i).map_or(&[], |v| v)
    }
    fn check_lang(&self) -> &CheckLang {
        &self.check_lang
    }
    fn as_trait_db(&self) -> &dyn TraitDb {
        self
    }
}

pub fn def(id: ir::DeclId) -> DefId {
    id.index
}

/// Maps `ir::Ty` (over a declaration's GenericEnv) to `sema` types, with
/// parameters numbered by their position in the env's ordered list.
pub struct TyMap<'a> {
    pub budget: &'a ir::PlanBudget,
    pub env: &'a ir::GenericEnv,
    /// Extra mapping for parameters of another binder (for example trait
    /// `Self` seen from an impl), position in the target list.
    pub extra: &'a [(ir::ParamId, u32)],
}

impl TyMap<'_> {
    fn param_index(&self, p: ir::ParamId) -> Option<u32> {
        if let Some((_, i)) = self.extra.iter().find(|(q, _)| *q == p) {
            return Some(*i);
        }
        self.env
            .ordered_params()
            .position(|q| q == p)
            .map(|i| i as u32)
    }

    pub fn ty(&self, t: &ir::Ty, unsupported: &mut Option<&'static str>) -> Ty {
        let mut un = |m: &'static str| {
            unsupported.get_or_insert(m);
            ty::error()
        };
        match t {
            ir::Ty::Bool => ty::bool_ty(),
            ir::Ty::Char => ty::mk(TyKind::Char),
            ir::Ty::Str => ty::mk(TyKind::Str),
            ir::Ty::Int(i) => ty::int(*i),
            ir::Ty::Float(f) => ty::mk(TyKind::Float(*f)),
            ir::Ty::Unit => ty::unit(),
            ir::Ty::Never => ty::mk(TyKind::Never),
            ir::Ty::Tuple(v) => ty::mk(TyKind::Tuple(
                v.iter().map(|x| self.ty(x, unsupported)).collect(),
            )),
            ir::Ty::Ref { kind, referent } => {
                ty::reference(mutbl(*kind), self.ty(referent, unsupported))
            }
            ir::Ty::RawPtr { kind, pointee } => {
                ty::mk(TyKind::RawPtr(mutbl(*kind), self.ty(pointee, unsupported)))
            }
            ir::Ty::Array { element, len } => {
                ty::mk(TyKind::Array(self.ty(element, unsupported), *len as u64))
            }
            ir::Ty::Slice { element } => ty::mk(TyKind::Slice(self.ty(element, unsupported))),
            ir::Ty::Struct(s) => {
                let d = s.def();
                ty::adt(
                    def(d.source_decl),
                    d.type_args
                        .iter()
                        .map(|x| self.ty(x, unsupported))
                        .collect(),
                )
            }
            ir::Ty::Enum(e) => {
                let d = e.def();
                ty::adt(
                    def(d.source_decl),
                    d.type_args
                        .iter()
                        .map(|x| self.ty(x, unsupported))
                        .collect(),
                )
            }
            ir::Ty::Param(p) => match self.param_index(*p) {
                Some(i) => ty::param(i),
                None => un("parameter outside its generic environment"),
            },
            ir::Ty::AssociatedType(p) => {
                let tr = self.trait_ref(&p.trait_ref, unsupported);
                ty::mk(TyKind::Projection(Arc::new(ty::ProjectionTy {
                    trait_ref: tr,
                    item: def(p.item),
                })))
            }
            ir::Ty::PhantomData(_) => un("PhantomData type in the semantic core"),
            ir::Ty::Opaque(_) => un("opaque type in the semantic core"),
            ir::Ty::Closure(_) => un("closure type in a declaration"),
            ir::Ty::FnPointer(pointer) => {
                let Ok(mut inputs) = ir::FnPointerTy::storage(pointer.inputs.len(), self.budget)
                else {
                    return un("function pointer semantic scratch allocation failed");
                };
                if self
                    .budget
                    .retain_bytes(std::mem::size_of::<TyKind>() + 2 * std::mem::size_of::<usize>())
                    .is_err()
                {
                    return un("function pointer semantic backing resource limit");
                }
                for t in pointer.inputs.iter() {
                    inputs.push(self.ty(t, unsupported));
                }
                ty::mk(TyKind::FnPtr(inputs, self.ty(&pointer.output, unsupported)))
            }
            ir::Ty::FnItem(_) => un("function item type in a declaration"),
            ir::Ty::Dyn(_) => un("trait object type in the semantic core"),
        }
    }

    pub fn trait_ref(
        &self,
        t: &ir::TraitRef,
        unsupported: &mut Option<&'static str>,
    ) -> ty::TraitRef {
        ty::TraitRef {
            trait_id: def(t.declaration),
            self_ty: self.ty(&t.self_ty, unsupported),
            args: t.args.iter().map(|a| self.ty(a, unsupported)).collect(),
        }
    }

    pub fn pred(&self, p: &ir::Predicate, unsupported: &mut Option<&'static str>) -> ty::Predicate {
        match p {
            ir::Predicate::Implements(t) => ty::Predicate::Trait(self.trait_ref(t, unsupported)),
            ir::Predicate::ProjectionEq {
                projection,
                ty: rhs,
            } => ty::Predicate::Projection(
                ty::ProjectionTy {
                    trait_ref: self.trait_ref(&projection.trait_ref, unsupported),
                    item: def(projection.item),
                },
                self.ty(rhs, unsupported),
            ),
        }
    }
}

pub fn mutbl(k: ir::BorrowKind) -> Mutbl {
    match k {
        ir::BorrowKind::Shared => Mutbl::Not,
        ir::BorrowKind::Mut => Mutbl::Mut,
    }
}

impl WorldDb {
    /// Build from a validated plan. Each table is keyed per item; nothing
    /// here aggregates results across bodies.
    pub fn build(plan: &PlanCtx) -> Result<Self, String> {
        let world = plan.trait_world()?;
        let headers = world.all_headers();
        let mut db = WorldDb::default();
        let note = |db: &mut WorldDb, id: DefId, u: Option<&'static str>| {
            if let Some(m) = u {
                db.unsupported.push((id, m));
            }
        };

        let mut item_owner: HashMap<DefId, DefId> = HashMap::new();
        for h in headers.traits.iter() {
            let id = def(h.declaration);
            for it in h.items.iter() {
                item_owner.insert(def(*it), id);
            }
            let map = TyMap {
                budget: &plan.budget,
                env: &h.env,
                extra: &[],
            };
            let mut u = None;
            let supertraits = h
                .supertraits
                .iter()
                .map(|s| map.trait_ref(s, &mut u))
                .collect();
            let generics = (h.env.total_arity() as u32).saturating_sub(1);
            db.traits.insert(
                id,
                TraitData {
                    id,
                    generics,
                    supertraits,
                    assoc_types: h.associated_types.iter().map(|d| def(*d)).collect(),
                    methods: Vec::new(),
                },
            );
            note(&mut db, id, u);
        }

        for h in headers.impls.iter() {
            let id = def(h.declaration);
            let map = TyMap {
                budget: &plan.budget,
                env: &h.schema_env,
                extra: &[],
            };
            let mut u = None;
            let trait_ref = map.trait_ref(&h.trait_ref, &mut u);
            let self_ty = trait_ref.self_ty.clone();
            let predicates = h.predicates.iter().map(|p| map.pred(p, &mut u)).collect();
            let assoc_types = h
                .associated_types
                .iter()
                .map(|a| (def(a.associated), map.ty(&a.rhs, &mut u)))
                .collect();
            db.impls_of.entry(trait_ref.trait_id).or_default().push(id);
            db.impls.insert(
                id,
                ImplData {
                    id,
                    generics: h.schema_env.total_arity() as u32,
                    trait_ref: Some(trait_ref),
                    self_ty,
                    predicates,
                    assoc_types,
                },
            );
            note(&mut db, id, u);
        }

        // Trait items: signatures come from the validated item headers.
        let mut names: HashMap<DefId, String> = HashMap::new();
        for info in plan.sema_declarations() {
            if info.kind == ir::DeclKind::TraitItem {
                names.insert(def(info.id), plan.sema_item_name(info).unwrap_or_default());
            }
        }
        for h in headers.items.iter() {
            let id = def(h.declaration);
            let s = &h.schema;
            let map = TyMap {
                budget: &plan.budget,
                env: &s.env,
                extra: &[],
            };
            let mut u = None;
            let inputs = s
                .signature
                .params
                .iter()
                .map(|t| map.ty(t, &mut u))
                .collect();
            let output = map.ty(&s.signature.result, &mut u);
            let predicates = s.predicates.iter().map(|p| map.pred(p, &mut u)).collect();
            let total = s.env.total_arity() as u32;
            let parent = s.env.scopes().first().map_or(0, |p| p.arity() as u32);
            let t = def(h.trait_decl);
            let name = names.get(&id).cloned().unwrap_or_default();
            if let Some(td) = db.traits.get_mut(&t) {
                td.methods.push((name, id));
            }
            db.fns.insert(
                id,
                FnSig {
                    generics: total,
                    parent_generics: parent,
                    inputs,
                    output,
                    predicates,
                    owner: Some(Owner::Trait(t)),
                    has_self: !matches!(h.receiver, ir::ReceiverMode::None),
                },
            );
            note(&mut db, id, u);
        }

        // Callables: free functions, inherent and trait methods, trait items.
        for info in plan.sema_declarations() {
            let is_callable = matches!(
                info.kind,
                ir::DeclKind::Function | ir::DeclKind::AssociatedFunction | ir::DeclKind::TraitItem
            );
            if !is_callable {
                continue;
            }
            let Some(schema) = plan.sema_schema(info.id) else {
                continue;
            };
            let id = def(info.id);
            let env = &schema.env;
            let map = TyMap {
                budget: &plan.budget,
                env,
                extra: &[],
            };
            let mut u = None;
            let inputs = schema
                .signature
                .params
                .iter()
                .map(|t| map.ty(t, &mut u))
                .collect();
            let output = map.ty(&schema.signature.result, &mut u);
            let predicates = schema
                .predicates
                .predicates()
                .iter()
                .map(|p| map.pred(p, &mut u))
                .collect();
            let own = info.param_env.arity() as u32;
            let total = env.total_arity() as u32;
            let has_self = !matches!(info.receiver, crate::compiler::frontend::ReceiverKind::None);
            let name = plan.sema_item_name(info).unwrap_or_default();
            let owner = match &info.path.associated {
                _ if info.kind == ir::DeclKind::TraitItem => {
                    item_owner.get(&id).map(|t| Owner::Trait(*t))
                }
                Some(a) => {
                    let impl_id = def(a.impl_decl);
                    if a.trait_decl.is_none() {
                        // Inherent impl: register by simplified self type.
                        if let std::collections::hash_map::Entry::Vacant(e) =
                            db.impls.entry(impl_id)
                        {
                            let self_ty = map.ty(&a.self_pattern, &mut u);
                            if let Some(key) = ty::simplify(&self_ty) {
                                db.inherent.entry(key).or_default().push(impl_id);
                            }
                            e.insert(ImplData {
                                id: impl_id,
                                generics: total - own,
                                trait_ref: None,
                                self_ty,
                                predicates: Vec::new(),
                                assoc_types: Vec::new(),
                            });
                        }
                    }
                    db.impl_items
                        .entry(impl_id)
                        .or_default()
                        .push((name.clone(), id));
                    Some(Owner::Impl(impl_id))
                }
                None => None,
            };
            if let Some(Owner::Trait(t)) = owner
                && let Some(td) = db.traits.get_mut(&t)
            {
                td.methods.push((name.clone(), id));
            }
            db.fns.insert(
                id,
                FnSig {
                    generics: total,
                    parent_generics: total - own,
                    inputs,
                    output,
                    predicates,
                    owner,
                    has_self,
                },
            );
            note(&mut db, id, u);
        }

        db.discover_lang(plan);
        // Inherent associated constants, typed from their validated header
        // over the owning impl's environment. Trait constants are not mapped.
        for decl in plan.associated_constant_declarations() {
            let Ok(h) = plan.associated_constant_header(decl, &plan.budget) else {
                db.unsupported
                    .push((def(decl), "associated constant header unavailable"));
                continue;
            };
            let owner = def(h.owner);
            // Trait-impl constants are not mapped yet (explicit Unsupported).
            let Ok(owner_info) = plan.declaration(h.owner) else {
                continue;
            };
            if owner_info.kind != ir::DeclKind::Impl {
                continue;
            }
            let map = TyMap {
                budget: &plan.budget,
                env: &h.env,
                extra: &[],
            };
            let mut u = None;
            // Constant-only inherent impls (`impl i32 { const MIN }`) have no
            // callable to register them above: register from the header.
            if !db.impls.contains_key(&owner) {
                let Some(owner_ty) = &h.owner_ty else {
                    continue;
                };
                let self_ty = map.ty(owner_ty, &mut u);
                if let Some(key) = ty::simplify(&self_ty) {
                    db.inherent.entry(key).or_default().push(owner);
                }
                db.impls.insert(
                    owner,
                    ImplData {
                        id: owner,
                        generics: h.env.total_arity() as u32,
                        trait_ref: None,
                        self_ty,
                        predicates: Vec::new(),
                        assoc_types: Vec::new(),
                    },
                );
            }
            let Ok(info) = plan.declaration(decl) else {
                continue;
            };
            let Some(name) = plan.sema_item_name(info) else {
                continue;
            };
            let t = map.ty(&h.ty, &mut u);
            note(&mut db, def(decl), u);
            db.impl_consts.entry(owner).or_default().push((name, t));
        }
        // Field tables for every nominal, from its symbolic schema.
        for info in plan.sema_declarations() {
            if !matches!(info.kind, ir::DeclKind::Struct | ir::DeclKind::Enum) {
                continue;
            }
            let Ok((penv, schema)) = plan.nominal_schema(info.id, &plan.budget) else {
                db.unsupported
                    .push((def(info.id), "nominal schema unavailable"));
                continue;
            };
            let Ok(env) = ir::GenericEnv::new(&[penv], &plan.budget) else {
                continue;
            };
            let variants = match &schema {
                ir::Ty::Struct(s) => vec![
                    s.def()
                        .fields
                        .read(&plan.budget)?
                        .iter()
                        .map(|f| (f.name.clone(), f.ty.clone()))
                        .collect(),
                ],
                ir::Ty::Enum(e) => e
                    .def()
                    .variants
                    .iter()
                    .map(|v| {
                        Ok::<_, String>(
                            v.fields
                                .read(&plan.budget)?
                                .iter()
                                .map(|f| (f.name.clone(), f.ty.clone()))
                                .collect(),
                        )
                    })
                    .collect::<Result<Vec<_>, String>>()?,
                _ => continue,
            };
            db.add_adt(info.id, &env, variants, &plan.budget);
        }
        Ok(db)
    }

    fn discover_lang(&mut self, plan: &PlanCtx) {
        let mut by_name: HashMap<String, DefId> = HashMap::new();
        let mut assoc_by: HashMap<(DefId, String), DefId> = HashMap::new();
        for info in plan.sema_declarations() {
            let Some(name) = plan.sema_item_name(info) else {
                continue;
            };
            match info.kind {
                ir::DeclKind::Trait => {
                    self.trait_names.insert(def(info.id), name.clone());
                    by_name.entry(name).or_insert(def(info.id));
                }
                ir::DeclKind::AssociatedType => {
                    if let Some(owner) = info.owner {
                        assoc_by.insert((def(owner), name), def(info.id));
                    }
                }
                ir::DeclKind::Struct | ir::DeclKind::Enum => {
                    self.adt_names.insert(def(info.id), name);
                }
                _ => {}
            }
        }
        let t = |n: &str| by_name.get(n).copied();
        let a = |tr: Option<DefId>, n: &str| {
            tr.and_then(|tr| assoc_by.get(&(tr, n.to_string())).copied())
        };
        let fn_once = t("FnOnce");
        let iterator = t("Iterator");
        let into_iter = t("IntoIterator");
        let deref = t("Deref");
        self.lang = LangItems {
            sized: t("Sized"),
            copy: t("Copy"),
            clone: t("Clone"),
            fn_: t("Fn"),
            fn_mut: t("FnMut"),
            fn_once,
            fn_once_output: a(fn_once, "Output"),
            deref,
            deref_target: a(deref, "Target"),
            iterator,
            iterator_item: a(iterator, "Item"),
            into_iterator: into_iter,
            into_iter_item: a(into_iter, "Item"),
            into_iter_into_iter: a(into_iter, "IntoIter"),
        };
        let ops = [
            (BinOp::Add, "Add"),
            (BinOp::Sub, "Sub"),
            (BinOp::Mul, "Mul"),
            (BinOp::Div, "Div"),
            (BinOp::Rem, "Rem"),
            (BinOp::BitAnd, "BitAnd"),
            (BinOp::BitOr, "BitOr"),
            (BinOp::BitXor, "BitXor"),
            (BinOp::Shl, "Shl"),
            (BinOp::Shr, "Shr"),
        ];
        let binops = ops
            .iter()
            .filter_map(|(op, n)| {
                let tr = t(n)?;
                Some((*op, tr, a(Some(tr), "Output")?))
            })
            .collect();
        let index = t("Index");
        let adt_by: HashMap<&str, DefId> = self
            .adt_names
            .iter()
            .map(|(k, v)| (v.as_str(), *k))
            .collect();
        self.check_lang = CheckLang {
            binops,
            index: index.and_then(|i| Some((i, a(Some(i), "Output")?))),
            index_mut: t("IndexMut"),
            range: adt_by.get("Range").copied(),
            range_inclusive: adt_by.get("RangeInclusive").copied(),
            string: adt_by.get("String").copied(),
            option: adt_by.get("Option").copied(),
            result: adt_by.get("Result").copied(),
        };
    }

    /// Register one nominal's variant/field table from its IR definition.
    pub fn add_adt(
        &mut self,
        decl: ir::DeclId,
        env: &ir::GenericEnv,
        variants: Vec<Vec<(String, ir::Ty)>>,
        budget: &ir::PlanBudget,
    ) {
        let map = TyMap {
            budget,
            env,
            extra: &[],
        };
        let mut u = None;
        let variants = variants
            .into_iter()
            .map(|fields| VariantData {
                fields: fields
                    .into_iter()
                    .map(|(n, t)| (n, map.ty(&t, &mut u)))
                    .collect(),
            })
            .collect();
        self.adts.insert(
            def(decl),
            AdtData {
                generics: env.total_arity() as u32,
                variants,
            },
        );
        if let Some(m) = u {
            self.unsupported.push((def(decl), m));
        }
    }

    pub fn trait_count(&self) -> usize {
        self.traits.len()
    }
    pub fn impl_count(&self) -> usize {
        self.impls.len()
    }
    pub fn fn_count(&self) -> usize {
        self.fns.len()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::compiler::frontend::{self, PlanCtx};
    use crate::compiler::ir::PlanBudget;

    fn plan_for(name: &str, src: &str) -> Result<PlanCtx, String> {
        let prepared = crate::compiler::source::prepare_text_with_prelude(
            name,
            src,
            &crate::compiler::source::SourceOptions::default(),
        )
        .map_err(|e| e.to_string())?;
        let parsed = frontend::parse_prepared(&prepared).map_err(|e| e.to_string())?;
        let program = frontend::resolve(&parsed)?;
        PlanCtx::from_program(&program, &PlanBudget::default())
    }

    #[test]
    fn world_db_maps_prelude() {
        let plan = plan_for("t.rs", "fn main() { let v: Vec<i32> = Vec::new(); }").unwrap();
        let db = WorldDb::build(&plan).unwrap();
        assert!(db.lang.fn_once.is_some(), "FnOnce lang item");
        assert!(db.check_lang.index.is_some(), "Index lang item");
        assert!(
            db.fn_count() > 0 && db.trait_count() > 0,
            "{} fns {} traits",
            db.fn_count(),
            db.trait_count()
        );
        let vec = db
            .adt_names
            .iter()
            .find(|(_, n)| n.as_str() == "Vec")
            .map(|(k, _)| *k)
            .expect("Vec");
        let inherent = db.inherent_impls(SimplifiedType::Adt(vec));
        assert!(!inherent.is_empty(), "Vec inherent impls");
        let names: Vec<&str> = inherent
            .iter()
            .flat_map(|i| db.impl_items(*i))
            .map(|(n, _)| n.as_str())
            .collect();
        assert!(
            names.contains(&"push") && names.contains(&"new"),
            "{names:?}"
        );
        let with_methods = db.traits.values().filter(|t| !t.methods.is_empty()).count();
        let trait_items = plan
            .sema_declarations()
            .iter()
            .filter(|i| i.kind == ir::DeclKind::TraitItem)
            .count();
        let with_schema = plan
            .sema_declarations()
            .iter()
            .filter(|i| i.kind == ir::DeclKind::TraitItem && plan.sema_schema(i.id).is_some())
            .count();
        println!(
            "traits with methods {with_methods}/{}; trait items {trait_items}, with schema {with_schema}",
            db.traits.len()
        );
    }

    /// Check every user (non-prelude) source callable with the semantic core.
    fn shadow(plan: &PlanCtx, db: &WorldDb) -> (usize, usize, Vec<String>) {
        let program = plan.sema_program();
        let prelude = program.prelude_module;
        let traits: Vec<DefId> = db.trait_names.keys().copied().collect();
        let (mut ok, mut total, mut notes) = (0, 0, Vec::new());
        for info in plan.sema_declarations() {
            if !matches!(
                info.kind,
                ir::DeclKind::Function | ir::DeclKind::AssociatedFunction
            ) {
                continue;
            }
            if let Some(p) = prelude
                && program
                    .modules
                    .is_descendant(info.path.module, p)
                    .unwrap_or(true)
            {
                continue;
            }
            let name = info.name.clone();
            if name.contains("$closure") {
                continue;
            }
            let lowered = match plan.sema_lower_body(info.id) {
                Ok(l) => l,
                Err(e) if e.0 == "no source body" || e.0 == "no validated schema" => continue,
                Err(e) => {
                    total += 1;
                    notes.push(format!("  {name}: lower: {}", e.0));
                    continue;
                }
            };
            total += 1;
            let env = crate::compiler::sema::solve::ParamEnv {
                predicates: plan
                    .sema_schema(info.id)
                    .map(|s| {
                        let map = TyMap {
                            budget: &plan.budget,
                            env: &s.env,
                            extra: &[],
                        };
                        let mut u = None;
                        s.predicates
                            .predicates()
                            .iter()
                            .map(|p| map.pred(p, &mut u))
                            .collect()
                    })
                    .unwrap_or_default(),
            };
            let r = crate::compiler::sema::check::check_body(
                db,
                &env,
                &traits,
                &lowered.body,
                2_000_000,
            );
            if r.errors.is_empty() {
                ok += 1;
            } else {
                let e = format!("{:?}", r.errors[0].kind);
                notes.push(format!("  {name}: check: {}", &e[..e.len().min(160)]));
            }
        }
        (ok, total, notes)
    }

    /// The same programs verified against stock at workers 1 and 8 (native
    /// build, 2026-10-21) all pass the semantic core. Wrong uses are rejected.
    #[test]
    fn primitive_differential_sources_check_and_reject() {
        for (name, src) in [
            (
                "intr_ref",
                "fn by_ref(x: &mut u64) -> u64 { x.wrapping_mul(3) } fn main(){ let mut a = u64::MAX; let _ = by_ref(&mut a); }",
            ),
            (
                "intr_val",
                "fn main(){ let n: i64 = -7; let _ = (n.abs(), n.pow(2)); }",
            ),
            (
                "slice_len",
                "fn slen(s: &[i32]) -> usize { s.len() } fn main(){ let arr = [1i32, 2, 3]; let _ = (slen(&arr), arr.len()); }",
            ),
            (
                "str_len",
                "fn main(){ let n: usize = \"h\u{e9}llo\".len(); let _ = n; }",
            ),
            (
                "float_eq",
                "fn eqv<T: PartialEq>(a: T, b: T) -> bool { a == b } fn main(){ let _ = (eqv(1.5f64, 1.5f64), eqv(1.0f32, 2.0f32)); }",
            ),
            (
                "range_for",
                "fn main(){ let mut s = 0u8; for i in 0..3u8 { s += i; } for i in 1..=2i64 { let _: i64 = i; } let _ = s; }",
            ),
        ] {
            let plan = plan_for(name, src).unwrap_or_else(|e| panic!("{name}: {e}"));
            let db = WorldDb::build(&plan).unwrap();
            let (ok, total, notes) = shadow(&plan, &db);
            assert_eq!(ok, total, "{name}: {notes:?}");
        }
        for (name, src) in [
            // abs exists only on signed integers.
            (
                "abs_unsigned",
                "fn main(){ let n: u32 = 7; let _ = n.abs(); }",
            ),
            // slice len is usize, not u8.
            (
                "len_type",
                "fn main(){ let a = [1i32]; let n: u8 = a.len(); let _ = n; }",
            ),
            // range items take the bound type.
            (
                "range_item",
                "fn main(){ for i in 0..3u8 { let _: i64 = i; } }",
            ),
        ] {
            if let Ok(plan) = plan_for(name, src) {
                let db = WorldDb::build(&plan).unwrap();
                let (ok, total, notes) = shadow(&plan, &db);
                assert!(ok < total, "{name} accepted: {notes:?}");
            }
        }
    }

    /// Inherent associated constants type through the semantic core,
    /// including a primitive owner (`i32::MIN`) and a user impl with only a
    /// constant. An unknown constant is rejected, not approximated.
    #[test]
    fn inherent_associated_constants_are_typed() {
        let good = "struct P; impl P { const K: u8 = 3; } fn main() { let a: i32 = i32::MIN; let b: u8 = P::K; let _ = (a, b); }";
        let plan = plan_for("good.rs", good).unwrap();
        let db = WorldDb::build(&plan).unwrap();
        let (ok, total, notes) = shadow(&plan, &db);
        assert_eq!((ok, total), (1, 1), "{notes:?}");

        // Wrong use type: P::K is u8.
        let bad =
            "struct P; impl P { const K: u8 = 3; } fn main() { let b: bool = P::K; let _ = b; }";
        if let Ok(plan) = plan_for("bad.rs", bad) {
            let db = WorldDb::build(&plan).unwrap();
            let (ok, total, notes) = shadow(&plan, &db);
            assert_eq!((ok, total), (0, 1), "{notes:?}");
        }
    }

    /// format!/write! operands lower into the semantic core and are still
    /// type checked: a mismatched operand stays an error.
    #[test]
    fn format_into_operands_are_checked() {
        let good = "fn main() { let n: u32 = 3; let s = format!(\"{} {}\", n, n + 1); let _ = s; }";
        let plan = plan_for("good.rs", good).unwrap();
        let db = WorldDb::build(&plan).unwrap();
        let (ok, total, notes) = shadow(&plan, &db);
        assert_eq!((ok, total), (1, 1), "{notes:?}");

        // A mismatched format operand must be rejected by planning (the old
        // checker) or by the semantic core. Never accepted by both.
        for (name, bad) in [
            (
                "bad.rs",
                "fn main() { let n: u32 = 3; let s = format!(\"{}\", n + true); let _ = s; }",
            ),
            (
                "bad2.rs",
                "fn main() { let n: u32 = 3; let s = format!(\"{}\", { let b: bool = n; b }); let _ = s; }",
            ),
        ] {
            if let Ok(plan) = plan_for(name, bad) {
                let db = WorldDb::build(&plan).unwrap();
                let (ok, total, notes) = shadow(&plan, &db);
                assert_eq!((ok, total), (0, 1), "{name}: {notes:?}");
            }
        }
    }

    #[test]
    #[ignore = "diagnostic probe: run with --ignored --nocapture"]
    fn corpus_plan_stage() {
        let dir = concat!(env!("CARGO_MANIFEST_DIR"), "/tests/fixtures/full_programs");
        let mut files: Vec<_> = std::fs::read_dir(dir)
            .unwrap()
            .filter_map(|e| e.ok().map(|e| e.path()))
            .filter(|p| p.extension().is_some_and(|x| x == "rs"))
            .collect();
        files.sort();
        for f in files {
            let src = std::fs::read_to_string(&f).unwrap();
            let name = f.file_name().unwrap().to_string_lossy().to_string();
            let stage = match plan_for(&name, &src) {
                Err(e) => format!("PLAN: {}", e.lines().next().unwrap_or("")),
                Ok(plan) => match WorldDb::build(&plan) {
                    Err(e) => format!("DB: {e}"),
                    Ok(db) => {
                        let t = std::time::Instant::now();
                        let (ok, total, notes) = shadow(&plan, &db);
                        let ms = t.elapsed().as_secs_f64() * 1e3;
                        let mut s = format!("bodies {ok}/{total} ok ({ms:.1} ms)");
                        for n in notes.iter().take(12) {
                            s.push('\n');
                            s.push_str(n);
                        }
                        s
                    }
                },
            };
            println!("{name}: {stage}");
        }
    }
}
