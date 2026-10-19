use justrust_engine::compiler;

#[test]
fn associated_and_free_bodies_share_one_frontier_and_check_has_no_objects() {
    let source = r#"struct Pair<T,U>{a:T,b:U}struct W<T>{value:T}impl<T> W<T>{fn pair<U>(self,u:U)->Pair<U,T>{Pair::<U,T>{a:u,b:self.value}}}fn forward<T,U>(a:T,b:U)->Pair<U,T>{W::<T>{value:a}.pair::<U>(b)}fn run()->i32{let p=forward::<i32,u8>(3,4u8);p.a as i32+p.b}
fn main(){if run()==7{println!("associated items oracle ok");}}
"#;
    for workers in [1, 8] {
        for check in [true, false] {
            let compiled = compiler::compile(source.to_owned(), workers, check).unwrap();
            assert!(compiled.report.capture_complete);
            assert_eq!(compiled.executable.is_none(), check);
            assert_eq!(compiled.functions, 4);
            assert_eq!(compiled.control_flow.len(), 4);
            assert!(compiled.analyses.iter().all(|analysis| analysis.passed()));
            for kind in [102, 108, 109] {
                let facts: Vec<_> = compiled
                    .report
                    .node_evaluations
                    .iter()
                    .filter(|(key, _)| key.kind == kind)
                    .collect();
                assert_eq!(facts.len(), 4);
                assert!(facts.iter().all(|(_, count)| *count == 1));
            }
            assert_eq!(
                compiled
                    .report
                    .node_evaluations
                    .iter()
                    .filter(|(key, _)| {
                        key.kind == 112
                            && key
                                .params
                                .first()
                                .is_some_and(|index| *index < compiled.functions as u64)
                    })
                    .count(),
                2
            );
            assert_eq!(
                compiled
                    .report
                    .node_evaluations
                    .iter()
                    .filter(|(key, _)| key.kind == 103)
                    .count(),
                if check { 0 } else { 4 }
            );
        }
    }
}

#[test]
fn unused_generic_impl_methods_need_universal_proofs_even_without_method_parameters() {
    for source in [
        r#"struct W<T>{x:T}impl<T> W<T>{fn bad<U>(self,u:U)->(U,U){(u,u)}}fn run()->i32{7}
fn main(){run();}
"#,
        r#"struct W<T>{x:T}impl<T> W<T>{fn bad(self)->(T,T){(self.x,self.x)}}fn main(){println!("must not publish");}"#,
    ] {
        for workers in [1, 8] {
            for check in [true, false] {
                let failure = compiler::compile_observed(source.to_owned(), workers, check)
                    .err()
                    .unwrap();
                assert!(
                    failure
                        .analyses
                        .iter()
                        .any(|(_, analysis)| !analysis.passed())
                );
                let report = failure.report.unwrap();
                assert!(report.capture_complete);
                assert!(!report.published_kind(105));
                assert!(
                    report
                        .node_evaluations
                        .iter()
                        .any(|(key, _)| key.kind == 112)
                );
            }
        }
    }
}

#[test]
fn invalid_unused_headers_and_privacy_fail_before_native_publication() {
    for source in [
        r#"struct S{x:i32}impl S{fn bad<U>(self)->U{7}}fn run()->i32{7}
fn main(){run();}
"#,
        r#"struct S{x:i32}impl<T> S{fn unused(self)->i32{self.x}}fn run()->i32{7}
fn main(){run();}
"#,
        r#"struct W<T>{x:T}impl<T> W<T>{fn clash(self)->i32{3}}impl W<i32>{fn clash(self)->i32{4}}fn run()->i32{7}
fn main(){run();}
"#,
        r#"mod a{pub struct S{pub x:i32}impl S{fn take(self)->i32{self.x}}}fn run()->i32{a::S{x:7}.take()}
fn main(){run();}
"#,
        r#"mod a{pub struct S{x:i32}}mod b{impl super::a::S{pub fn leak(self)->i32{self.x}}}fn run()->i32{7}
fn main(){run();}
"#,
    ] {
        for workers in [1, 8] {
            for check in [true, false] {
                let failure = compiler::compile_observed(source.to_owned(), workers, check)
                    .err()
                    .unwrap();
                let report = failure.report.unwrap();
                assert!(report.capture_complete);
                assert!(!report.published_kind(105));
            }
        }
    }
}

#[test]
fn disjoint_impl_symbols_and_exact_recursive_method_instances_are_deterministic() {
    for (source, count) in [
        (
            r#"struct W<T>{x:T}impl W<u8>{fn score(self)->i32{1}}impl W<u16>{fn score(self)->i32{2}}struct Pair<T,U>{a:T,b:U}impl<T> Pair<T,T>{fn score(self)->i32{3}}impl Pair<i32,u8>{fn score(self)->i32{1}}fn run()->i32{W::<u8>{x:0u8}.score()+W::<u16>{x:0u16}.score()+Pair::<i32,i32>{a:0,b:0}.score()+Pair::<i32,u8>{a:0,b:0u8}.score()}
fn main(){if run()==7{println!("associated items oracle ok");}}
"#,
            6,
        ),
        (
            r#"struct W<T>{x:T}impl<T> W<T>{fn recur<U>(self,u:U,n:i32)->(T,U){if n==0{(self.x,u)}else{self.recur::<U>(u,n-1)}}}fn run()->i32{let p=W::<i32>{x:3}.recur::<u8>(4u8,3);p.0+p.1 as i32}
fn main(){if run()==7{println!("associated items oracle ok");}}
"#,
            3,
        ),
    ] {
        let one = compiler::compile(source.to_owned(), 1, false).unwrap();
        let eight = compiler::compile(source.to_owned(), 8, false).unwrap();
        assert_eq!(one.executable, eight.executable);
        for compiled in [&one, &eight] {
            assert!(compiled.report.capture_complete);
            assert_eq!(compiled.control_flow.len(), count);
            assert!(compiled.analyses.iter().all(|analysis| analysis.passed()));
            assert_eq!(
                compiled
                    .report
                    .node_evaluations
                    .iter()
                    .filter(|(key, _)| key.kind == 103)
                    .count(),
                count
            );
        }
    }
}
