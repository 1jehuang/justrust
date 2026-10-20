//! End-to-end tests of the semantic core against a hand-built mini std.
//! Each test is a body that the current frontend rejects (see
//! SEMANTIC-CORE.md probes) and that stock rustc accepts, or a program
//! stock rejects that must stay rejected.

use super::check::*;
use super::hir::*;
use super::solve::*;
use super::ty::*;
use std::collections::HashMap;

// ---- mini std ids
const SIZED: DefId = 1;
const COPY: DefId = 2;
const CLONE: DefId = 3;
const FN: DefId = 4;
const FN_MUT: DefId = 5;
const FN_ONCE: DefId = 6;
const FN_ONCE_OUTPUT: DefId = 7;
const ITERATOR: DefId = 8;
const ITEM: DefId = 9;
const INTO_ITER: DefId = 10;
const II_ITEM: DefId = 11;
const II_INTOITER: DefId = 12;
const ADD: DefId = 13;
const ADD_OUT: DefId = 14;
const INDEX: DefId = 15;
const INDEX_OUT: DefId = 16;
const DEREF: DefId = 17;
const DEREF_TARGET: DefId = 18;
const FROM_ITER: DefId = 19;

const VEC: DefId = 100;
const OPTION: DefId = 101;
const MAP: DefId = 102;
const ITER: DefId = 103; // slice::Iter<'_, T> without lifetime
const ENUMERATE: DefId = 104;
const RANGE: DefId = 105;
const RANGE_INCL: DefId = 106;
const WRAP: DefId = 107;

// impls
const I_VEC_INHERENT: DefId = 200;
const I_SLICE_INHERENT: DefId = 201;
const I_ITER_ITERATOR: DefId = 202;
const I_MAP_ITERATOR: DefId = 203;
const I_ENUM_ITERATOR: DefId = 204;
const I_II_FOR_ITERATOR: DefId = 205;
const I_II_REF_VEC: DefId = 206;
const I_VEC_INDEX: DefId = 207;
const I_VEC_DEREF: DefId = 208;
const I_VEC_FROMITER: DefId = 209;
const I_RANGE_ITERATOR: DefId = 210;
const I_OPTION_INHERENT: DefId = 211;
const I_VEC_CLONE: DefId = 212;
const I_WRAP_INHERENT: DefId = 213;
const I_II_VEC: DefId = 214;

// fns
const VEC_NEW: DefId = 300;
const VEC_PUSH: DefId = 301;
const VEC_LEN: DefId = 302;
const SLICE_ITER: DefId = 303;
const ITERATOR_NEXT: DefId = 304;
const ITERATOR_MAP: DefId = 305;
const ITERATOR_ENUMERATE: DefId = 306;
const ITERATOR_COLLECT: DefId = 307;
const II_INTO_ITER: DefId = 308;
const OPTION_UNWRAP_OR: DefId = 309;
const CLONE_CLONE: DefId = 310;
const WRAP_NEW: DefId = 311;
const WRAP_ADD: DefId = 312;
const WRAP_FIRST: DefId = 313;
const ITERATOR_SUM: DefId = 314;
const SLICE_LEN: DefId = 315;

#[derive(Default)]
struct Db {
    traits: HashMap<DefId, TraitData>,
    impls: HashMap<DefId, ImplData>,
    impls_of: HashMap<DefId, Vec<DefId>>,
    inherent: HashMap<SimplifiedType, Vec<DefId>>,
    impl_items: HashMap<DefId, Vec<(String, DefId)>>,
    fns: HashMap<DefId, FnSig>,
    adts: HashMap<DefId, AdtData>,
    lang: LangItems,
    check_lang: CheckLang,
}

impl TraitDb for Db {
    fn trait_data(&self, t: DefId) -> &TraitData {
        &self.traits[&t]
    }
    fn impls_of(&self, t: DefId) -> &[DefId] {
        self.impls_of.get(&t).map(|v| v.as_slice()).unwrap_or(&[])
    }
    fn impl_data(&self, i: DefId) -> &ImplData {
        &self.impls[&i]
    }
    fn inherent_impls(&self, k: SimplifiedType) -> &[DefId] {
        self.inherent.get(&k).map(|v| v.as_slice()).unwrap_or(&[])
    }
    fn lang(&self) -> &LangItems {
        &self.lang
    }
}
impl ItemDb for Db {
    fn fn_sig(&self, f: DefId) -> &FnSig {
        &self.fns[&f]
    }
    fn adt(&self, a: DefId) -> &AdtData {
        &self.adts[&a]
    }
    fn impl_items(&self, i: DefId) -> &[(String, DefId)] {
        self.impl_items.get(&i).map(|v| v.as_slice()).unwrap_or(&[])
    }
    fn check_lang(&self) -> &CheckLang {
        &self.check_lang
    }
    fn as_trait_db(&self) -> &dyn TraitDb {
        self
    }
}

