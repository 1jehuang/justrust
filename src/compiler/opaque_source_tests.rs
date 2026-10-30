use super::stock_rustc;
use super::*;

fn source_check(source: &str, workers: usize) -> Result<(), String> {
    let snapshot = Arc::new(Snapshot {
        input: Input::Text(source.into()),
        source_options: source::SourceOptions::default(),
        budget: ir::PlanBudget::default(),
    });
    let engine = Engine::new(CompilerProvider(snapshot));
    let check = engine.request(crate::compiler::keys::CheckRoot.key());
    let report = engine.run(workers).unwrap();
    assert!(report.capture_complete);
    assert!(
        !report.node_evaluations.iter().any(|(key, _)| {
            matches!(
                key.kind,
                BUILD | OBJECT | OPAQUE_REPRESENTATION | OPAQUE_CALL_DISPATCH | INSTANCE
            )
        }),
        "source CHECK demanded physical instance or executable facts"
    );
    check.completion().map_err(|error| format!("{error:?}"))
}

#[test]
fn opaque_source_check_uses_declared_bounds_without_representation_demands() {
    let source = r#"
        trait Read { fn read(&self) -> i32; }
        struct Number { value: i32 }
        impl Read for Number { fn read(&self) -> i32 { self.value } }
        fn make() -> impl Read { Number { value: 42 } }
        fn main() { let value = make(); assert!(value.read() == 42); }
    "#;
    for workers in [1, 8] {
        source_check(source, workers).unwrap();
    }
}

#[test]
fn opaque_source_check_validates_unused_associated_constants_without_physical_demands() {
    let source = r#"
        trait Read { fn read(&self) -> i32; }
        trait Flag { const VALUE: u8; }
        struct Number { value: i32 }
        impl Read for Number { fn read(&self) -> i32 { self.value } }
        impl Flag for Number { const VALUE: u8 = 1; }
        fn make() -> impl Read { Number { value: 42 } }
        fn main() { let value = make(); assert!(value.read() == 42); }
    "#;
    for workers in [1, 8] {
        source_check(source, workers).unwrap();
        for invalid in [
            source.replace("const VALUE: u8 = 1;", "const VALUE: u8 = true;"),
            source.replace("const VALUE: u8 = 1;", ""),
        ] {
            assert!(
                source_check(&invalid, workers).is_err(),
                "opaque source admission omitted unused associated constant: {invalid}"
            );
        }
    }
}

#[test]
fn opaque_captured_compose_pack_preserves_source_ownership() {
    let source = r#"
        fn compose<A,B,C>(f:impl Fn(A)->B,g:impl Fn(B)->C)->impl Fn(A)->C {
            move |x:A|g(f(x))
        }
        fn main() {}
    "#;
    for workers in [1, 8] {
        source_check(source, workers).unwrap();
    }
}

#[test]
fn opaque_captured_pack_repeated_family_is_explicitly_unsupported() {
    let source = "fn same<F:Fn(i32)->i32>(first:F,second:F)->impl Fn(i32)->i32{move |x:i32|first(second(x))}fn main(){}";
    let error = source_check(source, 1).unwrap_err();
    assert!(
        error.contains("unsupported: opaque repeated-family capture fields"),
        "unexpected boundary: {error}"
    );
}

#[test]
fn opaque_native_object_build_matches_stock_workers1_8() {
    let source = r#"
        trait Read { fn read(&self) -> i32; }
        struct Number { value: i32 }
        impl Read for Number { fn read(&self) -> i32 { self.value } }
        fn make() -> impl Read { Number { value: 42 } }
        fn main() { let value = make(); assert!(value.read() == 42); }
    "#;
    native_matches_stock(source);
}

#[test]
fn opaque_forwarded_declarations_match_stock_workers1_8() {
    native_matches_stock(
        r#"
        struct V;
        trait Inspect { fn value(&self) -> i32; }
        impl Inspect for V { fn value(&self) -> i32 { 42 } }
        fn make() -> impl Inspect { V }
        fn forward() -> impl Inspect { make() }
        fn again() -> impl Inspect { forward() }
        fn main() { assert!(again().value() == 42); }
    "#,
    );
    native_matches_stock(
        r#"
        trait Inspect { type Item; fn value(&self) -> i32; }
        struct V;
        impl Inspect for V { type Item = i32; fn value(&self) -> i32 { 42 } }
        fn make<T: Copy, U: Copy>(_t: T, _u: U) -> impl Inspect<Item = i32> { V }
        fn forward<A: Copy, B: Copy>(a: A, b: B) -> impl Inspect<Item = i32> { make::<B, A>(b, a) }
        fn main() {
            assert!(forward::<u8, u16>(1u8, 2u16).value() == 42);
            assert!(forward::<u16, u8>(1u16, 2u8).value() == 42);
        }
    "#,
    );
    native_matches_stock_with_dispatch(
        "fn one() -> impl Sized { 1 } fn two() -> impl Sized { one() } fn main() {}",
        false,
    );
}

