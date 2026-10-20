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
            ir::Ty::Opaque(_) => un("opaque type in the semantic core"),
            ir::Ty::Closure(_) => un("closure type in a declaration"),
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

        for h in headers.traits.iter() {
            let id = def(h.declaration);
            let map = TyMap {
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
            let map = TyMap { env, extra: &[] };
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
                Some(a) if info.kind == ir::DeclKind::TraitItem => {
                    Some(Owner::Trait(def(a.trait_decl.unwrap_or(a.impl_decl))))
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
            if let Some(Owner::Trait(t)) = owner {
                if let Some(td) = db.traits.get_mut(&t) {
                    td.methods.push((name.clone(), id));
                }
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
        };
    }

    /// Register one nominal's variant/field table from its IR definition.
    pub fn add_adt(
        &mut self,
        decl: ir::DeclId,
        env: &ir::GenericEnv,
        variants: Vec<Vec<(String, ir::Ty)>>,
    ) {
        let map = TyMap { env, extra: &[] };
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
                    Ok(db) => format!(
                        "db ok: {} fns {} impls {} traits, {} unsupported",
                        db.fn_count(),
                        db.impl_count(),
                        db.trait_count(),
                        db.unsupported.len()
                    ),
                },
            };
            println!("{name}: {}", &stage[..stage.len().min(200)]);
        }
    }
}