fn p(i: u32) -> Ty {
    param(i)
}
fn tref(t: DefId, s: Ty, args: Vec<Ty>) -> TraitRef {
    TraitRef {
        trait_id: t,
        self_ty: s,
        args,
    }
}
fn proj(t: DefId, s: Ty, args: Vec<Ty>, item: DefId) -> Ty {
    mk(TyKind::Projection(std::sync::Arc::new(ProjectionTy {
        trait_ref: tref(t, s, args),
        item,
    })))
}
fn pproj(t: DefId, s: Ty, args: Vec<Ty>, item: DefId, rhs: Ty) -> Predicate {
    Predicate::Projection(
        ProjectionTy {
            trait_ref: tref(t, s, args),
            item,
        },
        rhs,
    )
}
fn ptrait(t: DefId, s: Ty, args: Vec<Ty>) -> Predicate {
    Predicate::Trait(tref(t, s, args))
}
fn shr(t: Ty) -> Ty {
    reference(Mutbl::Not, t)
}
fn mutr(t: Ty) -> Ty {
    reference(Mutbl::Mut, t)
}
fn usize_t() -> Ty {
    int(IntTy::Usize)
}
fn tuple(v: Vec<Ty>) -> Ty {
    mk(TyKind::Tuple(v))
}
fn opt(t: Ty) -> Ty {
    adt(OPTION, vec![t])
}

impl Db {
    fn tr(&mut self, id: DefId, generics: u32, assoc: Vec<DefId>, methods: Vec<(&str, DefId)>) {
        self.traits.insert(
            id,
            TraitData {
                id,
                generics,
                supertraits: Vec::new(),
                assoc_types: assoc,
                methods: methods
                    .into_iter()
                    .map(|(n, d)| (n.to_string(), d))
                    .collect(),
            },
        );
    }
    fn imp(
        &mut self,
        id: DefId,
        generics: u32,
        tr: Option<TraitRef>,
        self_ty: Ty,
        preds: Vec<Predicate>,
        assoc: Vec<(DefId, Ty)>,
        items: Vec<(&str, DefId)>,
    ) {
        if let Some(t) = &tr {
            self.impls_of.entry(t.trait_id).or_default().push(id);
        } else {
            self.inherent
                .entry(simplify(&self_ty).unwrap())
                .or_default()
                .push(id);
        }
        self.impls.insert(
            id,
            ImplData {
                id,
                generics,
                trait_ref: tr,
                self_ty,
                predicates: preds,
                assoc_types: assoc,
            },
        );
        self.impl_items.insert(
            id,
            items.into_iter().map(|(n, d)| (n.to_string(), d)).collect(),
        );
    }
    #[allow(clippy::too_many_arguments)]
    fn f(
        &mut self,
        id: DefId,
        generics: u32,
        parent: u32,
        owner: Option<Owner>,
        has_self: bool,
        inputs: Vec<Ty>,
        output: Ty,
        preds: Vec<Predicate>,
    ) {
        self.fns.insert(
            id,
            FnSig {
                generics,
                parent_generics: parent,
                inputs,
                output,
                predicates: preds,
                owner,
                has_self,
            },
        );
    }

