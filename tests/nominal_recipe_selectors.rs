//! Index expressions are operator constraints, never nominal field recipes.
#![cfg(all(target_os = "linux", target_arch = "x86_64"))]
#[path = "support/iterator_oracle.rs"]
mod oracle;

#[test]
fn builtin_index_rejects_non_integer_operand() {
    oracle::stock_and_native_reject("fn main(){let a=[1u8];let n=a[false];}");
}

#[test]
fn nominal_constant_and_dynamic_index() {
    oracle::stock_native(
        "fn main(){let mut v=Vec::new();v.push(7i32);v.push(9);let i=1usize;println!(\"{} {}\",v[0],v[i]);}",
    );
}

#[test]
fn nominal_index_assignment() {
    oracle::stock_native(
        "fn main(){let mut v=vec![1i32,2];let i=1usize;v[i]=7;println!(\"{}\",v[i]);}",
    );
}

#[test]
fn builtin_index_retains_element_constraints() {
    oracle::stock_native(
        "fn main(){let mut a=[1u8,2];let i=1usize;a[i]=7;println!(\"{} {}\",a[0],a[i]);}",
    );
}

#[test]
fn nominal_index_uses_exact_trait_argument_not_field_position() {
    oracle::stock_native(
        "use std::ops::Index;struct Slots{value:i32}impl Index<bool> for Slots{type Output=i32;fn index(&self, flag:bool)->&i32{&self.value}}fn main(){let s=Slots{value:7};println!(\"{}\",s[true]);}",
    );
}

#[test]
fn unchanged_corpus_diagnostics() {
    for (name, source) in [
        (
            "04",
            include_str!("fixtures/full_programs/04_primes_sieve.rs"),
        ),
        (
            "10",
            include_str!("fixtures/full_programs/10_string_processing.rs"),
        ),
        (
            "18",
            include_str!("fixtures/full_programs/18_tokenizer_calculator.rs"),
        ),
    ] {
        for workers in [1, 8] {
            let result = justrust_engine::compiler::compile(source.into(), workers, true);
            match result {
                Ok(_) => println!("{name} workers={workers}: CHECK passed"),
                Err(error) => {
                    println!("{name} workers={workers}: {error}");
                    assert!(
                        !error
                            .to_string()
                            .contains("non-field selector in nominal recipe")
                    );
                }
            }
        }
    }
}
