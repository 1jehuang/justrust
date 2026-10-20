//! Static-owner inference is source checking, not executable planning.
#![cfg(all(target_os = "linux", target_arch = "x86_64"))]
#[test]
fn static_owner_check_demands_no_executable_work() {
    let source = "struct Packet<T>{value:T} impl<T> Packet<T>{fn wrap(value:T)->Option<Self>{Some(Self{value})}} fn main(){let value=Packet::<u64>::wrap(42);}";
    for workers in [1, 8] {
        let checked = justrust_engine::compiler::compile(source.into(), workers, true).unwrap();
        assert!(checked.executable.is_none());
        assert!(
            checked
                .report
                .node_evaluations
                .iter()
                .all(|(key, _)| { !matches!(key.kind, 103 | 105 | 113 | 130 | 131 | 133) }),
            "static-owner CHECK demanded executable work"
        );
    }
}