    fn mini_std() -> Self {
        let mut d = Db::default();
        d.lang = LangItems {
            sized: Some(SIZED),
            copy: Some(COPY),
            clone: Some(CLONE),
            fn_: Some(FN),
            fn_mut: Some(FN_MUT),
            fn_once: Some(FN_ONCE),
            fn_once_output: Some(FN_ONCE_OUTPUT),
            deref: Some(DEREF),
            deref_target: Some(DEREF_TARGET),
            iterator: Some(ITERATOR),
            iterator_item: Some(ITEM),
            into_iterator: Some(INTO_ITER),
            into_iter_item: Some(II_ITEM),
            into_iter_into_iter: Some(II_INTOITER),
        };
        d.check_lang = CheckLang {
            binops: vec![(BinOp::Add, ADD, ADD_OUT)],
            index: Some((INDEX, INDEX_OUT)),
            index_mut: None,
            range: Some(RANGE),
            range_inclusive: Some(RANGE_INCL),
            string: None,
        };
        for t in [SIZED, COPY] {
            d.tr(t, 0, vec![], vec![]);
        }
        d.tr(CLONE, 0, vec![], vec![("clone", CLONE_CLONE)]);
        d.f(
            CLONE_CLONE,
            1,
            1,
            Some(Owner::Trait(CLONE)),
            true,
            vec![shr(p(0))],
            p(0),
            vec![],
        );
        for t in [FN, FN_MUT] {
            d.tr(t, 1, vec![], vec![]);
        }
        d.tr(FN_ONCE, 1, vec![FN_ONCE_OUTPUT], vec![]);
        d.tr(ADD, 1, vec![ADD_OUT], vec![]);
        d.tr(INDEX, 1, vec![INDEX_OUT], vec![]);
        d.tr(DEREF, 0, vec![DEREF_TARGET], vec![]);
        d.tr(FROM_ITER, 1, vec![], vec![]);
        // trait Iterator { type Item; fn next(&mut self)->Option<Item>; fn map; fn enumerate; fn collect; fn sum }
        d.tr(
            ITERATOR,
            0,
            vec![ITEM],
            vec![
                ("next", ITERATOR_NEXT),
                ("map", ITERATOR_MAP),
                ("enumerate", ITERATOR_ENUMERATE),
                ("collect", ITERATOR_COLLECT),
                ("sum", ITERATOR_SUM),
            ],
        );
        let self_item = proj(ITERATOR, p(0), vec![], ITEM);
        d.f(
            ITERATOR_NEXT,
            1,
            1,
            Some(Owner::Trait(ITERATOR)),
            true,
            vec![mutr(p(0))],
            opt(self_item.clone()),
            vec![],
        );
        // fn map<B, F: FnMut(Self::Item) -> B>(self, f: F) -> Map<Self, F>   params: Self=0, B=1, F=2
        d.f(
            ITERATOR_MAP,
            3,
            1,
            Some(Owner::Trait(ITERATOR)),
            true,
            vec![p(0), p(2)],
            adt(MAP, vec![p(0), p(2)]),
            vec![
                ptrait(FN_MUT, p(2), vec![tuple(vec![self_item.clone()])]),
                ptrait(FN_ONCE, p(2), vec![tuple(vec![self_item.clone()])]),
                pproj(
                    FN_ONCE,
                    p(2),
                    vec![tuple(vec![self_item.clone()])],
                    FN_ONCE_OUTPUT,
                    p(1),
                ),
            ],
        );
        d.f(
            ITERATOR_ENUMERATE,
            1,
            1,
            Some(Owner::Trait(ITERATOR)),
            true,
            vec![p(0)],
            adt(ENUMERATE, vec![p(0)]),
            vec![],
        );
        // fn collect<B: FromIterator<Self::Item>>(self) -> B
        d.f(
            ITERATOR_COLLECT,
            2,
            1,
            Some(Owner::Trait(ITERATOR)),
            true,
            vec![p(0)],
            p(1),
            vec![ptrait(FROM_ITER, p(1), vec![self_item.clone()])],
        );
        // fn sum(self) -> Self::Item (simplified: no Sum trait)
        d.f(
            ITERATOR_SUM,
            1,
            1,
            Some(Owner::Trait(ITERATOR)),
            true,
            vec![p(0)],
            self_item.clone(),
            vec![],
        );
        // trait IntoIterator { type Item; type IntoIter; fn into_iter(self) -> IntoIter }
        d.tr(
            INTO_ITER,
            0,
            vec![II_ITEM, II_INTOITER],
            vec![("into_iter", II_INTO_ITER)],
        );
        d.f(
            II_INTO_ITER,
            1,
            1,
            Some(Owner::Trait(INTO_ITER)),
            true,
            vec![p(0)],
            proj(INTO_ITER, p(0), vec![], II_INTOITER),
            vec![],
        );

        d.adts.insert(
            VEC,
            AdtData {
                generics: 1,
                variants: vec![VariantData { fields: vec![] }],
            },
        );
        d.adts.insert(
            OPTION,
            AdtData {
                generics: 1,
                variants: vec![
                    VariantData { fields: vec![] },
                    VariantData {
                        fields: vec![("0".into(), p(0))],
                    },
                ],
            },
        );
        d.adts.insert(
            MAP,
            AdtData {
                generics: 2,
                variants: vec![VariantData { fields: vec![] }],
            },
        );
        d.adts.insert(
            ITER,
            AdtData {
                generics: 1,
                variants: vec![VariantData { fields: vec![] }],
            },
        );
        d.adts.insert(
            ENUMERATE,
            AdtData {
                generics: 1,
                variants: vec![VariantData { fields: vec![] }],
            },
        );
        d.adts.insert(
            RANGE,
            AdtData {
                generics: 1,
                variants: vec![VariantData {
                    fields: vec![("start".into(), p(0)), ("end".into(), p(0))],
                }],
            },
        );
        d.adts.insert(
            RANGE_INCL,
            AdtData {
                generics: 1,
                variants: vec![VariantData { fields: vec![] }],
            },
        );
        d.adts.insert(
            WRAP,
            AdtData {
                generics: 1,
                variants: vec![VariantData {
                    fields: vec![("items".into(), adt(VEC, vec![p(0)]))],
                }],
            },
        );

        // impl<T> Vec<T> { new, push, len }
        d.imp(
            I_VEC_INHERENT,
            1,
            None,
            adt(VEC, vec![p(0)]),
            vec![],
            vec![],
            vec![("new", VEC_NEW), ("push", VEC_PUSH), ("len", VEC_LEN)],
        );
        d.f(
            VEC_NEW,
            1,
            1,
            Some(Owner::Impl(I_VEC_INHERENT)),
            false,
            vec![],
            adt(VEC, vec![p(0)]),
            vec![],
        );
        d.f(
            VEC_PUSH,
            1,
            1,
            Some(Owner::Impl(I_VEC_INHERENT)),
            true,
            vec![mutr(adt(VEC, vec![p(0)])), p(0)],
            unit(),
            vec![],
        );
        d.f(
            VEC_LEN,
            1,
            1,
            Some(Owner::Impl(I_VEC_INHERENT)),
            true,
            vec![shr(adt(VEC, vec![p(0)]))],
            usize_t(),
            vec![],
        );
        // impl<T> [T] { iter, len }
        let slice = mk(TyKind::Slice(p(0)));
        d.imp(
            I_SLICE_INHERENT,
            1,
            None,
            slice.clone(),
            vec![],
            vec![],
            vec![("iter", SLICE_ITER), ("len", SLICE_LEN)],
        );
        d.f(
            SLICE_ITER,
            1,
            1,
            Some(Owner::Impl(I_SLICE_INHERENT)),
            true,
            vec![shr(slice.clone())],
            adt(ITER, vec![p(0)]),
            vec![],
        );
        d.f(
            SLICE_LEN,
            1,
            1,
            Some(Owner::Impl(I_SLICE_INHERENT)),
            true,
            vec![shr(slice.clone())],
            usize_t(),
            vec![],
        );
        // impl<T> Deref for Vec<T> { type Target = [T] }
        d.imp(
            I_VEC_DEREF,
            1,
            Some(tref(DEREF, adt(VEC, vec![p(0)]), vec![])),
            adt(VEC, vec![p(0)]),
            vec![],
            vec![(DEREF_TARGET, slice.clone())],
            vec![],
        );
        // impl<T> Index<usize> for Vec<T> { type Output = T }
        d.imp(
            I_VEC_INDEX,
            1,
            Some(tref(INDEX, adt(VEC, vec![p(0)]), vec![usize_t()])),
            adt(VEC, vec![p(0)]),
            vec![],
            vec![(INDEX_OUT, p(0))],
            vec![],
        );
        // impl<T> FromIterator<T> for Vec<T>
        d.imp(
            I_VEC_FROMITER,
            1,
            Some(tref(FROM_ITER, adt(VEC, vec![p(0)]), vec![p(0)])),
            adt(VEC, vec![p(0)]),
            vec![],
            vec![],
            vec![],
        );
        // impl<T: Clone> Clone for Vec<T>
        d.imp(
            I_VEC_CLONE,
            1,
            Some(tref(CLONE, adt(VEC, vec![p(0)]), vec![])),
            adt(VEC, vec![p(0)]),
            vec![ptrait(CLONE, p(0), vec![])],
            vec![],
            vec![],
        );
        // impl<T> Iterator for Iter<T> { type Item = &T }
        d.imp(
            I_ITER_ITERATOR,
            1,
            Some(tref(ITERATOR, adt(ITER, vec![p(0)]), vec![])),
            adt(ITER, vec![p(0)]),
            vec![],
            vec![(ITEM, shr(p(0)))],
            vec![],
        );
        // impl<B, I: Iterator, F: FnMut(I::Item) -> B> Iterator for Map<I, F> { type Item = B }  B=0, I=1, F=2
        let i_item = proj(ITERATOR, p(1), vec![], ITEM);
        d.imp(
            I_MAP_ITERATOR,
            3,
            Some(tref(ITERATOR, adt(MAP, vec![p(1), p(2)]), vec![])),
            adt(MAP, vec![p(1), p(2)]),
            vec![
                ptrait(ITERATOR, p(1), vec![]),
                ptrait(FN_MUT, p(2), vec![tuple(vec![i_item.clone()])]),
                pproj(
                    FN_ONCE,
                    p(2),
                    vec![tuple(vec![i_item.clone()])],
                    FN_ONCE_OUTPUT,
                    p(0),
                ),
            ],
            vec![(ITEM, p(0))],
            vec![],
        );
        // impl<I: Iterator> Iterator for Enumerate<I> { type Item = (usize, I::Item) }
        d.imp(
            I_ENUM_ITERATOR,
            1,
            Some(tref(ITERATOR, adt(ENUMERATE, vec![p(0)]), vec![])),
            adt(ENUMERATE, vec![p(0)]),
            vec![ptrait(ITERATOR, p(0), vec![])],
            vec![(
                ITEM,
                tuple(vec![usize_t(), proj(ITERATOR, p(0), vec![], ITEM)]),
            )],
            vec![],
        );
        // impl<A> Iterator for Range<A> { type Item = A }  (simplified bound)
        d.imp(
            I_RANGE_ITERATOR,
            1,
            Some(tref(ITERATOR, adt(RANGE, vec![p(0)]), vec![])),
            adt(RANGE, vec![p(0)]),
            vec![],
            vec![(ITEM, p(0))],
            vec![],
        );
        // impl<I: Iterator> IntoIterator for I { Item = I::Item, IntoIter = I }
        d.imp(
            I_II_FOR_ITERATOR,
            1,
            Some(tref(INTO_ITER, p(0), vec![])),
            p(0),
            vec![ptrait(ITERATOR, p(0), vec![])],
            vec![
                (II_ITEM, proj(ITERATOR, p(0), vec![], ITEM)),
                (II_INTOITER, p(0)),
            ],
            vec![],
        );
        // impl<'a, T> IntoIterator for &'a Vec<T> { Item = &T, IntoIter = Iter<T> }
        d.imp(
            I_II_REF_VEC,
            1,
            Some(tref(INTO_ITER, shr(adt(VEC, vec![p(0)])), vec![])),
            shr(adt(VEC, vec![p(0)])),
            vec![],
            vec![(II_ITEM, shr(p(0))), (II_INTOITER, adt(ITER, vec![p(0)]))],
            vec![],
        );
        // impl<T> IntoIterator for Vec<T> { Item = T, IntoIter = Iter<T> } (iter type simplified)
        d.imp(
            I_II_VEC,
            1,
            Some(tref(INTO_ITER, adt(VEC, vec![p(0)]), vec![])),
            adt(VEC, vec![p(0)]),
            vec![],
            vec![(II_ITEM, p(0)), (II_INTOITER, adt(ITER, vec![p(0)]))],
            vec![],
        );
        // impl<T> Option<T> { fn unwrap_or(self, d: T) -> T }
        d.imp(
            I_OPTION_INHERENT,
            1,
            None,
            opt(p(0)),
            vec![],
            vec![],
            vec![("unwrap_or", OPTION_UNWRAP_OR)],
        );
        d.f(
            OPTION_UNWRAP_OR,
            1,
            1,
            Some(Owner::Impl(I_OPTION_INHERENT)),
            true,
            vec![opt(p(0)), p(0)],
            p(0),
            vec![],
        );
        // struct Wrap<T> { items: Vec<T> }  impl<T: Clone> Wrap<T> { new, add, first }
        d.imp(
            I_WRAP_INHERENT,
            1,
            None,
            adt(WRAP, vec![p(0)]),
            vec![ptrait(CLONE, p(0), vec![])],
            vec![],
            vec![("new", WRAP_NEW), ("add", WRAP_ADD), ("first", WRAP_FIRST)],
        );
        d.f(
            WRAP_NEW,
            1,
            1,
            Some(Owner::Impl(I_WRAP_INHERENT)),
            false,
            vec![],
            adt(WRAP, vec![p(0)]),
            vec![],
        );
        d.f(
            WRAP_ADD,
            1,
            1,
            Some(Owner::Impl(I_WRAP_INHERENT)),
            true,
            vec![mutr(adt(WRAP, vec![p(0)])), p(0)],
            unit(),
            vec![],
        );
        d.f(
            WRAP_FIRST,
            1,
            1,
            Some(Owner::Impl(I_WRAP_INHERENT)),
            true,
            vec![shr(adt(WRAP, vec![p(0)]))],
            opt(p(0)),
            vec![],
        );
        d
    }
}

