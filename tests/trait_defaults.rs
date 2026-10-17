//! Source default bodies must never escape universal validation by being unused.
//! Until their callable registration and proof environment are implemented, reject
//! every provided body explicitly, including otherwise valid overridden bodies.
use justrust_engine::compiler;

fn rejects_default(source: &str) {
    for workers in [1, 8] {
        for check in [true, false] {
            let failure = compiler::compile_observed(source.to_owned(), workers, check)
                .err()
                .expect("unvalidated default body was accepted");
            let diagnostic = failure.to_string();
            assert!(
                diagnostic.contains("source trait default method bodies are unsupported"),
                "default rejection must be explicit: {diagnostic}"
            );
        }
    }
}

#[test]
fn unused_valid_default_is_explicitly_unsupported() {
    rejects_default("trait Value{fn value(self)->i32{7}}fn main(){}");
}

#[test]
fn unused_invalid_default_cannot_disappear_from_validation() {
    rejects_default("trait Value{fn value(self)->i32{missing()}}fn main(){}");
}

#[test]
fn overridden_invalid_default_still_cannot_disappear() {
    rejects_default(
        "struct S{x:i32}trait Value{fn value(self)->i32{missing()}}impl Value for S{fn value(self)->i32{7}}fn main(){let n=S{x:1}.value();}",
    );
}

#[test]
fn unused_default_move_violation_cannot_disappear() {
    rejects_default("trait Duplicate{fn duplicate(self)->(Self,Self){(self,self)}}fn main(){}");
}

#[test]
fn overridden_valid_default_is_explicitly_unsupported() {
    rejects_default(
        "struct S{x:i32}trait Value{fn value(self)->i32{7}}impl Value for S{fn value(self)->i32{8}}fn main(){let n=S{x:1}.value();}",
    );
}
