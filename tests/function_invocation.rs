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
fn function_pointer_coercion_and_calls_remain_explicitly_unsupported() {
    let source = r#"struct Callable{fun:fn(i32,i32)->i32}fn add(a:i32,b:i32)->i32{a+b}fn main(){let value=Callable{fun:add};println!("{}",(value.fun)(2,3));}"#;
    for workers in [1, 8] {
        for check in [true, false] {
            let error = match justrust_engine::compiler::compile(source.into(), workers, check) {
                Ok(_) => panic!("function pointer type admitted without ABI support"),
                Err(error) => error.to_string(),
            };
            assert!(
                error.to_lowercase().contains("unsupported") && error.contains("function pointer"),
                "{error}"
            );
        }
    }
}

#[test]
fn unused_bad_invocations_are_rejected() {
    for source in [
        "fn bad(){let f=4;(f)(1);}fn main(){}",
        "fn bad(){let f=|x:i32|x;(f)(1,2);}fn main(){}",
        "fn bad(){let f=|x:i32|x;(f)(true);}fn main(){}",
        "fn bad(){let f=|x:i32|x;(f)(;}fn main(){}",
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
        r#"fn unused(v:fn(i32)->i32){v(1);}fn main(){}"#,
        r#"fn unused(){let value:fn(i32)->i32=|x:i32|x;}fn main(){}"#,
        r#"fn add(x:i32)->i32{x}fn unused(){let value=add as fn(i32)->i32;}fn main(){}"#,
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