const SCOPE: &[DefId] = &[ITERATOR, INTO_ITER, CLONE];

/// Tiny body builder.
#[derive(Default)]
struct B {
    exprs: Vec<HirExpr>,
    locals: u32,
}
impl B {
    fn e(&mut self, x: HirExpr) -> ExprId {
        self.exprs.push(x);
        (self.exprs.len() - 1) as ExprId
    }
    fn local(&mut self) -> LocalId {
        self.locals += 1;
        self.locals - 1
    }
    fn int(&mut self, v: u128) -> ExprId {
        self.e(HirExpr::Lit(HirLit::Int(v, None)))
    }
    fn var(&mut self, l: LocalId) -> ExprId {
        self.e(HirExpr::Local(l))
    }
    fn mcall(&mut self, r: ExprId, name: &str, args: Vec<ExprId>) -> ExprId {
        self.e(HirExpr::MethodCall {
            receiver: r,
            name: name.into(),
            turbofish: vec![],
            args,
        })
    }
    fn bind(l: LocalId) -> HirPat {
        HirPat::Binding {
            local: l,
            by_ref: None,
        }
    }
    fn body(self, stmts: Vec<HirStmt>, tail: Option<ExprId>, ret: Ty) -> HirBody {
        let mut b = self;
        let root = b.e(HirExpr::Block { stmts, tail });
        HirBody {
            exprs: b.exprs,
            locals: b.locals,
            params: vec![],
            ret,
            root,
        }
    }
}

