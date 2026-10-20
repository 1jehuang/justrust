use justrust_engine::compiler;

#[test]
fn private_stride_rejects_foreign_intrinsic_owners_and_public_access() {
    for source in [
        "fn main(){let _value=__justrust_storage_stride!(u64);}",
        "mod mem{pub fn storage_stride<T>()->usize{__justrust_storage_stride!(T)}} fn main(){let _value=mem::storage_stride::<u64>();}",
        "fn main(){let _value=std::mem::storage_stride::<u64>();}",
    ] {
        for workers in [1, 8] {
            for check in [true, false] {
                let error = compiler::compile(source.into(), workers, check)
                    .err()
                    .expect("private compiler query was exposed")
                    .to_string();
                assert!(
                    error.contains("private")
                        || error.contains("canonical source")
                        || error.contains("visible"),
                    "{source}: {error}"
                );
            }
        }
    }
}
