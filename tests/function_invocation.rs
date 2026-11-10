//! General callee expressions use the same canonical callable authority as local calls.
#![cfg(all(target_os = "linux", target_arch = "x86_64"))]
#[path = "support/iterator_oracle.rs"]
mod oracle;

#[test]
fn parenthesized_local_callable_matches_stock() {
    oracle::stock_native(r#"fn main(){let f=|x:i32|x+1;println!("{}",(f)(4));}"#);
}

#[test]
fn generic_callable_field_matches_stock() {
    oracle::stock_native(
        r#"struct Callable<F>{fun:F}fn apply<F:FnMut(i32)->i32>(mut value:Callable<F>)->i32{(value.fun)(4)}fn main(){println!("{}",apply(Callable{fun:|x:i32|x+1}));}"#,
    );
}

#[test]
fn callee_is_evaluated_before_arguments_and_only_once() {
    oracle::stock_native(
        r#"fn choose<F>(counter:&mut i32,value:F)->F{*counter+=1;println!("callee");value}fn argument()->i32{println!("argument");4}fn main(){let mut counter=0;let value=(choose(&mut counter,|x:i32|x+1))(argument());assert_eq!(counter,1);println!("{}",value);}"#,
    );
}

#[test]
fn function_pointer_coercion_and_calls_match_stock() {
    let source = r#"struct Callable{fun:fn(i32,i32)->i32}fn add(a:i32,b:i32)->i32{a+b}fn main(){let value=Callable{fun:add};println!("{}",(value.fun)(2,3));}"#;
    oracle::stock_native(source);
}

#[test]
fn native_function_pointer_values_aggregate_context_and_exact_call_abi() {
    oracle::stock_native(
        r#"
        fn add(x:i32)->i32{x+7}
        fn sub(x:i32)->i32{x-2}
        fn id<T>(x:T)->T{x}
        fn apply(f:fn(i32)->i32,x:i32)->i32{f(x)}
        fn choose(flag:bool)->fn(i32)->i32{if flag{add}else{sub}}
        fn pair(x:(i64,bool))->(i64,bool){(x.0+1,!x.1)}
        fn main(){
            let fs:[fn(i32)->i32;2]=[add,sub];
            let tuple:(fn(i32)->i32,fn(u8)->u8)=(add,id);
            let copied=fs;
            let f=choose(false);
            let p:fn((i64,bool))->(i64,bool)=pair;
            let cast=add as fn(i32)->i32;
            let floats:[fn(f64)->f64;2]=[f64::sqrt,f64::abs];
            println!("{} {} {} {} {} {} {:?} {} {}",fs[0](5),copied[1](5),apply(add,4),f(9),(tuple.0)(1),(tuple.1)(3),p((8,true)),cast(2),floats[0](16.0)+floats[1](-2.0));
            let mut selected:fn(i32)->i32=add;
            let before=selected({selected=sub;5});
            println!("{} {}",before,selected(5));
        }
    "#,
    );
}

#[test]
fn function_pointers_generic_bounds_owned_arguments_and_wide_results_match_stock() {
    oracle::stock_native(
        r#"
        fn id<T>(x:T)->T{x}
        fn maker<T>()->fn(T)->T{id}
        fn apply<T>(f:fn(T)->T,x:T)->T{f(x)}
        fn array<T>(fs:[fn(T)->T;2],x:T)->T{fs[1](x)}
        fn tuple<T>(fs:(fn(T)->T,fn(T)->T),x:T)->T{(fs.0)(x)}
        fn bounded<F:FnOnce(i128)->i128>(f:F,x:i128)->i128{f(x)}
        fn shared<F:Fn(i128)->i128>(f:F,x:i128)->i128{f(x)}
        fn mutable<F:FnMut(i128)->i128>(mut f:F,x:i128)->i128{f(x)}
        fn inc(x:i128)->i128{x+1}
        fn pair(a:i128,b:i128)->(i128,i128){(a+1,b+2)}
        fn main(){
            let string:fn(String)->String=id;
            let returned=maker::<u8>();
            let wide:fn(i128)->i128=inc;
            let many:fn(i128,i128)->(i128,i128)=pair;
            println!("{} {} {}",apply(string,String::from("owned")),bounded(wide,18446744073709551616i128),apply(id::<u8>,7u8));
            println!("{:?}",many(18446744073709551616i128,36893488147419103232i128));
            println!("{} {}",shared(wide,10),mutable(wide,20));
            println!("{} {} {}",array([id::<u8>,id::<u8>],9u8),tuple((id::<u8>,id::<u8>),10u8),returned(11u8));
        }
    "#,
    );
}

#[test]
fn forged_function_pointer_reification_signatures_and_unvalidated_facts_reject() {
    use justrust_engine::compiler::{cfg, frontend, ir};
    use std::sync::Arc;
    let program =
        frontend::parse("fn inc(x:i32)->i32{x+1}fn get()->fn(i32)->i32{inc}fn main(){}").unwrap();
    frontend::check(&program).unwrap();
    let budget = ir::PlanBudget::default();
    let plan = frontend::PlanCtx::from_program(&program, &budget).unwrap();
    let declaration = plan.function_decl(1).unwrap();
    let facts = plan.callable_facts(declaration).unwrap();
    let target = ir::CallTarget {
        declaration,
        type_args: Arc::from([]),
    };
    let checked = plan.checked_body(&target, &facts, facts.budget()).unwrap();
    checked
        .validate_with_facts(ir::FactsCtx::Validated(&facts))
        .unwrap();
    let (site, source) = checked
        .exprs
        .iter()
        .enumerate()
        .find_map(|(site, expression)| match expression.kind {
            ir::ExprKind::PtrCast { value } => Some((site, value)),
            _ => None,
        })
        .unwrap();
    let from = &checked.exprs[source.0].ty;
    for (inputs, output) in [
        (vec![ir::Ty::Bool], ir::Ty::Int(ir::IntTy::I32)),
        (vec![ir::Ty::Int(ir::IntTy::I32)], ir::Ty::Bool),
        (vec![], ir::Ty::Int(ir::IntTy::I32)),
    ] {
        let to = ir::Ty::FnPointer(ir::FnPointerTy::new(inputs, output, facts.budget()).unwrap());
        assert!(ir::validate_pointer_cast(from, &to, ir::FactsCtx::Validated(&facts)).is_err());
        let mut forged = checked.clone();
        forged.exprs[site].ty = to;
        assert!(
            forged
                .validate_with_facts(ir::FactsCtx::Validated(&facts))
                .is_err()
        );
        assert!(cfg::lower_template(Arc::new(forged), facts.clone(), facts.budget()).is_err());
    }
    assert!(
        ir::validate_pointer_cast(from, &checked.exprs[site].ty, ir::FactsCtx::Concrete).is_err()
    );

    use justrust_engine::compiler::{source, token_primitives::ExpansionBudget};
    let options = source::SourceOptions::default();
    let expansion = ExpansionBudget::default();
    let root = source::prepare_prelude_unit(&options, &expansion).unwrap();
    let library = source::prepare_child_unit(&root, 0, &options, &expansion).unwrap();
    let index = library
        .children
        .iter()
        .position(|child| child.path.ends_with("ops.rs"))
        .unwrap();
    let ops = source::prepare_child_unit(&library, index, &options, &expansion).unwrap();
    // Mount the unchanged embedded callable declarations at their original route.
    // Ordinary dependencies need no builtin identity for this checked body.
    let user_text = r#"
mod __justrust_std {
    pub mod option { pub enum Option<T> { Some(T), None } }
    pub mod cmp {
        pub trait PartialEq<Rhs:?Sized=Self> { fn eq(&self, other:&Rhs)->bool; }
        pub trait Eq:PartialEq {}
        pub trait PartialOrd:PartialEq { fn le(&self, other:&Self)->bool; }
    }
    pub mod iter {
        pub trait Iterator { type Item; fn next(&mut self)->super::option::Option<Self::Item>; }
        pub trait DoubleEndedIterator:Iterator { fn next_back(&mut self)->super::option::Option<Self::Item>; }
    }
    impl<T> iter::Iterator for ops::Range<T> {
        type Item=T;
        fn next(&mut self)->option::Option<T>{option::Option::None}
    }
    pub mod ops;
    pub mod prelude { pub mod v1 { pub use super::super::ops::{Fn,FnMut,FnOnce}; pub use super::super::cmp::{PartialEq,Eq,PartialOrd}; } }
}
fn invoke(f:fn(i32)->i32)->i32{f(1)}fn main(){}
"#;
    let fixture_root = std::path::PathBuf::from("/function-pointer-source-fixture");
    let snapshot = source::SourceSnapshot::new(
        fixture_root.clone(),
        [
            (
                std::path::PathBuf::from("main.rs"),
                user_text.as_bytes().to_vec(),
            ),
            (
                std::path::PathBuf::from("__justrust_std/ops.rs"),
                ops.contents.as_bytes().to_vec(),
            ),
        ],
    )
    .unwrap();
    let options = source::SourceOptions {
        source_snapshot: Some(snapshot),
        ..options
    };
    let user = source::prepare_root_unit(&fixture_root.join("main.rs"), &options, &expansion);
    let mut assembler = source::SourceAssembler::user_only(&options);
    let source::AssemblyStep::Need(need) = assembler.provide(Arc::new(user)).unwrap() else {
        panic!("real embedded ops child was not requested");
    };
    let mut original_module = root.children[0].module.clone();
    original_module.extend(ops.module.iter().cloned());
    assert_eq!(
        need.parent.as_ref().unwrap().children[need.index].module,
        original_module
    );
    let source::AssemblyStep::Done(prepared) = assembler.provide(Arc::new(Ok(ops))).unwrap() else {
        panic!("unexpected ops source dependency");
    };
    let program = frontend::resolve(&frontend::parse_prepared(&prepared).unwrap()).unwrap();
    let budget = ir::PlanBudget::default();
    let plan = frontend::PlanCtx::from_program(&program, &budget).unwrap();
    let declaration = plan
        .sema_declarations()
        .iter()
        .find(|info| {
            info.kind == ir::DeclKind::Function
                && !plan.is_prelude_decl(info.id).unwrap()
                && plan.sema_item_name(info).as_deref() == Some("invoke")
        })
        .unwrap()
        .id;
    let facts = plan.callable_facts(declaration).unwrap();
    let target = ir::CallTarget {
        declaration,
        type_args: Arc::from([]),
    };
    let checked = plan.checked_body(&target, &facts, facts.budget()).unwrap();
    checked
        .validate_with_facts(ir::FactsCtx::Validated(&facts))
        .unwrap();
    let mut forged = checked.clone();
    let call = forged
        .exprs
        .iter_mut()
        .find_map(|expression| match &mut expression.kind {
            ir::ExprKind::Call {
                target: ir::Callee::Untupled(call),
                ..
            } => Some(call),
            _ => None,
        })
        .unwrap();
    call.trait_ref.self_ty = ir::Ty::FnPointer(
        ir::FnPointerTy::new(vec![ir::Ty::Bool], ir::Ty::Bool, facts.budget()).unwrap(),
    );
    assert!(
        forged
            .validate_with_facts(ir::FactsCtx::Validated(&facts))
            .is_err()
    );
    assert!(cfg::lower_template(Arc::new(forged), facts.clone(), facts.budget()).is_err());
}

#[test]
fn unused_bad_invocations_are_rejected() {
    for source in [
        "fn bad(){let f=4;(f)(1);}fn main(){}",
        "fn bad(){let f=|x:i32|x;(f)(1,2);}fn main(){}",
        "fn bad(){let f=|x:i32|x;(f)(true);}fn main(){}",
        "fn bad(){let f=|x:i32|x;(f)(;}fn main(){}",
        "fn bad(f:fn(i32)->i32){f(true);}fn main(){}",
        "fn bad(f:fn(i32)->i32){f(1,2);}fn main(){}",
        "fn bad(f:fn(i32)->i32)->bool{f(1)}fn main(){}",
        "fn a(x:i32)->i32{x}fn bad(){let f:fn(bool)->i32=a;}fn main(){}",
        "fn a(x:i32)->i32{x}fn bad(){let f:fn(i32)->bool=a;}fn main(){}",
        "fn a(x:i32)->i32{x}fn b(x:bool)->i32{0}fn bad(){let fs:[fn(i32)->i32;2]=[a,b];}fn main(){}",
        "fn id<T>(x:T)->T{x}fn apply<T>(f:fn(T)->T,x:T){}fn bad(){apply(id::<bool>,7u8);}fn main(){}",
        "unsafe fn item(x:i32)->i32{x}fn apply<T>(f:fn(T)->T,x:T){}fn bad(){apply(item,1i32);}fn main(){}",
        "fn id<T>(x:T)->T{x}fn take<T>(fs:(fn(T)->T,fn(T)->T),x:T)->T{(fs.0)(x)}fn bad(){let fs=(id::<u8>,id::<u8>);take(fs,1u8);}fn main(){}",
        "fn id<T>(x:T)->T{x}fn take<T>(fs:[fn(T)->T;2],x:T)->T{fs[0](x)}fn bad(){let fs=[id::<u8>,id::<u8>];take(fs,2u8);}fn main(){}",
    ] {
        oracle::stock_and_native_reject(source);
    }
}

#[test]
fn canonical_function_pointer_types_match_stock_without_indirect_calls() {
    for source in [
        r#"fn require<T:Copy+Sized>(){}fn witness<S>(){require::<fn(S)->S>();}fn main(){witness::<i32>();println!("fn types");}"#,
        r#"struct NotCopy{value:i32}fn witness<S>(marker:[fn(S)->S;0]){let first=marker;let second=marker;}fn main(){witness::<NotCopy>([]);println!("copy signature");}"#,
        r#"struct Marker<F>{field:[F;0]}fn make<S>()->Marker<fn(S)->S>{Marker{field:[]}}fn witness<S>()->i32{let value=make::<S>();let first=value.field;let second=value.field;7}fn main(){println!("{}",witness::<i32>());}"#,
        r#"struct Marker<T>{field:[fn(T)->T;0]}fn make<T>()->Marker<T>{Marker{field:[]}}fn witness<T>(value:Marker<T>)->i32{let first=value.field;let second=value.field;7}fn main(){println!("{}",witness(make::<i32>()));}"#,
        r#"fn require<T:Copy+Sized>(){}fn witness<S>(){require::<fn(fn(S)->S)->fn()->S>();require::<fn(left:S,right:bool)->S>();require::<fn()->!>();}fn main(){witness::<i32>();println!("nested signature");}"#,
    ] {
        oracle::stock_native(source);
    }
}

#[test]
fn distinct_function_pointer_signatures_do_not_unify() {
    for source in [
        r#"fn take(v:[fn(i32)->bool;0]){}fn main(){let value:[fn(bool)->i32;0]=[];take(value);}"#,
        r#"fn take(v:[fn(i32,i32)->i32;0]){}fn main(){let value:[fn(i32)->i32;0]=[];take(value);}"#,
        r#"fn unused<S>(v:[fn(S)->S;0])->[fn(S)->bool;0]{v}fn main(){}"#,
        r#"fn unused(v:[fn(Unknown)->i32;0]){}fn main(){}"#,
    ] {
        oracle::stock_and_native_reject(source);
    }
}

#[test]
fn unused_function_pointer_signatures_reject_nested_impl_trait_and_nonreturn_never() {
    for source in [
        r#"fn bad()->fn()->impl Copy{loop{}}fn main(){}"#,
        r#"fn bad(value:fn()->impl Copy){}fn main(){}"#,
        r#"fn bad(value:fn(impl Copy)){}fn main(){}"#,
        r#"fn bad(value:fn(!)->i32){}fn main(){}"#,
        r#"fn bad(value:fn()->(!)){}fn main(){}"#,
    ] {
        oracle::stock_and_native_reject(source);
    }
}

#[test]
fn unsupported_function_pointer_safety_abi_binders_and_value_forms_are_explicit() {
    for source in [
        r#"fn unused(v:unsafe fn(i32)->i32){}fn main(){}"#,
        r#"fn unused(v:extern "C" fn(i32)->i32){}fn main(){}"#,
        r#"fn unused(v:for<'a> fn(&'a i32)->&'a i32){}fn main(){}"#,
        r#"fn unused(v:extern "C" fn(i32,...)){}fn main(){}"#,
        r#"fn unused(){let value:fn(i32)->i32=|x:i32|x;}fn main(){}"#,
        r#"unsafe fn add(x:i32)->i32{x}fn unused(){unsafe{let value:fn(i32)->i32=add;}}fn main(){}"#,
    ] {
        for workers in [1, 8] {
            for check in [true, false] {
                let error = justrust_engine::compiler::compile(source.into(), workers, check)
                    .err()
                    .expect("unsupported function pointer form accepted")
                    .to_string();
                assert!(
                    error.to_lowercase().contains("unsupported")
                        && error.contains("function pointer"),
                    "{error}\n{source}"
                );
            }
        }
    }
}