fn run(body: &HirBody) -> Typeck {
    let db = Db::mini_std();
    let env = ParamEnv::default();
    check_body(&db, &env, SCOPE, body, 1_000_000)
}

fn ok(body: &HirBody) -> Typeck {
    let r = run(body);
    assert!(r.errors.is_empty(), "unexpected errors: {:?}", r.errors);
    r
}

/// `let mut v = Vec::new(); v.push(3usize); v.len()` : Vec<usize>
#[test]
fn vec_new_inferred_from_later_push() {
    let mut b = B::default();
    let v = b.local();
    let new = b.e(HirExpr::AssocFn {
        self_ty: adt(VEC, vec![hole()]),
        name: "new".into(),
    });
    let call = b.e(HirExpr::Call {
        callee: new,
        args: vec![],
    });
    let vr = b.var(v);
    let three = b.e(HirExpr::Lit(HirLit::Int(3, Some(IntTy::Usize))));
    let push = b.mcall(vr, "push", vec![three]);
    let vr2 = b.var(v);
    let len = b.mcall(vr2, "len", vec![]);
    let body = b.body(
        vec![
            HirStmt::Let {
                pat: B::bind(v),
                ty: hole(),
                init: Some(call),
                else_block: None,
            },
            HirStmt::Expr(push),
        ],
        Some(len),
        usize_t(),
    );
    let r = ok(&body);
    assert_eq!(r.local_tys[v as usize], adt(VEC, vec![usize_t()]));
}