#[test]
fn opaque_projection_normalization_preserves_supported_equality_headers() {
    native_matches_stock_with_dispatch(
        "trait Has{type Out;}fn hide<T:Has<Out=i32>>(x:T::Out)->impl Sized{x}fn main(){}",
        false,
    );
}

#[test]
fn opaque_forwarding_declaration_validation_keeps_body_boundaries() {
    struct Probe(CompilerProvider);
    impl Provider for Probe {
        fn outputs(&self, key: &Key) -> usize {
            self.0.outputs(key)
        }
        fn start(&self, cx: Context, key: Key) -> TaskFuture {
            if key.kind != 9006 {
                return self.0.start(cx, key);
            }
            Box::pin(async move {
                let facts = Facts { cx: &cx };
                let types = facts.types().await?;
                let owner = types
                    .callable_decl(key.params[0] as usize)
                    .map_err(Error::new)?;
                let admission: Arc<opaque_declarations::Admission> = cx
                    .read_item(
                        &keys::opaque_declaration::OpaqueDeclaration::Validate(owner),
                        0,
                    )
                    .await?;
                admission.validate_owner(owner)?;
                assert!(
                    admission
                        .validate_owner(types.callable_decl(0).unwrap())
                        .is_err()
                );
                cx.publish_fact(0, ())
            })
        }
    }
    for workers in [1, 8] {
        let source = "trait Inspect {} struct V; impl Inspect for V {} fn make()->impl Inspect{V} fn forward()->impl Inspect{make()} fn unrelated()->bool{1} fn main(){}";
        let snapshot = || {
            Arc::new(Snapshot {
                input: Input::Text(source.into()),
                source_options: source::SourceOptions::default(),
                budget: ir::PlanBudget::default(),
            })
        };
        let engine = Engine::new(CompilerProvider(snapshot()));
        let body_key = keys::item(TEMPLATE_BODY, 1);
        let body = engine.request(body_key.clone());
        let report = engine.run(workers).unwrap();
        body.completion().unwrap();
        assert!(
            !report
                .node_evaluations
                .iter()
                .any(|(key, _)| key.kind == TEMPLATE_BODY && key.params == vec![0]),
            "public source checking read the defining callee body"
        );
        let engine = Engine::new(Probe(CompilerProvider(snapshot())));
        let declaration = engine.request(keys::test_key(9006, vec![1]));
        let report = engine.run(workers).unwrap();
        declaration.completion().unwrap();
        assert_eq!(
            report
                .node_evaluations
                .iter()
                .filter(
                    |(key, _)| keys::opaque_declaration::OpaqueDeclaration::recognizes(key)
                        && key.params.get(2) == Some(&0)
                )
                .count(),
            2
        );
        assert_eq!(
            report
                .node_evaluations
                .iter()
                .filter(
                    |(key, _)| keys::opaque_declaration::OpaqueDeclaration::recognizes(key)
                        && key.params.get(2) == Some(&1)
                )
                .count(),
            1
        );
        assert!(
            !report
                .node_evaluations
                .iter()
                .any(|(key, _)| key.kind == TEMPLATE_BODY && key.params == vec![2]),
            "declaration validation demanded an unrelated body"
        );
        assert!(
            !report.node_evaluations.iter().any(|(key, _)| matches!(
                key.kind,
                BUILD | OBJECT | OPAQUE_REPRESENTATION | OPAQUE_CALL_DISPATCH | INSTANCE
            )),
            "source declaration validation demanded physical facts"
        );
        assert!(
            source_check(source, workers).is_err(),
            "CHECK omitted the unrelated invalid unused body"
        );
        for cycle in [
            "fn first()->impl Sized{second()} fn second()->impl Sized{first()} fn main(){}",
            "fn first<T:Copy>(v:T)->impl Sized{second(v)} fn second<U:Copy>(v:U)->impl Sized{first(v)} fn main(){}",
            "fn growing<T:Copy>(v:T)->impl Sized{growing((v,v))} fn main(){}",
            "fn first()->impl FnOnce()->i32{second()}fn second()->impl FnOnce()->i32{first()}fn main(){}",
        ] {
            let snapshot = Arc::new(Snapshot {
                input: Input::Text(cycle.into()),
                source_options: source::SourceOptions::default(),
                budget: ir::PlanBudget::default(),
            });
            let engine = Engine::new(Probe(CompilerProvider(snapshot)));
            let requested = engine.request(keys::test_key(9006, vec![0]));
            let report = engine.run(workers).unwrap();
            let error = requested.completion().unwrap_err().to_string();
            assert!(
                error.contains("unsupported: recursive opaque declaration"),
                "declaration cycle protocol did not reject its own cycle: {error}"
            );
            assert!(!report.node_evaluations.iter().any(|(key, _)| matches!(
                key.kind,
                BUILD | OBJECT | OPAQUE_REPRESENTATION | OPAQUE_CALL_DISPATCH | INSTANCE
            )));
        }
    }
}

