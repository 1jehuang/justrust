//! Associated equality outputs constrain impl parameters, without inference
//! backwards through a non-injective associated projection.
#![cfg(all(target_os = "linux", target_arch = "x86_64"))]
use justrust_engine::compiler;
#[path = "support/iterator_oracle.rs"]
mod oracle;

#[test]
fn native_iterator_prelude_preserves_plain_program() {
    oracle::stock_native("fn main(){println!(\"ok\");}");
}

#[test]
fn unchanged_sieve_matches_stock() {
    oracle::stock_native(include_str!("fixtures/full_programs/04_primes_sieve.rs"));
}

#[test]
fn native_vec_shared_iteration_matches_stock() {
    oracle::stock_native(
        "fn main(){let empty:Vec<i32>=Vec::new();for x in &empty{println!(\"{}\",x);} let values=vec![3i32,5,7];let mut total=0;for (index,&value) in values.iter().enumerate(){total+=index as i32+value;}for value in &values{total+=*value;}println!(\"{}\",total);let mut it=values.iter();let first=it.next();let second=it.next();match(first,second){(Some(a),Some(b))=>println!(\"{} {}\",*a,*b),_=>()}}",
    );
}

#[test]
fn native_vec_iteration_preserves_loans() {
    for source in [
        "fn main(){let mut values=vec![1i32,2];for value in &values{values.push(3);println!(\"{}\",*value);}}",
        "fn main(){let result:&i32;{let values=vec![7i32];result=values.iter().next().unwrap();}println!(\"{}\",*result);}",
        "struct Fake;impl Fake{fn iter(&self)->i32{0}}fn main(){for n in Fake.iter(){println!(\"{}\",n);}}",
        "fn escape<'a>(values:&'a Vec<i32>)->&'static i32{values.iter().next().unwrap()}fn main(){}",
        "fn unrelated<'a,'b>(values:&'a Vec<i32>,other:&'b i32)->&'b i32{values.iter().next().unwrap()}fn main(){}",
    ] {
        oracle::stock_and_native_reject(source);
    }
}

#[test]
fn associated_output_determines_unused_impl_type_parameter() {
    oracle::stock_native(
        "trait Has{type Out;} struct S; impl Has for S{type Out=i32;} struct Wrap<I>(I); trait Value{fn value(&self)->i32;} impl<T,I:Has<Out=T>> Value for Wrap<I>{fn value(&self)->i32{42}} fn main(){let x=Wrap(S);println!(\"{}\",x.value());}",
    );
}

#[test]
fn unused_unconstrained_impl_parameters_still_reject() {
    for source in [
        "trait Tr{} struct S; impl<T> Tr for S{} fn main(){}",
        "trait Has{type Out;} trait Tr{} struct S; impl<T:Has<Out=U>,U:Has<Out=T>> Tr for S{} fn main(){}",
    ] {
        oracle::stock_and_native_reject(source);
        for workers in [1, 8] {
            for check in [true, false] {
                let error = compiler::compile(source.into(), workers, check)
                    .err()
                    .expect("unused invalid impl must reject");
                assert!(error.to_string().contains("unconstrained"), "{error}");
            }
        }
    }
}

#[test]
fn typed_vec_iter_loops_preserve_loans() {
    for source in [
        // (1) the iterator outlives its vector.
        "fn main(){let r;{let v:Vec<i32>=vec![1];r=v.iter();}for x in r{println!(\"{}\",*x);}}",
        // (2) mutation while the loop holds the shared loan.
        "fn main(){let mut v:Vec<i32>=vec![1];for x in v.iter(){v.push(*x);}}",
        // (3) a loop item escapes past the vector's scope.
        "fn main(){let r:&i32;{let v:Vec<i32>=vec![1,2];let mut last:&i32=&0;for x in v.iter(){last=x;}r=last;}println!(\"{}\",*r);}",
        // Same shapes in an unused function.
        "fn unused()->i32{let r;{let v:Vec<i32>=vec![1];r=v.iter();}let mut s:i32=0;for x in r{s+=*x;}s} fn main(){}",
    ] {
        oracle::stock_and_native_reject(source);
    }
}

#[test]
fn typed_vec_iter_loop_sieve_shape_matches_stock() {
    oracle::stock_native(
        "fn main(){let limit:usize=30;let mut is_prime:Vec<bool>=vec![true;limit+1];is_prime[0]=false;is_prime[1]=false;let mut i:usize=2;while i*i<=limit{if is_prime[i]{let mut j:usize=i*i;while j<=limit{is_prime[j]=false;j+=i;}}i+=1;}let mut count:usize=0;for p in is_prime.iter(){if *p{count+=1;}}let mut sum:i32=0;let v:Vec<i32>=vec![2,3,5];for &x in v.iter(){sum+=x;}println!(\"{} {}\",count,sum);}",
    );
}

#[test]
fn typed_vec_ref_into_iter_loops_match_stock_and_preserve_loans() {
    oracle::stock_native(
        "fn f(v:&Vec<i32>)->i32{let mut s:i32=0;for x in v{s+=*x;}s} fn main(){let v:Vec<i32>=vec![4,5];let mut t:i32=0;for &x in &v{t+=x;}println!(\"{} {}\",f(&v),t);}",
    );
    for source in [
        "fn main(){let mut v:Vec<i32>=vec![1];for x in &v{v.push(*x);}}",
        "fn bad<'a>(v:&'a Vec<i32>)->&'static i32{let mut last:&i32=&0;for x in v{last=x;}last} fn main(){}",
    ] {
        oracle::stock_and_native_reject(source);
    }
}

#[test]
fn source_iterator_count_and_bound_from_iterator() {
    // Default `count` is ordinary prelude source. A generic trait method
    // reached as `B::mk(..)` through a `B: Mk<A>` bound (the `B::from_iter`
    // shape) infers its own method actuals from the unique trait candidate.
    oracle::stock_native(
        "fn main(){let v=vec![1i32,2,3];println!(\"{}\",v.iter().count());let e:Vec<i32>=Vec::new();println!(\"{}\",e.iter().count());}",
    );
    oracle::stock_native(
        "trait Mk<A>{fn mk<X:Copy>(a:A,x:X)->Self;} struct N(i32); impl Mk<i32> for N{fn mk<X:Copy>(a:i32,_x:X)->N{N(a)}} fn build<A,B:Mk<A>>(a:A)->B{B::mk(a,1u8)} fn main(){let n:N=build(7i32);println!(\"{}\",n.0);}",
    );
    oracle::stock_and_native_reject(
        "trait Mk<A>{fn mk<X:Copy>(a:A,x:X)->Self;} struct N(i32); impl Mk<i32> for N{fn mk<X:Copy>(a:i32,_x:X)->N{N(a)}} fn build<A,B:Mk<A>>(a:A)->B{B::mk(a,String::new())} fn main(){let n:N=build(7i32);println!(\"{}\",n.0);}",
    );
    oracle::stock_and_native_reject(
        "fn main(){let v=vec![1i32];let n:i32=v.iter().count();println!(\"{}\",n);}",
    );
}
