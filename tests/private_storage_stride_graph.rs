use justrust_engine::compiler;

#[test]
fn private_stride_check_remains_codegen_free_without_new_effect_root() {
    let source = "enum E{A,B(u16)} fn size<T>()->usize{std::mem::size_of::<T>()} fn main(){assert_eq!(size::<[[u16;3];2]>(),12usize);let value=Box::new(7u64);let got=*value.as_ref();assert_eq!(got,7u64);let mut values:Vec<E>=Vec::new();let p=values.as_ptr();let q=values.as_mut_ptr();assert!(p as usize!=0usize);assert!(q as usize!=0usize);values.push(E::B(513u16));}";
    for workers in [1, 8] {
        let checked = compiler::compile(source.into(), workers, true).unwrap();
        assert!(checked.executable.is_none());
        for kind in [102, 103, 108, 137, 140, 142] {
            assert!(
                !checked.report.published_kind(kind),
                "CHECK demanded codegen or closed instance {kind}"
            );
        }
        assert!(checked.report.published_kind(115));
        assert!(checked.report.published_kind(128));
    }
}
