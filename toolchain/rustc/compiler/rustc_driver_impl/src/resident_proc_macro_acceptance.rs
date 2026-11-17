//! Explicit real native integration gate. Never selected by ordinary builds.

#[test]
#[ignore = "requires explicitly supplied matching helper, macro library, source and sysroot"]
fn native_request_macro_expansion() {
    use rustc_data_structures::jobserver::resident::RequestLease;
    use rustc_metadata::resident_proc_macro_request::RequestMacros;
    use std::path::PathBuf;

    let helper = PathBuf::from(std::env::var_os("JUSTRUST_TEST_RESIDENT_HELPER").unwrap());
    let library = std::env::var("JUSTRUST_TEST_RESIDENT_MACRO_LIBRARY").unwrap();
    let source = std::env::var("JUSTRUST_TEST_RESIDENT_MACRO_SOURCE").unwrap();
    let sysroot = std::env::var("JUSTRUST_TEST_RESIDENT_MACRO_SYSROOT").unwrap();
    let output = PathBuf::from(std::env::var_os("JUSTRUST_TEST_RESIDENT_MACRO_OUTPUT").unwrap());
    std::fs::create_dir_all(&output).unwrap();
    for revision in 0..2 {
        let lease = RequestLease::begin_serial().unwrap();
        lease
            .with_current(|| {
                let owner = RequestMacros::begin(helper.clone(), lease.client()).unwrap();
                let args = vec![
                    "jr-rustc-integration".into(),
                    source.clone(),
                    "--crate-name=resident_macro_acceptance".into(),
                    "--edition=2024".into(),
                    "--crate-type=rlib".into(),
                    format!("--extern=resident_macro_fixture={library}"),
                    "--sysroot".into(),
                    sysroot.clone(),
                    "-o".into(),
                    output.join(format!("revision-{revision}.rlib")).to_str().unwrap().into(),
                ];
                // This uses normal parsing, metadata loading, expansion, typechecking,
                // codegen and linking. The fixture's compile-time assertion requires
                // macro global/thread-local counts 1,2 again on the second request.
                assert!(
                    crate::catch_fatal_errors(|| {
                        crate::compiler_entrypoint(
                            &args,
                            &mut crate::TimePassesCallbacks::default(),
                        );
                    })
                    .is_ok()
                );
                drop(owner);
            })
            .unwrap();
        lease.finish().unwrap();
        assert!(output.join(format!("revision-{revision}.rlib")).metadata().unwrap().len() > 0);
    }
}