/// `let mut sum = 0; for k in 0..3 { sum += a[k] * 2 }` with `a: [i64; 3]`.
/// The literal 0 becomes i64 from a later use, and `k` becomes usize from indexing.
#[test]
fn literal_type_flows_backwards_from_later_use() {
    let mut b = B::default();
    let a = b.local();
    let sum = b.local();
    let k = b.local();
    let one = b.int(1);
    let two_ = b.int(2);
    let three_ = b.int(3);
    let arr = b.e(HirExpr::Array(vec![one, two_, three_]));
    let zero = b.int(0);
    let lo = b.int(0);
    let hi = b.int(3);
    let range = b.e(HirExpr::Range {
        start: Some(lo),
        end: Some(hi),
        inclusive: false,
    });
    let ar = b.var(a);
    let kr = b.var(k);
    let idx = b.e(HirExpr::Index {
        base: ar,
        index: kr,
    });
    let two = b.int(2);
    let mul = b.e(HirExpr::Binary(BinOp::Mul, idx, two));
    let sr = b.var(sum);
    let add = b.e(HirExpr::AssignOp(BinOp::Add, sr, mul));
    let loop_body = b.e(HirExpr::Block {
        stmts: vec![HirStmt::Expr(add)],
        tail: None,
    });
    let f = b.e(HirExpr::For {
        pat: B::bind(k),
        iter: range,
        body: loop_body,
        label: 0,
    });
    let body = b.body(
        vec![
            HirStmt::Let {
                pat: B::bind(a),
                ty: mk(TyKind::Array(int(IntTy::I64), 3)),
                init: Some(arr),
                else_block: None,
            },
            HirStmt::Let {
                pat: B::bind(sum),
                ty: hole(),
                init: Some(zero),
                else_block: None,
            },
            HirStmt::Expr(f),
        ],
        None,
        unit(),
    );
    let r = ok(&body);
    assert_eq!(r.local_tys[sum as usize], int(IntTy::I64));
    assert_eq!(r.local_tys[k as usize], usize_t());
}

/// `let w: Vec<i32> = v.iter().map(|x| x + 1).collect();`
/// Closure parameter `x: &i32` is deduced from the pending FnMut bound.
#[test]
fn closure_param_inferred_through_map_and_collect() {
    let mut b = B::default();
    let v = b.local();
    let w = b.local();
    let x = b.local();
    let vr = b.var(v);
    let it = b.mcall(vr, "iter", vec![]);
    let xr = b.var(x);
    let one = b.int(1);
    // `x + 1` with x: &i32 -> scalar add through the reference.
    let add = b.e(HirExpr::Binary(BinOp::Add, xr, one));
    let clo = b.e(HirExpr::Closure {
        params: vec![(B::bind(x), hole())],
        ret: hole(),
        body: add,
        id: 0,
    });
    let map = b.mcall(it, "map", vec![clo]);
    let col = b.mcall(map, "collect", vec![]);
    let mut body = b.body(
        vec![HirStmt::Let {
            pat: B::bind(w),
            ty: adt(VEC, vec![int(IntTy::I32)]),
            init: Some(col),
            else_block: None,
        }],
        None,
        unit(),
    );
    body.params = vec![(B::bind(v), adt(VEC, vec![int(IntTy::I32)]))];
    let r = ok(&body);
    assert_eq!(r.local_tys[x as usize], shr(int(IntTy::I32)));
}

