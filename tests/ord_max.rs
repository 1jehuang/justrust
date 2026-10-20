//! Genuine Ord::max source default, separate from first-class function items.
#![cfg(all(target_os = "linux", target_arch = "x86_64"))]
#[path = "support/closure_oracle.rs"]
mod oracle;
use oracle::{Case, Expect};

#[test]
fn ord_max_method_ufcs_alias_and_generic_match_stock() {
    oracle::check(
        "ord-max-scalar",
        &[Case {
            name: "canonical_default_calls",
            source: "type Number=i32;fn choose<T:Ord>(a:T,b:T)->T{a.max(b)}fn main(){let a=3i32.max(7);let b=i32::max(9,4);let c=Number::max(-5,-2);let d=<i32 as Ord>::max(6,6);let e=choose(11i32,8i32);println!(\"{} {} {} {} {}\",a,b,c,d,e);}",
            expect: Expect::Run("7 9 -2 6 11\n"),
        }],
    );
}

macro_rules! equal_values {
    ($body:literal) => {
        concat!(
            "use std::cmp::Ordering;struct Equal{tag:i32}impl PartialEq for Equal{fn eq(&self,other:&Self)->bool{true}}impl Eq for Equal{}impl PartialOrd for Equal{fn partial_cmp(&self,other:&Self)->Option<Ordering>{Some(Ordering::Equal)}}",
            $body
        )
    };
}

#[test]
fn ord_max_equal_noncopy_values_choose_second() {
    oracle::check(
        "ord-max-tie",
        &[Case {
            name: "noncopy_equal_second",
            source: equal_values!(
                "impl Ord for Equal{fn cmp(&self,other:&Self)->Ordering{Ordering::Equal}}fn main(){let chosen=Equal{tag:1}.max(Equal{tag:2});println!(\"{}\",chosen.tag);}"
            ),
            expect: Expect::Run("2\n"),
        }],
    );
}

#[test]
fn ord_max_user_override_is_canonical() {
    oracle::check(
        "ord-max-override",
        &[Case {
            name: "override_not_default",
            source: equal_values!(
                "impl Ord for Equal{fn cmp(&self,other:&Self)->Ordering{Ordering::Equal}fn max(self,other:Self)->Self{self}}fn main(){let chosen=<Equal as Ord>::max(Equal{tag:1},Equal{tag:2});println!(\"{}\",chosen.tag);}"
            ),
            expect: Expect::Run("1\n"),
        }],
    );
}

#[test]
fn ord_max_move_and_unrelated_spelling_do_not_bypass_authority() {
    oracle::check(
        "ord-max-authority",
        &[
            Case {
                name: "noncopy_receiver_is_moved",
                source: equal_values!(
                    "impl Ord for Equal{fn cmp(&self,other:&Self)->Ordering{Ordering::Equal}}fn main(){let first=Equal{tag:1};let chosen=first.max(Equal{tag:2});println!(\"{} {}\",first.tag,chosen.tag);}"
                ),
                expect: Expect::Invalid("E0382"),
            },
            Case {
                name: "unrelated_max_does_not_prove_ord",
                source: "struct Value;trait Other{fn max(self,other:Self)->Self;}impl Other for Value{fn max(self,other:Self)->Self{self}}fn main(){let value=<Value as Ord>::max(Value,Value);}",
                expect: Expect::Invalid("E0277"),
            },
        ],
    );
}

#[test]
fn ord_max_invalid_membership_arguments_and_unused_body_reject() {
    oracle::check(
        "ord-max-invalid",
        &[
            Case {
                name: "missing_ord",
                source: "struct Value;fn main(){let value=Value.max(Value);}",
                expect: Expect::Invalid("E0599"),
            },
            Case {
                name: "wrong_argument_type",
                source: "fn main(){let value=i32::max(1,true);}",
                expect: Expect::Invalid("E0308"),
            },
            Case {
                name: "unused_bad_body",
                source: "fn unused()->u8{true}fn main(){let value=1i32.max(2);}",
                expect: Expect::Invalid("E0308"),
            },
        ],
    );
}
