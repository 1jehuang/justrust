use super::*;

fn source_check(source: &str, workers: usize) -> Result<(), String> {
    let snapshot = Arc::new(Snapshot {
        input: Input::Text(source.into()),
        source_options: source::SourceOptions::default(),
        budget: ir::PlanBudget::default(),
    });
    let engine = Engine::new(CompilerProvider(snapshot));
    let check = engine.request(Key::new(CHECK, vec![]));
    let report = engine.run(workers).unwrap();
    assert!(
        !report.node_evaluations.iter().any(|(key, _)| {
            matches!(
                key.kind,
                BUILD | OBJECT | OPAQUE_REPRESENTATION | OPAQUE_CALL_DISPATCH | MONO_PLAN
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
            assert!(first.read() == 42);
            assert!(second.read() == 42);
        }
    "#,
    );
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
    let compiler = std::env::var_os("JUSTRUST_REAL_RUSTC").unwrap_or_else(|| "rustc".into());
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
        let key = Key::new(BUILD, vec![]);
        let build = engine.request(key.clone());
        let report = engine.run(workers).unwrap();
        build
            .completion()
            .unwrap_or_else(|error| panic!("opaque BUILD workers={workers}: {error:?}"));
        assert!(
            report
                .node_evaluations
                .iter()
                .any(|(key, _)| key.kind == OPAQUE_CALL_DISPATCH)
        );
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
        "fn one() -> impl Sized { 1 } fn two() -> impl Sized { one() } fn main() {}",
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
            let key = Key::new(BUILD, vec![]);
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