/// `for (n, &p) in v.iter().enumerate() { if p { ... } }` with v: Vec<bool>
#[test]
fn enumerate_pattern_destructures_projection_item() {
    let mut b = B::default();
    let v = b.local();
    let n = b.local();
    let pl = b.local();
    let vr = b.var(v);
    let it = b.mcall(vr, "iter", vec![]);
    let en = b.mcall(it, "enumerate", vec![]);
    let pr = b.var(pl);
    let then = b.e(HirExpr::Block {
        stmts: vec![],
        tail: None,
    });
    let iff = b.e(HirExpr::If {
        cond: pr,
        then,
        els: None,
    });
    let lb = b.e(HirExpr::Block {
        stmts: vec![HirStmt::Expr(iff)],
        tail: None,
    });
    let pat = HirPat::Tuple(vec![
        B::bind(n),
        HirPat::Ref(Mutbl::Not, Box::new(B::bind(pl))),
    ]);
    let f = b.e(HirExpr::For {
        pat,
        iter: en,
        body: lb,
        label: 0,
    });
    let mut body = b.body(vec![HirStmt::Expr(f)], None, unit());
    body.params = vec![(B::bind(v), adt(VEC, vec![bool_ty()]))];
    let r = ok(&body);
    assert_eq!(r.local_tys[n as usize], usize_t());
    assert_eq!(r.local_tys[pl as usize], bool_ty());
}

/// `let mut v = vec_of(3, 1, 2); let mut i = 0; while i < v.len() { v[i] += 1; i += 1 }`
/// `i` becomes usize from `v.len()`, the element defaults to i32.
#[test]
fn index_through_trait_and_compare_with_len() {
    let mut b = B::default();
    let v = b.local();
    let i = b.local();
    let new = b.e(HirExpr::AssocFn {
        self_ty: adt(VEC, vec![hole()]),
        name: "new".into(),
    });
    let call = b.e(HirExpr::Call {
        callee: new,
        args: vec![],
    });
    let vr = b.var(v);
    let three = b.int(3);
    let push = b.mcall(vr, "push", vec![three]);
    let zero = b.int(0);
    let ir = b.var(i);
    let vr2 = b.var(v);
    let len = b.mcall(vr2, "len", vec![]);
    let cond = b.e(HirExpr::Binary(BinOp::Lt, ir, len));
    let vr3 = b.var(v);
    let ir2 = b.var(i);
    let idx = b.e(HirExpr::Index {
        base: vr3,
        index: ir2,
    });
    let one = b.int(1);
    let inc_v = b.e(HirExpr::AssignOp(BinOp::Add, idx, one));
    let ir3 = b.var(i);
    let one2 = b.int(1);
    let inc_i = b.e(HirExpr::AssignOp(BinOp::Add, ir3, one2));
    let wb = b.e(HirExpr::Block {
        stmts: vec![HirStmt::Expr(inc_v), HirStmt::Expr(inc_i)],
        tail: None,
    });
    let wh = b.e(HirExpr::While {
        cond,
        body: wb,
        label: 0,
    });
    let body = b.body(
        vec![
            HirStmt::Let {
                pat: B::bind(v),
                ty: hole(),
                init: Some(call),
                else_block: None,
            },
            HirStmt::Expr(push),
            HirStmt::Let {
                pat: B::bind(i),
                ty: hole(),
                init: Some(zero),
                else_block: None,
            },
            HirStmt::Expr(wh),
        ],
        None,
        unit(),
    );
    let r = ok(&body);
    assert_eq!(r.local_tys[i as usize], usize_t());
    assert_eq!(r.local_tys[v as usize], adt(VEC, vec![int(IntTy::I32)]));
}

/// `let mut w = Wrap::new(); w.add(5); let x = w.first().unwrap_or(0);`
/// Generic struct inferred from a later method call, through an impl bound.
#[test]
fn generic_struct_inferred_from_later_method() {
    let mut b = B::default();
    let w = b.local();
    let x = b.local();
    let new = b.e(HirExpr::AssocFn {
        self_ty: adt(WRAP, vec![hole()]),
        name: "new".into(),
    });
    let call = b.e(HirExpr::Call {
        callee: new,
        args: vec![],
    });
    let wr = b.var(w);
    let five = b.int(5);
    let add = b.mcall(wr, "add", vec![five]);
    let wr2 = b.var(w);
    let first = b.mcall(wr2, "first", vec![]);
    let zero = b.int(0);
    let un = b.mcall(first, "unwrap_or", vec![zero]);
    let body = b.body(
        vec![
            HirStmt::Let {
                pat: B::bind(w),
                ty: hole(),
                init: Some(call),
                else_block: None,
            },
            HirStmt::Expr(add),
            HirStmt::Let {
                pat: B::bind(x),
                ty: hole(),
                init: Some(un),
                else_block: None,
            },
        ],
        None,
        unit(),
    );
    let r = ok(&body);
    assert_eq!(r.local_tys[x as usize], int(IntTy::I32));
}

/// Negative: `let v = Vec::new();` with no further use is E0282.
#[test]
fn unconstrained_vec_is_annotations_needed() {
    let mut b = B::default();
    let v = b.local();
    let new = b.e(HirExpr::AssocFn {
        self_ty: adt(VEC, vec![hole()]),
        name: "new".into(),
    });
    let call = b.e(HirExpr::Call {
        callee: new,
        args: vec![],
    });
    let body = b.body(
        vec![HirStmt::Let {
            pat: B::bind(v),
            ty: hole(),
            init: Some(call),
            else_block: None,
        }],
        None,
        unit(),
    );
    let r = run(&body);
    assert!(
        r.errors
            .iter()
            .any(|e| e.kind == CheckErrorKind::AnnotationsNeeded),
        "{:?}",
        r.errors
    );
}

