//! Invalid array-length receivers cannot be admitted as builtin operations.
use justrust_engine::compiler;

#[test]
fn invalid_array_len_receivers_reject_in_check_and_build() {
    for source in [
        "struct S{x:i32}fn main(){let a=[S{x:1}];let b=a;let n=a.len();}",
        "fn bad(){let a=[1,2];let n=a.len(3);}fn main(){}",
        "fn bad(){let a=[1,2];let n=a.len::<i32>();}fn main(){}",
        "fn bad(){let a=[1,2];let n:bool=a.len();}fn main(){}",
    ] {
        for workers in [1, 8] {
            for check in [true, false] {
                assert!(
                    compiler::compile(source.into(), workers, check).is_err(),
                    "accepted workers={workers},check={check}: {source}"
                );
            }
        }
    }
}

#[test]
fn array_methods_cannot_fall_back_after_selection_or_deferral() {
    let root = std::env::temp_dir().join(format!("array-len-priority-{}", std::process::id()));
    std::fs::create_dir_all(&root).unwrap();
    for (index, source) in [
        // Stock selects the mutable method, then rejects the immutable place.
        "trait Other{fn len(&mut self)->usize;}impl Other for [i32;2]{fn len(&mut self)->usize{99}}fn bad(){let a=[1,2];let n=a.len();}fn main(){}",
        // Unresolved method arguments must not authorize builtin fallback.
        "trait Other{fn len<T>(&self)->usize;}impl Other for [i32;2]{fn len<T>(&self)->usize{99}}fn main(){let a=[1,2];let n=a.len();}",
        "trait One{fn len(&self)->usize;}trait Two{fn len(&self)->usize;}impl One for [i32;2]{fn len(&self)->usize{99}}impl Two for [i32;2]{fn len(&self)->usize{77}}fn main(){let a=[1,2];let n=a.len();}",
        "trait Other{fn len(&self)->bool;}impl Other for [i32;2]{fn len(&self)->bool{true}}fn main(){let a=[1,2];let n:usize=a.len();}",
        "fn main(){let a=[1,2];let n=(&mut a).len();}",
        "fn main(){let mut a=[1,2];let r=&a;let n=(&mut a).len();println!(\"{}\",r[0]);}",
        "trait Lens{fn get(&self)->&[i32;2];}impl Lens for [i32;2]{fn get(&self)->&[i32;2]{self}}fn main(){let mut a=[1,2];let r=(&mut a).get();a[0]=3;println!(\"{}\",r[0]);}",
        "trait Lens{fn get(&self)->&[i32;2];}impl Lens for [i32;2]{fn get(&self)->&[i32;2]{self}}fn bad()->&'static[i32;2]{let mut a=[1,2];(&mut a).get()}fn main(){}",
    ].into_iter().enumerate() {
        let input = root.join(format!("invalid_{index}.rs"));
        std::fs::write(&input, source).unwrap();
        let stock = std::process::Command::new("rustc")
            .args(["--edition=2024", "--emit=metadata"])
            .arg(&input).arg("-o").arg(root.join("invalid.rmeta"))
            .output().unwrap();
        assert!(!stock.status.success(), "stock accepted case {index}");
        for workers in [1, 8] {
            for check in [true, false] {
                assert!(compiler::compile(source.into(), workers, check).is_err(),
                    "selected array method fell back: case={index},workers={workers},check={check}");
            }
        }
    }
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn temporary_destructor_receiver_remains_explicitly_unsupported() {
    let source = "struct S{x:i32}impl Drop for S{fn drop(&mut self){}}fn make()->[S;1]{[S{x:1}]}fn main(){let n=make().len();}";
    for workers in [1, 8] {
        match compiler::compile(source.into(), workers, true) {
            Ok(_) => panic!("temporary destructor boundary unexpectedly admitted"),
            Err(error) => assert!(error.0.to_lowercase().contains("unsupported"), "{error}"),
        }
    }
}
