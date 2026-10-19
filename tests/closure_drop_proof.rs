//! Even a no-call closure retains canonical lowering proof for owning scratch locals.
#![cfg(all(target_os = "linux", target_arch = "x86_64"))]
#[path = "support/oracle.rs"]
mod oracle;

#[test]
fn no_call_closure_keeps_owning_drop_lowering_authority() {
    oracle::check(
        "closure_drop_proof",
        &[oracle::Case {
            name: "unused_owning_capture_scratch",
            source: "struct D{i:i32}impl Drop for D{fn drop(&mut self){println!(\"{}\",self.i);}}fn main(){let d=D{i:7};{let f=||7i32;assert_eq!(f(),7);}println!(\"after\");}",
            expect: oracle::Expect::Run("after\n7\n"),
        }],
    );
}
