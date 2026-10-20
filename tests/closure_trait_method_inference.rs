//! Generic method schemas constrain closure terms before checked metadata exists.
#![cfg(all(target_os = "linux", target_arch = "x86_64"))]
#[path = "support/closure_oracle.rs"]
mod oracle;
use oracle::{Case, Expect};

#[test]
fn canonical_trait_method_inputs_constrain_closures() {
    oracle::check(
        "trait_method_closure_inputs",
        &[Case {
            name: "source_u8_input",
            source: "struct Seed;trait Transform{fn apply<F:FnOnce(u8)->u8>(self,f:F)->u8;}impl Transform for Seed{fn apply<F:FnOnce(u8)->u8>(self,f:F)->u8{f(7)}}fn main(){assert_eq!(Seed.apply(|x|x+1),8u8);}",
            expect: Expect::Run(""),
        }],
    );
}

#[test]
fn associated_trait_method_input_constrains_closure() {
    oracle::check(
        "trait_method_associated_closure_input",
        &[Case {
            name: "associated_input",
            source: "struct Seed;trait Transform{type Item;fn apply<F:FnOnce(Self::Item)->i32>(self,f:F)->i32;}impl Transform for Seed{type Item=i32;fn apply<F:FnOnce(i32)->i32>(self,f:F)->i32{f(7)}}fn main(){assert_eq!(Seed.apply(|x|x+1),8);}",
            expect: Expect::Run(""),
        }],
    );
}

#[test]
fn trait_method_constraints_do_not_choose_ambiguous_or_invalid_calls() {
    oracle::check(
        "trait_method_closure_invalid",
        &[
            Case {
                name: "ambiguous_traits",
                source: "struct Seed;trait Left{fn apply<F:FnOnce(i32)->i32>(self,f:F)->i32;}trait Right{fn apply<F:FnOnce(u8)->u8>(self,f:F)->u8;}impl Left for Seed{fn apply<F:FnOnce(i32)->i32>(self,f:F)->i32{f(7)}}impl Right for Seed{fn apply<F:FnOnce(u8)->u8>(self,f:F)->u8{f(7)}}fn unused(){Seed.apply(|x|x);}fn main(){}",
                expect: Expect::Invalid("E0034"),
            },
            Case {
                name: "wrong_result",
                source: "struct Seed;trait Transform{fn apply<F:FnOnce(u8)->u8>(self,f:F)->u8;}impl Transform for Seed{fn apply<F:FnOnce(u8)->u8>(self,f:F)->u8{f(7)}}fn unused(){Seed.apply(|x|true);}fn main(){}",
                expect: Expect::Invalid("E0308"),
            },
        ],
    );
}
