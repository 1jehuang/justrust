//! General callee expressions use the same canonical callable authority as local calls.
#![cfg(all(target_os = "linux", target_arch = "x86_64"))]
#[path = "support/iterator_oracle.rs"]
mod oracle;

#[test]
fn parenthesized_local_callable_matches_stock() {
    oracle::stock_native("fn main(){let f=|x:i32|x+1;println!(\"{}\",(f)(4));}");
}

#[test]
fn generic_callable_field_matches_stock() {
    oracle::stock_native(
        "struct Callable<F>{fun:F}fn apply<F:FnMut(i32)->i32>(mut value:Callable<F>)->i32{(value.fun)(4)}fn main(){println!(\"{}\",apply(Callable{fun:|x:i32|x+1}));}",
    );
}

#[test]
fn callee_is_evaluated_before_arguments_and_only_once() {
    oracle::stock_native(
        "fn choose<F>(counter:&mut i32,value:F)->F{*counter+=1;println!(\"callee\");value}fn argument()->i32{println!(\"argument\");4}fn main(){let mut counter=0;let value=(choose(&mut counter,|x:i32|x+1))(argument());assert_eq!(counter,1);println!(\"{}\",value);}",
    );
}

#[test]
fn function_pointer_types_remain_explicitly_unsupported() {
    let source = "struct Callable{fun:fn(i32,i32)->i32}fn add(a:i32,b:i32)->i32{a+b}fn main(){let value=Callable{fun:add};println!(\"{}\",(value.fun)(2,3));}";
    for workers in [1, 8] {
        for check in [true, false] {
            let error = match justrust_engine::compiler::compile(source.into(), workers, check) {
                Ok(_) => panic!("function pointer type admitted without ABI support"),
                Err(error) => error.to_string(),
            };
            assert!(
                error.contains("function pointer types are unsupported"),
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