#[test]
fn opaque_closed_generic_producers_match_stock_workers1_8() {
    native_matches_stock(
        r#"
        trait Read { fn read(&self) -> i32; }
        struct Number { value: i32 }
        impl Read for Number { fn read(&self) -> i32 { self.value } }
        fn make<T: Copy>(arg: T) -> impl Read {
            let copied = arg;
            Number { value: 42 }
        }
        fn main() {
            let first = make::<u8>(1u8);
            let second = make::<u16>(2u16);
            let mut third = make::<u8>(3u8);
            third = make::<u8>(4u8);
            assert!(third.read() == 42);
            assert!(first.read() == 42);
            assert!(second.read() == 42);
        }
    "#,
    );
    native_matches_stock(
        r#"
        trait Read { fn read(&self) -> i32; }
        struct Number { value: i32 }
        impl Read for Number { fn read(&self) -> i32 { self.value } }
        fn make<A: Copy, B: Copy>(a: A, b: B) -> impl Read {
            let copied = a;
            let other = b;
            Number { value: 42 }
        }
        fn read_generic<T: Copy>(input: T) -> i32 {
            let value = make(input, 2u16);
            value.read()
        }
        fn main() {
            let first = make(1u8, 2u16);
            let second = make(2u16, 1u8);
            assert!(first.read() == 42);
            assert!(second.read() == 42);
            assert!(read_generic(3u32) == 42);
        }
    "#,
    );
}

