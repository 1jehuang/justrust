//! Source rejection is required even when a native declaration is unused.
use justrust_engine::compiler;

#[test]
fn stock_valid_private_store_pointer_boundaries_are_explicitly_unsupported() {
    use std::{fs, process::Command};
    struct Scratch(std::path::PathBuf);
    impl Drop for Scratch {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }
    let scratch = Scratch(std::env::temp_dir().join(format!(
            "jr-c-pointer-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        )));
    fs::create_dir(&scratch.0).unwrap();
    let stock = std::env::var_os("JUSTRUST_REAL_RUSTC").unwrap_or_else(|| "rustc".into());
    for (index,source) in [
        "unsafe extern \"C\" {fn narrow(p:*mut u8);}fn main(){}",
        "type Tiny=u8;unsafe extern \"C\" {fn alias(p:*const Tiny);}fn main(){}",
        "#[repr(C)]struct Inner{p:*mut f32}#[repr(C)]struct Outer{inner:Inner}unsafe extern \"C\"{fn nested(x:Outer);}fn main(){}",
        "extern \"C\" fn callback(p:*mut bool){}fn main(){}",
        "unsafe extern \"C\" {fn zst(p:*mut ());}fn main(){}",
        "unsafe extern \"C\" {fn nested_pointer(p:*mut *mut u64);}fn main(){}",
    ].iter().enumerate() {
        let input = scratch.0.join(format!("case{index}.rs"));
        fs::write(&input,source).unwrap();
        let output = Command::new("timeout").arg("60").arg(&stock)
            .env_remove("RUSTC_BOOTSTRAP")
            .args(["--edition=2024","--emit=metadata","-Awarnings"])
            .arg(&input).arg("-o").arg(scratch.0.join(format!("case{index}.rmeta"))).output().unwrap();
        assert!(output.status.success(),"stock rejected valid pointer case {source}: {}",String::from_utf8_lossy(&output.stderr));
        for workers in [1,8] {
            for check in [true,false] {
                let failure = compiler::compile_observed((*source).to_owned(),workers,check).err().expect(source);
                assert!(failure.to_string().contains("unsupported"),"{source}: {failure}");
                if let Some(report)=failure.report { assert!(!report.published_kind(105)); }
            }
        }
    }
}

#[test]
fn private_carriers_are_not_c_storage_and_forged_transport_is_rejected() {
    use compiler::{
        c_abi::{
            self,
            classify::{Location, Transport},
            layout,
        },
        ir,
    };
    let budget = ir::PlanBudget::default();
    let byte = layout::of(&ir::Ty::Int(ir::IntTy::U8), &|_| false, &budget).unwrap();
    assert_eq!((byte.size, byte.align, byte.private_size), (1, 1, 8));
    let bytes = layout::of(
        &ir::Ty::Array {
            element: std::sync::Arc::new(ir::Ty::Int(ir::IntTy::U8)),
            len: 3,
        },
        &|_| false,
        &budget,
    )
    .unwrap();
    assert_eq!((bytes.size, bytes.align, bytes.private_size), (3, 1, 24));
    assert_eq!(
        bytes
            .leaves
            .iter()
            .map(|x| (x.c_offset, x.private_offset))
            .collect::<Vec<_>>(),
        vec![(0, 0), (1, 8), (2, 16)]
    );
    let word = layout::of(&ir::Ty::Int(ir::IntTy::U64), &|_| false, &budget).unwrap();
    let pair = layout::of(
        &ir::Ty::Array {
            element: std::sync::Arc::new(ir::Ty::Int(ir::IntTy::U64)),
            len: 2,
        },
        &|_| false,
        &budget,
    )
    .unwrap();
    let mut parameters = vec![word.clone(); 5];
    parameters.push(pair);
    parameters.push(word.clone());
    let mut signature = c_abi::CAbiSignature::new(parameters, word).unwrap();
    assert!(matches!(
        signature.plan.arguments[5],
        Transport::Memory { offset: 0 }
    ));
    assert_eq!(
        signature.plan.arguments[6],
        Transport::Registers(vec![(0, Location::Integer(5))])
    );
    signature.plan.integer_registers = 0;
    assert!(
        c_abi::import(&signature, "native", &budget)
            .unwrap_err()
            .contains("forged")
    );
}

#[test]
fn unused_unsupported_native_contracts_never_publish_success() {
    for source in [
        "extern \"C-unwind\" fn f() {} fn main() {}",
        "extern \"system\" fn f() {} fn main() {}",
        "extern \"C\" fn f<T>(x:T) {} fn main() {}",
        "extern \"C\" fn f(x: &u8) {} fn main() {}",
        "struct S { x: u8 } extern \"C\" fn f(x:S) {} fn main() {}",
        "#[repr(C)] struct S { x: char } extern \"C\" fn f(x:S) {} fn main() {}",
        "unsafe extern \"C\" { static N:u64; } fn main() {}",
        "unsafe extern \"C\" { fn variadic(x:u64,...); } fn main() {}",
        "unsafe extern \"C\" { fn private_array(p:*const [u8;3]); } fn main() {}",
        "unsafe extern \"C\" { fn _start(); } fn main() {}",
        "#[repr(packed)] struct S { x:u8 } fn main() {}",
        "#[unsafe(no_mangle)] fn f() {} fn main() {}",
    ] {
        for workers in [1, 8] {
            for check in [true, false] {
                let failure = compiler::compile_observed(source.to_owned(), workers, check)
                    .err()
                    .expect(source);
                assert!(
                    failure.to_string().contains("unsupported"),
                    "{source}: {failure}"
                );
                if let Some(report) = failure.report {
                    assert!(!report.published_kind(105));
                }
            }
        }
    }
}

#[test]
fn unused_invalid_native_body_and_unsafe_call_are_checked() {
    for source in [
        "extern \"C\" fn invalid()->u64{true}fn main(){}",
        "unsafe extern \"C\" { fn unknown()->u64; }fn main(){unknown();}",
    ] {
        for workers in [1, 8] {
            assert!(
                compiler::compile_observed(source.to_owned(), workers, true).is_err(),
                "{source}"
            );
        }
    }
}

#[test]
fn public_ast_mutation_cannot_reuse_native_signature_or_foreign_body_authority() {
    use compiler::{frontend, ir};
    let original =
        frontend::parse("unsafe extern \"C\" { fn unknown(x:u8)->u64; } fn main(){}").unwrap();
    let mut changed = original.clone();
    changed.functions[0].result = frontend::Type::Unit;
    assert!(frontend::PlanCtx::from_program(&changed, &ir::PlanBudget::default()).is_err());
    let mut renamed = original.clone();
    renamed.functions[0].name = original.functions[1].name.clone();
    assert!(frontend::PlanCtx::from_program(&renamed, &ir::PlanBudget::default()).is_err());
    let mut removed = original.clone();
    removed.functions.remove(0);
    assert!(frontend::PlanCtx::from_program(&removed, &ir::PlanBudget::default()).is_err());
    let mut changed = original.clone();
    changed.functions[0].body = original.functions[1].body.clone();
    assert!(frontend::PlanCtx::from_program(&changed, &ir::PlanBudget::default()).is_err());
    let mut pointer =
        frontend::parse("unsafe extern \"C\" {fn wide(p:*mut u64);}fn main(){}").unwrap();
    assert!(frontend::PlanCtx::from_program(&pointer, &ir::PlanBudget::default()).is_ok());
    let frontend::Type::RawPtr { pointee, .. } = &mut pointer.functions[0].params[0].1 else {
        panic!("pointer source fixture");
    };
    **pointee = frontend::Type::Int(ir::IntTy::U8);
    let failure = frontend::PlanCtx::from_program(&pointer, &ir::PlanBudget::default())
        .err()
        .unwrap();
    assert!(failure.contains("authority"), "{failure}");
    let mut duplicated = original.clone();
    duplicated.functions.push(original.functions[0].clone());
    assert!(frontend::PlanCtx::from_program(&duplicated, &ir::PlanBudget::default()).is_err());
}