/// Negative: `let x: bool = 1;` is a mismatch.
#[test]
fn integer_literal_into_bool_is_mismatch() {
    let mut b = B::default();
    let x = b.local();
    let one = b.int(1);
    let body = b.body(
        vec![HirStmt::Let {
            pat: B::bind(x),
            ty: bool_ty(),
            init: Some(one),
            else_block: None,
        }],
        None,
        unit(),
    );
    let r = run(&body);
    assert!(
        matches!(
            r.errors.first().map(|e| &e.kind),
            Some(CheckErrorKind::Mismatch { .. })
        ),
        "{:?}",
        r.errors
    );
}

/// Negative: `Wrap<T: Clone>::add` with a non-Clone element is rejected
/// by the impl predicate even though nothing else uses it.
#[test]
fn unsatisfied_impl_bound_is_error() {
    let mut b = B::default();
    let w = b.local();
    let new = b.e(HirExpr::AssocFn {
        self_ty: adt(WRAP, vec![hole()]),
        name: "new".into(),
    });
    let call = b.e(HirExpr::Call {
        callee: new,
        args: vec![],
    });
    let wr = b.var(w);
    // Map<..> is not Clone in the mini std: use an Iter<i32> value via a param.
    let pv = b.local();
    let pr = b.var(pv);
    let add = b.mcall(wr, "add", vec![pr]);
    let mut body = b.body(
        vec![
            HirStmt::Let {
                pat: B::bind(w),
                ty: hole(),
                init: Some(call),
                else_block: None,
            },
            HirStmt::Expr(add),
        ],
        None,
        unit(),
    );
    body.params = vec![(B::bind(pv), adt(ITER, vec![int(IntTy::I32)]))];
    let r = run(&body);
    assert!(
        r.errors
            .iter()
            .any(|e| matches!(e.kind, CheckErrorKind::Unsatisfied(_))),
        "{:?}",
        r.errors
    );
}

/// Negative: closure body type contradicts the collected element type.
#[test]
fn closure_output_mismatch_is_rejected() {
    let mut b = B::default();
    let v = b.local();
    let w = b.local();
    let x = b.local();
    let vr = b.var(v);
    let it = b.mcall(vr, "iter", vec![]);
    let t = b.e(HirExpr::Lit(HirLit::Bool));
    let clo = b.e(HirExpr::Closure {
        params: vec![(B::bind(x), hole())],
        ret: hole(),
        body: t,
        id: 0,
    });
    let map = b.mcall(it, "map", vec![clo]);
    let col = b.mcall(map, "collect", vec![]);
    let mut body = b.body(
        vec![HirStmt::Let {
            pat: B::bind(w),
            ty: adt(VEC, vec![int(IntTy::I32)]),
            init: Some(col),
            else_block: None,
        }],
        None,
        unit(),
    );
    body.params = vec![(B::bind(v), adt(VEC, vec![int(IntTy::I32)]))];
    let r = run(&body);
    assert!(!r.errors.is_empty());
}

/// Work stays linear in body size: doubling a chain of statements roughly
/// doubles metered work (graph-integrity spirit, per-node scaling).
#[test]
fn work_scales_linearly_with_body() {
    fn chain(n: usize) -> u64 {
        let mut b = B::default();
        let v = b.local();
        let new = b.e(HirExpr::AssocFn {
            self_ty: adt(VEC, vec![hole()]),
            name: "new".into(),
        });
        let call = b.e(HirExpr::Call {
            callee: new,
            args: vec![],
        });
        let mut stmts = vec![HirStmt::Let {
            pat: B::bind(v),
            ty: hole(),
            init: Some(call),
            else_block: None,
        }];
        for _ in 0..n {
            let vr = b.var(v);
            let one = b.int(1);
            stmts.push(HirStmt::Expr(b.mcall(vr, "push", vec![one])));
        }
        let body = b.body(stmts, None, unit());
        let db = Db::mini_std();
        let env = ParamEnv::default();
        let mut used = 0;
        // Re-run to measure: check_body does not expose the meter, so use
        // the smallest limit that succeeds.
        let mut lo = 1u64;
        let mut hi = 10_000_000u64;
        while lo < hi {
            let mid = (lo + hi) / 2;
            let r = check_body(&db, &env, SCOPE, &body, mid);
            if r.errors.iter().any(|e| e.kind == CheckErrorKind::Limit) {
                lo = mid + 1;
            } else {
                hi = mid;
                used = mid;
            }
        }
        used
    }
    let a = chain(32);
    let b = chain(64);
    let ratio = b as f64 / a as f64;
    assert!(ratio < 2.3, "work growth {ratio} ({a} -> {b})");
}