#[test]
fn opaque_generic_results_do_not_merge_distinct_owner_arguments() {
    use std::process::Command;
    let prefix = "trait Read { fn read(&self)->i32; } struct Number { value:i32 } impl Read for Number { fn read(&self)->i32 { self.value } } fn make<A:Copy,B:Copy>(a:A,b:B)->impl Read { Number { value:42 } }";
    for tail in [
        "fn main(){let mut value=make(1u8,2u16);value=make(2u16,1u8);}",
        "fn other<A:Copy,B:Copy>(a:A,b:B)->impl Read {Number{value:42}} fn main(){let mut value=make(1u8,2u16);value=other(1u8,2u16);}",
        "fn forward<A:Copy,B:Copy>(a:A,b:B)->impl Read{make(a,b)} fn main(){let mut value=forward(1u8,2u16);value=forward(2u16,1u8);}",
        "fn forward()->impl Read{make(1u8,2u16)} fn main(){let mut value=make(1u8,2u16);value=forward();}",
        "fn forward()->impl Read{make(1u8,2u16)} fn main(){let _n=forward().value;}",
        "fn forward()->impl Read{make(1u8,2u16)} fn main(){let _n:Number=forward();}",
        "fn first()->impl Read{second()} fn second()->impl Read{first()} fn main(){}",
        "fn first<T:Copy>(v:T)->impl Read{second(v)} fn second<U:Copy>(v:U)->impl Read{first(v)} fn main(){}",
        "fn growing<T:Copy>(v:T)->impl Sized{growing((v,v))} fn main(){}",
        "fn first()->impl FnOnce()->i32{second()}fn second()->impl FnOnce()->i32{first()}fn main(){}",
        "trait Has{type Out;} impl Has for Number{type Out=i32;} fn inner()->impl Has<Out=i32>{Number{value:42}} fn forward()->impl Has<Out=bool>{inner()} fn main(){}",
    ] {
        let source = format!("{prefix}{tail}");
        let directory = std::env::temp_dir().join(format!(
            "opaque-invalid-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir(&directory).unwrap();
        let input = directory.join("main.rs");
        std::fs::write(&input, &source).unwrap();
        let stock = Command::new(stock_rustc::stock_rustc())
            .env_remove("RUSTC_BOOTSTRAP")
            .args(["--edition=2024", "--emit=metadata"])
            .arg(&input)
            .arg("-o")
            .arg(directory.join("out.rmeta"))
            .output()
            .unwrap();
        assert!(!stock.status.success(), "stock accepted {source}");
        for workers in [1, 8] {
            assert!(source_check(&source, workers).is_err());
            let snapshot = Arc::new(Snapshot {
                input: Input::Text(source.clone()),
                source_options: source::SourceOptions::default(),
                budget: ir::PlanBudget::default(),
            });
            let engine = Engine::new(CompilerProvider(snapshot));
            let key = crate::compiler::keys::BuildRoot.key();
            let build = engine.request(key.clone());
            engine.run(workers).unwrap();
            assert!(build.completion().is_err(), "built invalid {source}");
            assert!(engine.fact_of::<Vec<u8>>(&key, 0).is_none());
        }
        std::fs::remove_dir_all(directory).unwrap();
    }
}

#[test]
fn opaque_associated_equality_bounds_preserve_declaration_authority() {
    let valid = r#"
        trait HasOutput { type Output; }
        struct Number { value: i32 }
        impl HasOutput for Number { type Output = i32; }
        fn make() -> impl HasOutput<Output = i32> { Number { value: 42 } }
        fn main() { let value = make(); }
    "#;
    for workers in [1, 8] {
        source_check(valid, workers).unwrap();
        let repeated = valid.replace(
            "impl HasOutput<Output = i32>",
            "impl HasOutput<Output = i32, Output = i32>",
        );
        source_check(&repeated, workers).unwrap();
        for invalid in [
            valid.replace(
                "impl HasOutput<Output = i32>",
                "impl HasOutput<Output = bool>",
            ),
            valid.replace(
                "impl HasOutput<Output = i32>",
                "impl HasOutput<Missing = i32>",
            ),
            valid.replace(
                "impl HasOutput<Output = i32>",
                "impl HasOutput<Output = i32, Output = bool>",
            ),
        ] {
            assert!(
                source_check(&invalid, workers).is_err(),
                "opaque declaration equality lost its original authority: {invalid}"
            );
        }
    }
}

#[test]
fn opaque_fn_output_shorthand_is_a_declaration_bound() {
    let source = r#"
        fn make() -> impl Fn(i32) -> i32 { |x| x + 1 }
        fn main() { let value = make(); assert!(value(41) == 42); }
    "#;
    for workers in [1, 8] {
        source_check(source, workers).unwrap();
        let invalid = source.replace("impl Fn(i32) -> i32", "impl Fn(i32) -> bool");
        assert!(source_check(&invalid, workers).is_err());
    }
}

fn native_matches_stock(source: &str) {
    native_matches_stock_with_dispatch(source, true);
}

fn native_matches_stock_with_dispatch(source: &str, requires_dispatch: bool) {
    use std::os::unix::fs::PermissionsExt;
    use std::process::Command;
    let directory = std::env::temp_dir().join(format!(
        "opaque-native-object-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    std::fs::create_dir(&directory).unwrap();
    let input = directory.join("main.rs");
    let stock_image = directory.join("stock");
    std::fs::write(&input, source).unwrap();
    let compiler = stock_rustc::stock_rustc();
    let version = Command::new(&compiler)
        .env_remove("RUSTC_BOOTSTRAP")
        .arg("-vV")
        .output()
        .unwrap();
    assert!(version.status.success());
    eprintln!(
        "opaque stock oracle {:?}: {}",
        compiler,
        String::from_utf8_lossy(&version.stdout)
    );
    let stock = Command::new(&compiler)
        .env_remove("RUSTC_BOOTSTRAP")
        .args([
            "--edition=2024",
            "-Cpanic=abort",
            "--crate-name=opaque_oracle",
        ])
        .arg(&input)
        .arg("-o")
        .arg(&stock_image)
        .output()
        .unwrap();
    assert!(
        stock.status.success(),
        "stock rejected: {}",
        String::from_utf8_lossy(&stock.stderr)
    );
    let expected = Command::new(&stock_image).output().unwrap();
    assert!(expected.status.success());
    for workers in [1, 8] {
        source_check(source, workers).unwrap();
        let snapshot = Arc::new(Snapshot {
            input: Input::Text(source.into()),
            source_options: source::SourceOptions::default(),
            budget: ir::PlanBudget::default(),
        });
        let engine = Engine::new(CompilerProvider(snapshot));
        let key = crate::compiler::keys::BuildRoot.key();
        let build = engine.request(key.clone());
        let report = engine.run(workers).unwrap();
        assert!(report.capture_complete);
        build
            .completion()
            .unwrap_or_else(|error| panic!("opaque BUILD workers={workers}: {error:?}"));
        if requires_dispatch {
            assert!(
                report
                    .node_evaluations
                    .iter()
                    .any(|(key, _)| key.kind == OPAQUE_CALL_DISPATCH)
            );
        }
        let image = engine
            .fact_of::<Vec<u8>>(&key, 0)
            .expect("successful BUILD must publish image");
        let path = directory.join(format!("native-{workers}"));
        std::fs::write(&path, image.as_slice()).unwrap();
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o700)).unwrap();
        let actual = Command::new(&path).output().unwrap();
        assert_eq!(
            actual.status.code(),
            expected.status.code(),
            "workers={workers}: {actual:?}"
        );
        assert_eq!(actual.stdout, expected.stdout, "workers={workers}");
        assert_eq!(actual.stderr, expected.stderr, "workers={workers}");
    }
    std::fs::remove_dir_all(directory).unwrap();
}

#[test]
fn opaque_definitions_require_a_concrete_nonrecursive_hidden_type() {
    let sources = [
        "fn unused() -> impl Sized { unused() } fn main() {}",
        "fn first() -> impl Sized { second() } fn second() -> impl Sized { first() } fn main() {}",
        "fn unused() -> impl Sized { loop {} } fn main() {}",
        "fn first<T:Copy>(v:T)->impl Sized{second(v)} fn second<U:Copy>(v:U)->impl Sized{first(v)} fn main(){}",
        "fn growing<T:Copy>(v:T)->impl Sized{growing((v,v))} fn main(){}",
        "fn first()->impl FnOnce()->i32{second()}fn second()->impl FnOnce()->i32{first()}fn main(){}",
    ];
    for workers in [1, 8] {
        for source in sources {
            assert!(
                source_check(source, workers).is_err(),
                "accepted incomplete opaque definition: {source}"
            );
            let snapshot = Arc::new(Snapshot {
                input: Input::Text(source.into()),
                source_options: source::SourceOptions::default(),
                budget: ir::PlanBudget::default(),
            });
            let engine = Engine::new(CompilerProvider(snapshot));
            let key = crate::compiler::keys::BuildRoot.key();
            let build = engine.request(key.clone());
            engine.run(workers).unwrap();
            assert!(
                build.completion().is_err(),
                "built incomplete opaque definition: {source}"
            );
            assert!(
                engine.fact_of::<Vec<u8>>(&key, 0).is_none(),
                "published an executable for incomplete opaque definition"
            );
        }
    }
}
#[test]
fn opaque_source_check_rejects_invalid_unused_declarations() {
    let sources = [
        "fn unused() -> i32 { true } fn main() {}",
        "trait Read { fn read(&self) -> i32; } struct Number { value: i32 } impl Read for Number { fn read(&self) -> i32 { true } } fn main() {}",
        "fn unused() -> &'static i32 { let value = 1; &value } fn main() {}",
        "const BAD: i32 = true; fn main() {}",
        "trait Marker {} struct Number { value: i32 } fn unused() -> impl Marker { Number { value: 1 } } fn main() {}",
        "struct Number { value: i32 } fn consume(value: Number) {} fn unused() { let value = Number { value: 1 }; consume(value); let again = value.value; } fn main() {}",
        "trait Marker {} struct Number { value: i32 } impl Marker for Number {} fn unused(flag: bool) -> impl Marker { if flag { Number { value: 1 } } else { false } } fn main() {}",
    ];
    for workers in [1, 8] {
        for source in sources {
            assert!(
                source_check(source, workers).is_err(),
                "accepted invalid unused item: {source}"
            );
        }
    }
}

#[test]
fn opaque_source_check_preserves_unused_associated_constant_admission() {
    native_matches_stock_with_dispatch(
        "struct Number {} impl Number { const GOOD: i32 = 42; } fn main() { assert!(Number::GOOD == 42); }",
        false,
    );
    for workers in [1, 8] {
        source_check(
            "struct Number {} impl Number { const GOOD: i32 = 42; } fn main() {}",
            workers,
        )
        .unwrap();
        source_check(
            "struct Number {} impl Number { const GOOD: i32 = 42; } fn main() { assert!(Number::GOOD == 42); }",
            workers,
        )
        .unwrap();
        for source in [
            "trait Values { const BAD: i32 = true; } fn main() {}",
            "struct Number {} impl Number { const BAD: i32 = true; } fn main() {}",
        ] {
            assert!(
                source_check(source, workers).is_err(),
                "accepted invalid unused associated constant: {source}"
            );
        }
    }
}
