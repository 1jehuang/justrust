//! Invalid unused mutable-capture bodies must fail through ownership authority.
#![cfg(all(target_os = "linux", target_arch = "x86_64"))]
use justrust_engine::compiler;
#[path = "support/oracle.rs"]
mod oracle;

#[test]
fn unused_mutable_capture_bodies_cannot_bypass_ownership() {
    assert!(
        std::process::Command::new("rustc")
            .arg("-vV")
            .output()
            .is_ok_and(|output| output.status.success()),
        "stock oracle required"
    );
    let sources = [
        "struct Value{n:i32}fn unused(mut n:i32){(|v:Value|->i32{n+=1;let moved=v;n+v.n})(Value{n:1});}fn main(){}",
        "fn unused(mut n:i32){(||->i32{let r=&mut n;let q=r;*r})();}fn main(){}",
        "fn unused(mut n:i32){(||->i32{let r=&mut n;let q=&n;*r+*q})();}fn main(){}",
        "fn unused(mut n:i32){(||->i32{let r=&n;n+=1;*r})();}fn main(){}",
    ];
    let cases: Vec<_> = sources
        .iter()
        .zip(["E0382", "E0382", "E0502", "E0506"])
        .enumerate()
        .map(|(index, (source, code))| oracle::Case {
            name: [
                "unused_move",
                "unused_unique_move",
                "unused_alias",
                "unused_write",
            ][index],
            source,
            expect: oracle::Expect::Invalid(code),
        })
        .collect();
    oracle::check("closure-mut-authority", &cases);
    for source in sources {
        for workers in [1, 8] {
            for check_only in [true, false] {
                let failure = compiler::compile_observed(source.into(), workers, check_only)
                    .err()
                    .expect("unused invalid mutable capture body must reject");
                let message = failure.to_string();
                assert!(!message.contains("unsupported"), "{message}\n{source}");
                assert!(!message.contains("panicked"), "{message}\n{source}");
            }
        }
    }
}
